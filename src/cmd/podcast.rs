use std::path::Path;

use crate::config::Config;
use crate::ffmpeg;
use crate::llm::LlmAgent;
use crate::podcast::{self, PodcastBackend, PodcastRequest, PodcastResult, VolcPodcast};
use crate::task::{Step, TaskEvents};
use crate::tts::TtsProvider;

fn script_prompt_template() -> String {
    if let Ok(t) = std::fs::read_to_string("prompts/podcast_script.txt") {
        return t;
    }
    include_str!("../../prompts/podcast_script.txt").to_string()
}

/// 核心流程（可注入 llm 与 backend 以便测试）
pub async fn run_with(
    dir: &Path,
    llm: &dyn LlmAgent,
    backend: &PodcastBackend,
    force_script: bool,
    user_prompt: Option<&str>,
    events: &TaskEvents,
) -> anyhow::Result<()> {
    let rewritten_path = dir.join("rewritten.md");
    anyhow::ensure!(
        rewritten_path.exists(),
        "缺少 {}/rewritten.md，请先运行 `media-factory rewrite`",
        dir.display()
    );
    let text = std::fs::read_to_string(&rewritten_path)?;
    let script_path = dir.join("script.md");
    events.step_running(Step::Podcast);

    match backend {
        PodcastBackend::Volc(v) => run_volc(v, dir, &script_path, &text, force_script, user_prompt, events).await,
        PodcastBackend::Tts(t) => run_tts(t.as_ref(), dir, &script_path, &text, llm, user_prompt, events).await,
    }
}

/// 把用户风格要求拼到输入文本前（volc-podcast 无独立风格字段，用前缀提示影响）
fn with_style(text: &str, user_prompt: Option<&str>) -> String {
    match user_prompt.map(|u| u.trim()) {
        Some(u) if !u.is_empty() => format!("【播客风格要求：{u}】\n\n{text}"),
        _ => text.to_string(),
    }
}

async fn run_volc(
    v: &VolcPodcast,
    dir: &Path,
    script_path: &Path,
    text: &str,
    force_script: bool,
    user_prompt: Option<&str>,
    events: &TaskEvents,
) -> anyhow::Result<()> {
    let input_text = with_style(text, user_prompt);
    // 模式 B：生成脚本供人工修改
    if force_script && !script_path.exists() {
        let res = v
            .generate(&PodcastRequest {
                input_text: Some(input_text),
                nlp_texts: None,
                only_nlp_text: true,
            })
            .await?;
        let PodcastResult::ScriptText(s) = res else {
            anyhow::bail!("预期返回脚本，但得到了音频");
        };
        std::fs::write(script_path, &s)?;
        events.artifact(Step::Podcast, "script.md");
        events.step_done(Step::Podcast);
        println!("✓ 已生成脚本: {}，请人工修改后重跑 `media-factory podcast --id {}`", script_path.display(), dir.file_name().unwrap().to_string_lossy());
        return Ok(());
    }

    // 已存在脚本 → 模式 B 合成（action=3）
    if script_path.exists() {
        let script = std::fs::read_to_string(script_path)?;
        let turns = podcast::parse_script(&script)?;
        let (sa, sb) = v.speakers();
        let nlp_texts = podcast::to_nlp_texts(&turns, &sa, &sb);
        let res = v
            .generate(&PodcastRequest {
                input_text: None,
                nlp_texts: Some(nlp_texts),
                only_nlp_text: false,
            })
            .await?;
        let PodcastResult::Audio { bytes, subtitles } = res else {
            anyhow::bail!("预期返回音频，但得到了脚本");
        };
        write_audio(dir, &bytes)?;
        events.artifact(Step::Podcast, "podcast.mp3");
        if !subtitles.is_empty() {
            let srt = dir.join("subtitle.srt");
            podcast::write_srt(&srt, &subtitles)?;
            events.artifact(Step::Podcast, "subtitle.srt");
            println!("✓ 字幕已生成: {}", srt.display());
        }
        events.step_done(Step::Podcast);
        return Ok(());
    }

    // 模式 A（默认）：文本 → 直接合成
    let res = v
        .generate(&PodcastRequest {
            input_text: Some(input_text),
            nlp_texts: None,
            only_nlp_text: false,
        })
        .await?;
    let PodcastResult::Audio { bytes, subtitles } = res else {
        anyhow::bail!("预期返回音频，但得到了脚本");
    };
    write_audio(dir, &bytes)?;
    events.artifact(Step::Podcast, "podcast.mp3");
    if !subtitles.is_empty() {
        let srt = dir.join("subtitle.srt");
        podcast::write_srt(&srt, &subtitles)?;
        events.artifact(Step::Podcast, "subtitle.srt");
        println!("✓ 字幕已生成: {}", srt.display());
    }
    events.step_done(Step::Podcast);
    Ok(())
}

async fn run_tts(
    t: &dyn TtsProvider,
    dir: &Path,
    script_path: &Path,
    text: &str,
    llm: &dyn LlmAgent,
    user_prompt: Option<&str>,
    events: &TaskEvents,
) -> anyhow::Result<()> {
    // 脚本：已存在则直接用，否则 pi 生成
    let script = if script_path.exists() {
        std::fs::read_to_string(script_path)?
    } else {
        let user_section = match user_prompt.map(|u| u.trim()) {
            Some(u) if !u.is_empty() => {
                format!("\n用户额外要求（请尽量融入对白风格，但以上基本要求仍然有效）：\n{u}\n")
            }
            _ => String::new(),
        };
        let prompt = script_prompt_template()
            .replace("{{TEXT}}", text)
            .replace("{{USER_PROMPT}}", &user_section);
        let s = llm.complete(&prompt).await?;
        std::fs::write(script_path, &s)?;
        s
    };

    let turns = podcast::parse_script(&script)?;
    let (host_v, guest_v) = t.default_voices();

    let mut segs = Vec::new();
    for (i, turn) in turns.iter().enumerate() {
        let voice = match turn.role {
            podcast::script::Role::Host => &host_v,
            podcast::script::Role::Guest => &guest_v,
        };
        let bytes = t.synthesize(&turn.text, voice).await?;
        let seg = dir.join(format!("seg-{i:03}.mp3"));
        std::fs::write(&seg, bytes)?;
        segs.push(seg);
    }

    let out = dir.join("podcast.mp3");
    ffmpeg::concat_mp3(&segs, &out)?;
    events.artifact(Step::Podcast, "podcast.mp3");
    events.artifact(Step::Podcast, "script.md");
    events.step_done(Step::Podcast);
    println!("✓ 播客完成: {}", out.display());
    Ok(())
}

fn write_audio(dir: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let out = dir.join("podcast.mp3");
    std::fs::write(&out, bytes)?;
    println!("✓ 播客完成: {}", out.display());
    Ok(())
}

/// 公开入口
pub async fn run(
    id: Option<String>,
    force_script: bool,
    user_prompt: Option<String>,
) -> anyhow::Result<String> {
    let dir = match &id {
        Some(i) => super::task_dir(Path::new("output"), i),
        None => super::latest_task_dir(Path::new("output"))?,
    };
    let id = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let backend = podcast::resolve_podcast(&cfg)?;
    let events = crate::task::TaskEvents::local(Path::new("output"), &id);
    run_with(&dir, llm.as_ref(), &backend, force_script, user_prompt.as_deref(), &events).await?;
    Ok(id)
}
