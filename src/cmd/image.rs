use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::llm::LlmAgent;
use crate::provider::{self, ImageProvider, ImageRequest};
use crate::task::{Step, TaskEvents};

/// 读取图像 prompt 模板：优先运行时读 cwd/prompts/image_prompt.txt，缺失用嵌入默认。
fn image_prompt_template() -> String {
    if let Ok(t) = std::fs::read_to_string("prompts/image_prompt.txt") {
        return t;
    }
    include_str!("../../prompts/image_prompt.txt").to_string()
}

/// 用 pi 从改写文案提炼图像 prompt
pub async fn distill_prompt(
    llm: &dyn LlmAgent,
    text: &str,
    user_prompt: Option<&str>,
) -> anyhow::Result<String> {
    let user_section = match user_prompt.map(|u| u.trim()) {
        Some(u) if !u.is_empty() => {
            format!("\n用户额外要求（请尽量融入画面风格，但以上基本要求仍然有效）：\n{u}\n")
        }
        _ => String::new(),
    };
    let prompt = image_prompt_template()
        .replace("{{TEXT}}", text)
        .replace("{{USER_PROMPT}}", &user_section);
    let out = llm.complete(&prompt).await?;
    Ok(out.trim().to_string())
}

/// 免责声明文案（叠加到图片底部，而非写入提示词）
pub const DISCLAIMER_TEXT: &str = "以上内容仅代表个人观点。不构成投资建议。";

/// 核心流程（可注入 llm 与 provider 以便测试）
pub async fn run_with(
    dir: &Path,
    reference: Vec<PathBuf>,
    llm: &dyn LlmAgent,
    provider: &dyn ImageProvider,
    user_prompt: Option<&str>,
    events: &TaskEvents,
    disclaimer: bool,
) -> anyhow::Result<()> {
    let rewritten_path = dir.join("rewritten.md");
    anyhow::ensure!(
        rewritten_path.exists(),
        "缺少 {}/rewritten.md，请先运行 `media-factory rewrite`",
        dir.display()
    );

    let text = std::fs::read_to_string(&rewritten_path)?;
    events.step_running(Step::Image);
    let img_prompt = distill_prompt(llm, &text, user_prompt).await?;
    println!("图像 prompt: {}", img_prompt);

    if !reference.is_empty() && !provider.supports_reference() {
        eprintln!("⚠️  当前生图 provider 不支持参考图，已忽略参考图降级为纯文本生图");
    }
    let reference = if provider.supports_reference() { reference } else { vec![] };

    let bytes = provider
        .generate(&ImageRequest {
            prompt: img_prompt,
            reference_images: reference,
        })
        .await?;

    let out = dir.join("image.png");
    std::fs::write(&out, bytes)?;

    // 勾选免责声明时：生成图片后把声明叠加到图片底部（等同字幕叠加逻辑，不污染提示词）
    if disclaimer {
        let tmp = dir.join("image_disclaimer.png");
        crate::ffmpeg::overlay_disclaimer(&out, DISCLAIMER_TEXT, &tmp)?;
        std::fs::rename(&tmp, &out)?;
        println!("✓ 已叠加免责声明");
    }

    events.artifact(Step::Image, "image.png");
    events.step_done(Step::Image);
    println!("✓ 生图完成: {}", out.display());
    Ok(())
}

/// 公开入口
pub async fn run(
    id: Option<String>,
    reference: Vec<String>,
    user_prompt: Option<String>,
    disclaimer: bool,
) -> anyhow::Result<String> {
    let dir = match &id {
        Some(i) => super::task_dir(Path::new("output"), i),
        None => super::latest_task_dir(Path::new("output"))?,
    };
    let id = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let reference: Vec<PathBuf> = reference.into_iter().map(PathBuf::from).collect();
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let provider = provider::resolve_image(&cfg)?;
    let events = crate::task::TaskEvents::local(Path::new("output"), &id);
    run_with(&dir, reference, llm.as_ref(), provider.as_ref(), user_prompt.as_deref(), &events, disclaimer).await?;
    Ok(id)
}
