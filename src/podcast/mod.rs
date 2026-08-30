pub mod script;
pub mod volc_podcast;
pub mod volc_proto;

use crate::config::{BuiltinKind, Config, ProviderConfig};
use crate::tts;

pub use script::{parse_script, to_nlp_texts};
pub use volc_podcast::{PodcastRequest, PodcastResult, VolcPodcast};

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
