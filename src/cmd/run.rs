use std::path::{Path, PathBuf};

use crate::cmd::{image, podcast, rewrite, video};
use crate::config::Config;
use crate::pi_rpc::PiRpcAgent;
use crate::podcast as podcast_backend;
use crate::provider;

/// 串联执行全部四步：改写 → 生图 → 播客 → 视频。
/// 任一步失败即停止，可用 `--id <id> <失败子命令>` 续跑。
pub async fn run(input: Option<String>, id: Option<String>, reference: Option<String>) -> anyhow::Result<()> {
    let cfg = Config::load(&Config::path())?;
    let llm = PiRpcAgent::new(cfg.tasks.llm.clone().map(|l| l.model))?;

    let source = rewrite::read_input(input)?;
    let id = rewrite::run_with(Path::new("output"), &source, id, &llm).await?;

    let img_provider = provider::resolve_image(&cfg)?;
    let reference = reference.map(PathBuf::from);
    image::run_with(Path::new("output"), &id, reference, &llm, img_provider.as_ref()).await?;

    let backend = podcast_backend::resolve_podcast(&cfg)?;
    podcast::run_with(Path::new("output"), &id, &llm, &backend, false).await?;

    video::run_with(Path::new("output"), &id)?;

    println!("完成！产物目录: output/{id}/");
    Ok(())
}
