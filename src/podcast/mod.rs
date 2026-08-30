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

/// 把字幕条目写成 SRT 文件（标准字幕格式）
pub fn write_srt(path: &std::path::Path, entries: &[SubtitleEntry]) -> anyhow::Result<()> {
    fn ts(secs: f64) -> String {
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
