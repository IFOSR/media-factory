use std::path::Path;

use crate::ffmpeg;
use crate::task::{Step, TaskEvents};

pub fn run_with(dir: &Path, events: &TaskEvents) -> anyhow::Result<()> {
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

    events.step_running(Step::Video);
    let out = dir.join("video.mp4");
    let srt = dir.join("subtitle.srt");
    let subtitle = if srt.exists() { Some(srt.as_path()) } else { None };
    ffmpeg::make_video(&image, &audio, subtitle, &out)?;
    events.artifact(Step::Video, "video.mp4");
    events.step_done(Step::Video);
    if subtitle.is_some() {
        println!("✓ 视频完成（含字幕）: {}", out.display());
    } else {
        println!("✓ 视频完成: {}", out.display());
    }
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
    let events = crate::task::TaskEvents::local(Path::new("output"), &id);
    run_with(&dir, &events)?;
    Ok(id)
}
