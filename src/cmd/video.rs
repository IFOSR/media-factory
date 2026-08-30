use std::path::Path;

use crate::ffmpeg;

pub fn run_with(dir: &Path) -> anyhow::Result<()> {
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
    let dir = match &id {
        Some(i) => super::task_dir(Path::new("output"), i),
        None => super::latest_task_dir(Path::new("output"))?,
    };
    let id = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    run_with(&dir)?;
    Ok(id)
}
