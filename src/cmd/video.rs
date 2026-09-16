//! 步骤 4/5：视频合成。
//!
//! - `dynamic`（默认）：把分镜交给算力机渲染动态解释画面（失败/不可用时按配置降级为 cover）
//! - `cover`：封面图贯穿全片（本机 ffmpeg，最快）

use std::path::Path;

use crate::config::Config;
use crate::render;
use crate::task::{Step, TaskEvents};

/// 仅封面模式（同步，供测试与内部调用）
#[allow(dead_code)]
pub fn run_with(dir: &Path, events: &TaskEvents) -> anyhow::Result<()> {
    let out = dir.join("video.mp4");
    render::compose_cover(dir, events)?;
    events.artifact(Step::Video, "video.mp4");
    events.step_done(Step::Video);
    println!("✓ 视频完成: {}", out.display());
    Ok(())
}

/// 完整合成（异步）：按配置选择画面模式，动态失败时降级为封面模式
pub async fn compose(
    dir: &Path,
    cfg: &Config,
    task_id: &str,
    events: &TaskEvents,
    disclaimer: Option<String>,
) -> anyhow::Result<()> {
    let image = dir.join("image.png");
    let audio = dir.join("podcast.mp3");
    anyhow::ensure!(
        image.exists(),
        "缺少 {}/image.png，请先运行 `media-factory image`",
        dir.display()
    );
    anyhow::ensure!(
        audio.exists(),
        "缺少 {}/podcast.mp3，请先运行 `media-factory podcast`",
        dir.display()
    );

    // 视频尺寸跟随配图比例（竖版 9:16 / 横版 16:9 / 方形），避免比例不匹配
    let (vw, vh) = crate::ffmpeg::video_size_for_image(&image);
    let dynamic = cfg.video.is_dynamic();
    let srt = dir.join("subtitle.srt");
    let subtitle = if srt.exists() { Some(srt) } else { None };
    let out = dir.join("video.mp4");
    // 重新合成前先清理旧成品，避免"文件已存在导致直接返回"的假成功
    if out.exists() {
        let prev = dir.join("video.prev.mp4");
        let _ = std::fs::remove_file(&prev);
        let _ = std::fs::rename(&out, &prev);
    }

    if dynamic {
        events.log(Step::Video, &format!("视频尺寸 {vw}x{vh}（跟随配图比例）"));
        let result = render::compose_dynamic(dir, cfg, task_id, events, disclaimer.clone(), (vw, vh)).await;
        match result {
            Ok(()) => {
                events.artifact(Step::Video, "video.mp4");
                events.step_done(Step::Video);
                println!("✓ 视频完成（动态解释画面）: {}", out.display());
                return Ok(());
            }
            Err(e) => {
                // 降级策略：cover = 立即出静态版（默认）；queue = 先把错误抛出交给上层重试
                let on_unavailable = cfg.video.dynamic.on_unavailable.to_lowercase();
                if on_unavailable == "queue" {
                    events.log(Step::Video, &format!("动态渲染失败：{e}"));
                    anyhow::bail!("动态渲染不可用（on_unavailable=queue）：{e}");
                }
                events.log(Step::Video, &format!("动态渲染失败（{e}），降级为封面图贯穿"));
            }
        }
    }

    // 封面模式（或用降级路径）
    let image2 = image.clone();
    let audio2 = audio.clone();
    let subtitle2 = subtitle.clone();
    let out2 = out.clone();
    let disclaimer2 = disclaimer.clone();
    events.log(Step::Video, "封面模式：ffmpeg 合成（封面图 + 音频 + 字幕 + 免责声明）");
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        // 先用图片生成无字幕底片，再叠加字幕与免责声明，保证两者样式统一
        let silent = out2.with_extension("base.mp4");
        crate::ffmpeg::make_video(&image2, &audio2, None, &silent)?;
        let font_px = (vw.min(vh) / 52).max(14);
        crate::ffmpeg::mux_video(
            &silent,
            &audio2,
            subtitle2.as_deref(),
            disclaimer2.as_deref(),
            font_px,
            &out2,
        )?;
        let _ = std::fs::remove_file(&silent);
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("合成线程失败: {e}"))??;

    events.artifact(Step::Video, "video.mp4");
    events.step_done(Step::Video);
    println!("✓ 视频完成（封面图贯穿）: {}", out.display());
    Ok(())
}

/// 公开入口（CLI）
pub async fn run(id: Option<String>) -> anyhow::Result<String> {
    let dir = match &id {
        Some(i) => super::task_dir(crate::config::output_root().as_path(), i),
        None => super::latest_task_dir(crate::config::output_root().as_path())?,
    };
    let id = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let cfg = Config::load(&Config::path())?;
    let events = crate::task::TaskEvents::local(crate::config::output_root().as_path(), &id);
    events.step_running(Step::Video);
    // CLI 场景下若动态模式需要回传地址，缺失时会给出明确提示并降级
    compose(&dir, &cfg, &id, &events, None).await?;
    Ok(id)
}
