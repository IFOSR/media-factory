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

/// 核心流程（可注入 llm 与 provider 以便测试）
pub async fn run_with(
    dir: &Path,
    reference: Option<PathBuf>,
    llm: &dyn LlmAgent,
    provider: &dyn ImageProvider,
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
    events.step_running(Step::Image);
    let img_prompt = distill_prompt(llm, &text, user_prompt).await?;
    println!("图像 prompt: {}", img_prompt);

    if reference.is_some() && !provider.supports_reference() {
        eprintln!("⚠️  当前生图 provider 不支持参考图，已忽略 --ref 降级为纯文本生图");
    }
    let reference = if provider.supports_reference() { reference } else { None };

    let bytes = provider
        .generate(&ImageRequest {
            prompt: img_prompt,
            reference_image: reference,
        })
        .await?;

    let out = dir.join("image.png");
    std::fs::write(&out, bytes)?;
    events.artifact(Step::Image, "image.png");
    events.step_done(Step::Image);
    println!("✓ 生图完成: {}", out.display());
    Ok(())
}

/// 公开入口
pub async fn run(
    id: Option<String>,
    reference: Option<String>,
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
    let reference = reference.map(PathBuf::from);
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let provider = provider::resolve_image(&cfg)?;
    let events = crate::task::TaskEvents::local(Path::new("output"), &id);
    run_with(&dir, reference, llm.as_ref(), provider.as_ref(), user_prompt.as_deref(), &events).await?;
    Ok(id)
}
