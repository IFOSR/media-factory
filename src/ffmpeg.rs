//! ffmpeg 封装（subprocess）

use std::path::Path;
use std::process::Command;

pub fn require_ffmpeg() -> anyhow::Result<()> {
    let ok = Command::new("ffmpeg").arg("-version").output().is_ok();
    anyhow::ensure!(ok, "未找到 ffmpeg，请先安装（brew install ffmpeg / apt install ffmpeg）");
    Ok(())
}

/// 检测 ffmpeg 是否支持字幕滤镜（需 libass）
pub fn has_subtitles_filter() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.lines().any(|l| l.trim_start().starts_with("subtitles"))
        })
        .unwrap_or(false)
}

/// 用 concat demuxer 拼接多个 mp3 段
pub fn concat_mp3(seg_files: &[impl AsRef<Path>], out: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(!seg_files.is_empty(), "没有可拼接的音频段");
    let list = out.with_extension("list.txt");
    let mut content = String::new();
    for f in seg_files {
        content.push_str(&format!("file '{}'\n", f.as_ref().display()));
    }
    std::fs::write(&list, content)?;

    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "concat", "-safe", "0", "-i"])
        .arg(&list)
        .args(["-c", "copy"])
        .arg(out)
        .status()?;
    anyhow::ensure!(status.success(), "ffmpeg 拼接失败");
    let _ = std::fs::remove_file(&list);
    Ok(())
}

/// 静态图 + 音频合成视频（图片贯穿全片，时长 = 音频时长）；可选烧入字幕
pub fn make_video(image: &Path, audio: &Path, subtitle: Option<&Path>, out: &Path) -> anyhow::Result<()> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-loop", "1", "-i"])
        .arg(image)
        .arg("-i")
        .arg(audio);

    if let Some(srt) = subtitle {
        anyhow::ensure!(
            has_subtitles_filter(),
            "当前 ffmpeg 未编译 libass，无法烧录字幕。\n请运行：brew install libass && brew reinstall ffmpeg"
        );
        // 转义路径中的特殊字符（subtitles 滤镜要求）
        let p = srt.to_string_lossy().replace('\\', "/").replace(':', "\\:");
        // 美观中文字幕样式：白色字体 + 黑色描边 + 半透明底框 + 底部居中
        let vf = format!(
            "subtitles='{p}':force_style='FontName=Hiragino Sans GB,FontSize=28,PrimaryColour=&H00FFFFFF,OutlineColour=&H00101010,BackColour=&H78000000,BorderStyle=3,Outline=1,Shadow=0,Alignment=2,MarginV=40,MarginL=30,MarginR=30'"
        );
        cmd.args(["-vf", &vf]);
    }

    cmd.args([
        "-c:v", "libx264", "-tune", "stillimage",
        "-c:a", "aac", "-b:a", "192k",
        "-pix_fmt", "yuv420p", "-shortest",
    ])
    .arg(out);

    let status = cmd.status()?;
    anyhow::ensure!(status.success(), "ffmpeg 合成视频失败");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_mp3(path: &Path, seconds: u32) {
        let status = Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"))
            .args(["-codec:a", "libmp3lame"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn concat_two_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        make_test_mp3(&a, 1);
        make_test_mp3(&b, 1);
        let out = dir.path().join("out.mp3");
        concat_mp3(&[&a, &b], &out).unwrap();
        assert!(out.exists());
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
    }
}
