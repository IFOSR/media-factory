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

/// 探测可用的中文字体（用于 drawtext 叠加免责声明；覆盖 macOS / Linux / Windows）
fn find_cjk_font() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    const CANDIDATES: &[&str] = &[
        "C:\\Windows\\Fonts\\msyh.ttc",      // 微软雅黑
        "C:\\Windows\\Fonts\\msyhbd.ttc",   // 微软雅黑粗体
        "C:\\Windows\\Fonts\\simhei.ttf",   // 黑体
        "C:\\Windows\\Fonts\\msjh.ttc",     // 微软正黑（繁体）
    ];
    #[cfg(not(target_os = "windows"))]
    const CANDIDATES: &[&str] = &[
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",      // Ubuntu/Debian fonts-wqy-zenhei
        "/usr/share/fonts/wqy-zenhei/wqy-zenhei.ttc",        // 其他发行版路径
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.exists())
        .or_else(cjk_font_via_fontconfig)
}

/// 静态候选之外的兜底：问 fontconfig 要一个支持中文的字体文件（兼容任意发行版）
#[cfg(not(target_os = "windows"))]
fn cjk_font_via_fontconfig() -> Option<PathBuf> {
    let out = Command::new("fc-list").args([":lang=zh", "file"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 每行形如 "/path/to/font.ttc: "
    let path = stdout
        .lines()
        .find_map(|l| l.split(':').next().map(str::trim).filter(|s| !s.is_empty()))?;
    let p = PathBuf::from(path);
    p.exists().then_some(p)
}

#[cfg(target_os = "windows")]
fn cjk_font_via_fontconfig() -> Option<PathBuf> {
    None
}

/// 探测图片/视频的像素尺寸（宽, 高）
pub fn probe_dimensions(path: &Path) -> Option<(u32, u32)> {
    let out = Command::new(ffprobe_bin())
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().split(',');
    let w: u32 = it.next()?.trim().parse().ok()?;
    let h: u32 = it.next()?.trim().parse().ok()?;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

/// 按配图比例推导视频尺寸：**保持与配图一致的宽高比**，短边归一到 1080
/// （竖版 9:16 → 1080x1920；横版 16:9 → 1920x1080；方形 → 1080x1080）
pub fn video_size_for_image(image: &Path) -> (u32, u32) {
    let (w, h) = probe_dimensions(image).unwrap_or((1920, 1080));
    let short = w.min(h) as f64;
    let scale = 1080.0 / short;
    let even = |v: f64| -> u32 {
        let n = v.round() as u32;
        (n - n % 2).max(2)
    };
    (even(w as f64 * scale), even(h as f64 * scale))
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
        .ok_or_else(|| anyhow::anyhow!("未找到中文字体，无法在图片上叠加免责声明。Linux 请安装：apt install fonts-noto-cjk"))?;
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
/// 字幕烧录字体名：按平台选择系统内置中文字体（避免 libass 找不到指定字体时随意 fallback）
fn subtitle_font_name() -> String {
    if cfg!(target_os = "macos") {
        "Hiragino Sans GB".into()
    } else if cfg!(target_os = "windows") {
        "Microsoft YaHei".into()
    } else {
        // Linux：问 fontconfig 实际装了哪些中文字体族，优先 Noto，其次取第一个可用
        cjk_family_via_fontconfig().unwrap_or_else(|| "Noto Sans CJK SC".into())
    }
}

/// 通过 fontconfig 查询已安装的中文字体族名（libass 在 Linux 上依赖 fontconfig 解析）
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cjk_family_via_fontconfig() -> Option<String> {
    let out = Command::new("fc-list").args([":lang=zh", "family"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 每行形如 "Noto Sans CJK SC,Noto Sans CJK SC Regular"，取首个族名
    let families: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.split(',').next().map(str::trim).filter(|s| !s.is_empty()))
        .collect();
    for pref in ["Noto Sans CJK SC", "WenQuanYi Zen Hei", "WenQuanYi Micro Hei"] {
        if let Some(f) = families.iter().find(|f| **f == pref) {
            return Some(f.to_string());
        }
    }
    families.first().map(|f| f.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn cjk_family_via_fontconfig() -> Option<String> {
    None
}

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
            "subtitles='{p}':force_style='FontName={},FontSize=13,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,Shadow=0,BorderStyle=1,Alignment=2,MarginV=40,WrapStyle=0'",
            subtitle_font_name()
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

    #[test]
    fn video_size_follows_aspect() {
        let d = tempfile::tempdir().unwrap();
        let mk = |name: &str, w: u32, h: u32| -> PathBuf {
            let p = d.path().join(name);
            let st = Command::new(ffmpeg_bin())
                .args(["-y", "-f", "lavfi", "-i"])
                .arg(format!("color=c=red:s={w}x{h}"))
                .args(["-frames:v", "1"])
                .arg(&p)
                .status()
                .unwrap();
            assert!(st.success());
            p
        };
        // 竖版 9:16
        assert_eq!(video_size_for_image(&mk("p.png", 1080, 1920)), (1080, 1920));
        // 横版 16:9
        assert_eq!(video_size_for_image(&mk("l.png", 1920, 1080)), (1920, 1080));
        // 方形
        assert_eq!(video_size_for_image(&mk("s.png", 1024, 1024)), (1080, 1080));
        // 2:3 竖版（短边归一到 1080）
        let (w, h) = video_size_for_image(&mk("p23.png", 1024, 1536));
        assert_eq!(w, 1080);
        assert_eq!(h % 2, 0);
        assert!(h > 1600 && h < 1640, "height={h}");
    }

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

/// 统一的滤镜参数转义（subtitles / drawtext 用）
fn escape_filter_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").replace(':', "\\:").replace('\'', "\\'")
}

/// 合成成品视频：视觉层（动态画面或静态图）+ 音频 [+ 字幕烧录] [+ 常驻免责声明]
///
/// - `visual`：视觉层视频（动态模式）或静态图片（封面模式）
/// - `subtitle`：可选 SRT（烧录）
/// - `disclaimer`：可选免责声明（全程常驻，叠加在右上角）
/// - `font_px`：免责声明字号（按输出宽度换算）
pub fn mux_video(
    visual: &Path,
    audio: &Path,
    subtitle: Option<&Path>,
    disclaimer: Option<&str>,
    font_px: u32,
    out: &Path,
) -> anyhow::Result<()> {
    let mut filters: Vec<String> = Vec::new();

    if let Some(srt) = subtitle {
        anyhow::ensure!(
            has_subtitles_filter(),
            "当前 ffmpeg 未编译 libass，无法烧录字幕。\n请安装带 libass 的 ffmpeg"
        );
        filters.push(format!(
            "subtitles='{}':force_style='FontName={},FontSize=13,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,Shadow=0,BorderStyle=1,Alignment=2,MarginV=40,WrapStyle=0'",
            escape_filter_path(srt),
            subtitle_font_name()
        ));
    }

    let mut disc_text_file: Option<PathBuf> = None;
    if let Some(text) = disclaimer.map(str::trim).filter(|t| !t.is_empty()) {
        let font = find_cjk_font()
            .ok_or_else(|| anyhow::anyhow!("未找到中文字体，无法叠加免责声明（Linux 可装 fonts-noto-cjk）"))?;
        let textfile = out.with_extension("disclaimer.txt");
        std::fs::write(&textfile, text)?;
        let margin = (font_px / 2).max(10);
        filters.push(format!(
            "drawtext=fontfile='{}':textfile='{}':fontcolor=yellow:fontsize={}:borderw=2:bordercolor=black@0.8:x=w-text_w-{}:y={}",
            escape_filter_path(&font),
            escape_filter_path(&textfile),
            font_px,
            margin,
            margin
        ));
        disc_text_file = Some(textfile);
    }

    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(["-y", "-i"]).arg(visual).arg("-i").arg(audio);
    if !filters.is_empty() {
        cmd.args(["-vf", &filters.join(",")]);
    }
    cmd.args([
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
        "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "192k",
        "-shortest",
    ])
    .arg(out);

    let status = cmd.status()?;
    if let Some(f) = disc_text_file {
        let _ = std::fs::remove_file(f);
    }
    anyhow::ensure!(status.success(), "ffmpeg 合成失败");
    Ok(())
}
