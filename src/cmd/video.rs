use std::path::Path;

use crate::ffmpeg;

pub fn run_with(output_root: &Path, id: &str) -> anyhow::Result<()> {
    let dir = output_root.join(id);
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

    let out = dir.join("video.mp4");
    ffmpeg::make_video(&image, &audio, &out)?;
    println!("✓ 视频完成: {}", out.display());
    Ok(())
}

pub fn run(id: Option<String>) -> anyhow::Result<String> {
    let id = resolve_id(id)?;
    run_with(Path::new("output"), &id)?;
    Ok(id)
}

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
