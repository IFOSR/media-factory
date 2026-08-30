use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::llm::LlmAgent;
use crate::pi_rpc::PiRpcAgent;
use crate::provider::{self, ImageProvider, ImageRequest};

/// 读取图像 prompt 模板：优先运行时读 cwd/prompts/image_prompt.txt，缺失用嵌入默认。
fn image_prompt_template() -> String {
    if let Ok(t) = std::fs::read_to_string("prompts/image_prompt.txt") {
        return t;
    }
    include_str!("../../prompts/image_prompt.txt").to_string()
}

/// 用 pi 从改写文案提炼图像 prompt
pub async fn distill_prompt(llm: &dyn LlmAgent, text: &str) -> anyhow::Result<String> {
    let prompt = image_prompt_template().replace("{{TEXT}}", text);
    let out = llm.complete(&prompt).await?;
    Ok(out.trim().to_string())
}

/// 核心流程（可注入 llm 与 provider 以便测试）
pub async fn run_with(
    output_root: &Path,
    id: &str,
    reference: Option<PathBuf>,
    llm: &dyn LlmAgent,
    provider: &dyn ImageProvider,
) -> anyhow::Result<()> {
    let dir = output_root.join(id);
    let rewritten_path = dir.join("rewritten.md");
    anyhow::ensure!(
        rewritten_path.exists(),
        "缺少 {}/rewritten.md，请先运行 `media-factory rewrite`",
        dir.display()
    );

    let text = std::fs::read_to_string(&rewritten_path)?;
    let img_prompt = distill_prompt(llm, &text).await?;
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
    println!("✓ 生图完成: {}", out.display());
    Ok(())
}

/// 公开入口
pub async fn run(id: Option<String>, reference: Option<String>) -> anyhow::Result<String> {
    let id = resolve_id(id)?;
    let reference = reference.map(PathBuf::from);
    let cfg = Config::load(&Config::path())?;
    let llm = PiRpcAgent::new(cfg.tasks.llm.clone().map(|l| l.model))?;
    let provider = provider::resolve_image(&cfg)?;
    run_with(Path::new("output"), &id, reference, &llm, provider.as_ref()).await?;
    Ok(id)
}

/// 任务 id：显式指定 or 取 output/ 下最新目录
fn resolve_id(id: Option<String>) -> anyhow::Result<String> {
    if let Some(id) = id {
        return Ok(id);
    }
    let mut dirs: Vec<_> = std::fs::read_dir("output")?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    dirs.last()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .ok_or_else(|| anyhow::anyhow!("未找到任务目录，请用 --id 指定或先运行 rewrite"))
}
