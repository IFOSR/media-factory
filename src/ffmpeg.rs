//! ffmpeg 封装（subprocess）

use std::path::{Path, PathBuf};
use std::process::Command;

/// 找到可用的 ffmpeg 二进制：优先用 ffmpeg-full（含 libass，支持字幕烧录）
pub fn ffmpeg_bin() -> PathBuf {
    let full = PathBuf::from("/usr/local/opt/ffmpeg-full/bin/ffmpeg");
    if full.exists() {
        return full;
    }
    let full_m1 = PathBuf::from("/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg");
    if full_m1.exists() {
        return full_m1;
    }
    PathBuf::from("ffmpeg")
}

fn ffprobe_bin() -> PathBuf {
    let dir = ffmpeg_bin().parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let p = dir.join("ffprobe");
    if p.exists() {
        p
    } else {
        PathBuf::from("ffprobe")
    }
}

/// 探测可用的中文字体（用于 drawtext 叠加免责声明）
fn find_cjk_font() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.exists())
}

/// 探测图片宽度（像素），用于按宽度自适应免责声明字号
fn probe_image_width(image: &Path) -> Option<u32> {
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width", "-of", "csv=p=0"])
        .arg(image)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// 在图片右上角叠加免责声明文字（黄色小字 + 黑描边，不遮挡主体内容）；输出写入 out
pub fn overlay_disclaimer(image: &Path, text: &str, out: &Path) -> anyhow::Result<()> {
    let font = find_cjk_font()
        .ok_or_else(|| anyhow::anyhow!("未找到中文字体，无法在图片上叠加免责声明"))?;
    // 文字写入临时文件，避免 drawtext 参数转义问题
    let textfile = out.with_extension("disclaimer.txt");
    std::fs::write(&textfile, text)?;

    let width = probe_image_width(image).unwrap_or(1024);
    // 小字号，按宽度自适应但保持克制（1024px → 约 25px）
    let fontsize = (width / 40).clamp(16, 36);
    let margin = (fontsize / 2).max(10);

    // 路径转义：drawtext 参数用单引号包裹，内部转义冒号与单引号
    let esc = |s: &str| s.replace(':', "\\:").replace('\'', "\\'");
    // 右上角：黄色文字 + 黑色描边，无背景框，尽量不遮挡内容
    let vf = format!(
        "drawtext=fontfile='{}':textfile='{}':fontcolor=yellow:fontsize={}:borderw=2:bordercolor=black@0.8:x=w-text_w-{}:y={}",
        esc(&font.to_string_lossy()),
        esc(&textfile.to_string_lossy()),
        fontsize,
        margin,
        margin,
    );

    let status = Command::new(ffmpeg_bin())
        .args(["-y", "-i"])
        .arg(image)
        .args(["-vf", &vf, "-frames:v", "1"])
        .arg(out)
        .status()?;
    anyhow::ensure!(status.success(), "ffmpeg 叠加免责声明失败");
    let _ = std::fs::remove_file(&textfile);
    Ok(())
}

pub fn require_ffmpeg() -> anyhow::Result<()> {
    let ok = Command::new(ffmpeg_bin()).arg("-version").output().is_ok();
    anyhow::ensure!(ok, "未找到 ffmpeg，请先安装（brew install ffmpeg / apt install ffmpeg）");
    Ok(())
}

/// 检测 ffmpeg 是否支持字幕滤镜（需 libass）
pub fn has_subtitles_filter() -> bool {
    Command::new(ffmpeg_bin())
        .args(["-hide_banner", "-filters"])
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.lines()
                .any(|l| l.split_whitespace().any(|t| t == "subtitles"))
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

    let status = Command::new(ffmpeg_bin())
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
    let mut cmd = Command::new(ffmpeg_bin());
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
        // 字幕：白字 + 黑描边（无底框），底部居中，单行，智能换行不拆单词
        let vf = format!(
            "subtitles='{p}':force_style='FontName=Hiragino Sans GB,FontSize=13,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,Shadow=0,BorderStyle=1,Alignment=2,MarginV=40,WrapStyle=0'"
        );
        cmd.args(["-vf", &vf]);
    }

    cmd.args([
        "-c:v", "libx264", "-preset", "veryfast", "-tune", "stillimage",
        "-r", "15",
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
        let status = Command::new(ffmpeg_bin())
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
