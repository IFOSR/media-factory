pub mod script;
pub mod volc_podcast;
pub mod volc_proto;

use crate::config::{BuiltinKind, Config, ProviderConfig};
use crate::tts;

pub use script::{parse_script, to_nlp_texts};
pub use volc_podcast::{PodcastRequest, PodcastResult, VolcPodcast};

/// 一条字幕（起止时间单位为秒）
#[derive(Debug, Clone)]
pub struct SubtitleEntry {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 把一段话按句子/长度切分成适合字幕的短行（每行约 max_chars 字以内）
pub fn split_subtitle_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        let n = cur.chars().count();
        // 句末标点必断；逗号在行接近满时断；行满硬切
        let hard = matches!(ch, '。' | '！' | '？' | '；' | '…');
        let soft = matches!(ch, '，' | '、');
        let should_break = hard || (soft && n >= max_chars - 4) || n >= max_chars;
        if should_break {
            let line = cur.trim().to_string();
            if !line.is_empty() {
                lines.push(line);
            }
            cur = String::new();
        }
    }
    let line = cur.trim().to_string();
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// 把一轮的整段文本按句子切分，并在 [start, end] 内按字数比例分配时间
pub fn split_subtitle_entries(
    speaker: &str,
    text: &str,
    start: f64,
    end: f64,
) -> Vec<SubtitleEntry> {
    let lines = split_subtitle_text(text, 20);
    if lines.is_empty() {
        return vec![];
    }
    let total: usize = lines.iter().map(|l| l.chars().count()).sum();
    let dur = (end - start).max(0.0);
    let mut entries = Vec::new();
    let mut t = start;
    for (i, line) in lines.iter().enumerate() {
        let frac = if total > 0 {
            line.chars().count() as f64 / total as f64
        } else {
            1.0 / lines.len() as f64
        };
        let d = dur * frac;
        // 说话人只在首行前缀，其余行只显示正文
        let text = if i == 0 {
            format!("{speaker}：{line}")
        } else {
            line.clone()
        };
        entries.push(SubtitleEntry {
            start: t,
            end: t + d,
            text,
        });
        t += d;
    }
    entries
}

/// 把字幕条目写成 SRT 文件（标准字幕格式）
pub fn write_srt(path: &std::path::Path, entries: &[SubtitleEntry]) -> anyhow::Result<()> {    fn ts(secs: f64) -> String {
        let ms = (secs.fract() * 1000.0).round() as u64;
        let total = secs.floor() as u64;
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        format!("{h:02}:{m:02}:{s:02},{ms:03}")
    }
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&format!("{} --> {}\n", ts(e.start), ts(e.end)));
        out.push_str(&format!("{}\n\n", e.text));
    }
    std::fs::write(path, out)?;
    Ok(())
}

/// 播客后端：火山播客 API（端到端）或通用 TTS（fallback）
pub enum PodcastBackend {
    Volc(VolcPodcast),
    Tts(Box<dyn tts::TtsProvider>),
}

const DEFAULT_SPEAKER_A: &str = "zh_female_mizaitongxue_v2_saturn_bigtts";
const DEFAULT_SPEAKER_B: &str = "zh_male_dayixiansheng_v2_saturn_bigtts";

pub fn resolve_podcast(cfg: &Config) -> anyhow::Result<PodcastBackend> {
    let selection = cfg
        .tasks
        .podcast
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("未配置播客 provider，请先运行 `media-factory config`"))?;
    let provider = cfg
        .providers
        .get(&selection.provider)
        .ok_or_else(|| anyhow::anyhow!("播客 provider 不存在: {}", selection.provider))?;

    match provider {
        ProviderConfig::Builtin {
            kind, api_key, extra, ..
        } if *kind == BuiltinKind::VolcPodcast => {
            let appid = extra
                .get("appid")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("volc-podcast 缺少 appid，请在配置向导中补全"))?;
            let speaker_a = extra
                .get("speaker1")
                .cloned()
                .unwrap_or_else(|| DEFAULT_SPEAKER_A.to_string());
            let speaker_b = extra
                .get("speaker2")
                .cloned()
                .unwrap_or_else(|| DEFAULT_SPEAKER_B.to_string());
            Ok(PodcastBackend::Volc(VolcPodcast::new(
                appid,
                api_key.clone(),
                (speaker_a, speaker_b),
            )))
        }
        _ => Ok(PodcastBackend::Tts(tts::resolve_tts(cfg)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sentences_into_lines() {
        let lines = split_subtitle_text("今天咱们来聊一聊字节跳动新发布的豆包工作。它是怎么做到通过飞书的深度集成。", 20);
        assert!(lines.len() >= 2);
        for l in &lines {
            assert!(l.chars().count() <= 20, "行过长: {l}");
        }
    }

    #[test]
    fn split_entries_distributes_timing() {
        let entries = split_subtitle_entries("主持人", "第一句话。第二句话。", 0.0, 10.0);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].text.starts_with("主持人："));
        assert!(!entries[1].text.starts_with("主持人："));
        // 时间连续且总时长为 10s
        assert!((entries[0].start - 0.0).abs() < 1e-6);
        assert!((entries[1].end - 10.0).abs() < 1e-6);
        assert!((entries[0].end - entries[1].start).abs() < 1e-6);
    }
}
