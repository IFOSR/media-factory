use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub tasks: TaskSelections,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TaskSelections {
    /// 语言模型选择（pi agent），存 "provider/model[:thinking]" 字符串；None = pi 默认模型
    pub llm: Option<LlmSelection>,
    pub image: Option<TaskSelection>,
    pub podcast: Option<TaskSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSelection {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSelection {
    pub provider: String,
}

/// 注意：语言模型（改写/脚本/图像 prompt）没有 ProviderConfig —— 由 pi 管理。
/// 这里只为生图 / 播客任务配置 provider。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "builtin")]
    Builtin {
        kind: BuiltinKind,
        api_key: String,
        #[serde(default)]
        extra: HashMap<String, String>, // volc-tts/volc-podcast 需要 appid/cluster 等
    },
    #[serde(rename = "openai-compatible")]
    Custom {
        base_url: String,
        api_key: String,
        model: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinKind {
    NanoBanana,     // 生图（Gemini Image，支持参考图）
    OpenAiImage,    // 生图（gpt-image）
    DoubaoSeedream, // 生图
    VolcPodcast,    // 播客大模型（推荐默认，端到端双人播客；extra 存 appid）
    GeminiTts,      // 播客 TTS（fallback 路径）
    OpenAiTts,      // 播客 TTS（fallback 路径）
    VolcTts,        // 播客 TTS（fallback 路径；extra 存 appid/cluster）
}

impl BuiltinKind {
    pub fn supports(&self, task: MediaTaskKind) -> bool {
        use BuiltinKind::*;
        use MediaTaskKind::*;
        matches!(
            (self, task),
            (NanoBanana, Image)
                | (OpenAiImage, Image)
                | (DoubaoSeedream, Image)
                | (VolcPodcast, Podcast)
                | (GeminiTts, Podcast)
                | (OpenAiTts, Podcast)
                | (VolcTts, Podcast)
        )
    }

    /// 是否为端到端播客 provider（走播客 API，不需脚本/拼接 fallback 路径）
    pub fn is_podcast_api(&self) -> bool {
        matches!(self, BuiltinKind::VolcPodcast)
    }
}

/// 需要直连 API 的媒体任务（语言模型任务不走这里）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTaskKind {
    Image,
    Podcast,
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap()
            .join(".media-factory")
            .join("config.yaml")
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_yaml::to_string(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let mut cfg = Config::default();
        cfg.tasks.llm = Some(LlmSelection {
            model: "google/gemini-2.5-pro".into(),
        });
        cfg.tasks.image = Some(TaskSelection {
            provider: "nano-banana".into(),
        });
        cfg.providers.insert(
            "nano-banana".into(),
            ProviderConfig::Builtin {
                kind: BuiltinKind::NanoBanana,
                api_key: "k123".into(),
                extra: Default::default(),
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.tasks.llm.unwrap().model,
            "google/gemini-2.5-pro"
        );
        match &loaded.providers["nano-banana"] {
            ProviderConfig::Builtin { api_key, .. } => assert_eq!(api_key, "k123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn custom_provider_roundtrip() {
        let mut cfg = Config::default();
        cfg.providers.insert(
            "my-img".into(),
            ProviderConfig::Custom {
                base_url: "https://api.example.com/v1".into(),
                api_key: "sk-x".into(),
                model: "my-image-model".into(),
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        match &loaded.providers["my-img"] {
            ProviderConfig::Custom {
                base_url, model, ..
            } => {
                assert_eq!(base_url, "https://api.example.com/v1");
                assert_eq!(model, "my-image-model");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn volc_tts_extra_roundtrip() {
        let mut extra = HashMap::new();
        extra.insert("appid".to_string(), "123".to_string());
        extra.insert("cluster".to_string(), "volcano_tts".to_string());
        let mut cfg = Config::default();
        cfg.providers.insert(
            "volc-tts".into(),
            ProviderConfig::Builtin {
                kind: BuiltinKind::VolcTts,
                api_key: "tok".into(),
                extra,
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        match &loaded.providers["volc-tts"] {
            ProviderConfig::Builtin { extra, .. } => assert_eq!(extra["appid"], "123"),
            _ => panic!("wrong variant"),
        }
    }
}
