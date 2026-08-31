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
    /// 语言模型 provider（与生图/播客同一套逻辑）：provider 指向 providers 里的 key
    #[serde(default)]
    pub llm: Option<LlmSelection>,
    pub image: Option<TaskSelection>,
    pub podcast: Option<TaskSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LlmSelection {
    /// 新格式：{ provider: "..." }（指向 providers 里的 key，如 pi / openai-compatible 自定义）
    Provider(TaskSelection),
    /// 旧格式：{ model: "..." }（pi 模型字符串），加载时迁移为 pi provider
    Model(LlmModel),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmModel {
    pub model: String,
}

impl LlmSelection {
    pub fn provider_key(&self) -> &str {
        match self {
            LlmSelection::Provider(t) => &t.provider,
            LlmSelection::Model(_) => "pi",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSelection {
    pub provider: String,
}

/// 注意：生图 / 播客 / 语言模型（LLM）任务都通过 providers 里的 provider 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "builtin")]
    Builtin {
        kind: BuiltinKind,
        api_key: String,
        #[serde(default)]
        extra: HashMap<String, String>, // volc-tts/volc-podcast 需要 appid/cluster；pi 用 model
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
    Pi,              // 语言模型：pi agent（默认），extra.model 存 pi 模型字符串
    NanoBanana,      // 生图（Gemini Image，支持参考图）
    OpenAiImage,     // 生图（gpt-image）
    DoubaoSeedream,  // 生图
    VolcPodcast,     // 播客大模型（推荐默认，端到端双人播客；extra 存 appid）
    GeminiTts,       // 播客 TTS（fallback 路径）
    OpenAiTts,       // 播客 TTS（fallback 路径）
    VolcTts,         // 播客 TTS（fallback 路径；extra 存 appid/cluster）
}

impl BuiltinKind {
    pub fn supports(&self, task: TaskKind) -> bool {
        use BuiltinKind::*;
        use TaskKind::*;
        matches!(
            (self, task),
            (Pi, Llm)
                | (NanoBanana, Image)
                | (OpenAiImage, Image)
                | (DoubaoSeedream, Image)
                | (VolcPodcast, Podcast)
                | (GeminiTts, Podcast)
                | (OpenAiTts, Podcast)
                | (VolcTts, Podcast)
        )
    }
}

/// 任务类型：语言模型 / 生图 / 播客
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TaskKind {
    Llm,
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
        let mut cfg: Self = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        cfg.migrate_legacy_llm();
        Ok(cfg)
    }

    /// 把旧的 { model: "..." } 迁移为 pi provider
    fn migrate_legacy_llm(&mut self) {
        if let Some(LlmSelection::Model(m)) = &self.tasks.llm {
            let mut extra = HashMap::new();
            extra.insert("model".to_string(), m.model.clone());
            self.providers.insert(
                "pi".to_string(),
                ProviderConfig::Builtin {
                    kind: BuiltinKind::Pi,
                    api_key: String::new(),
                    extra,
                },
            );
            self.tasks.llm = Some(LlmSelection::Provider(TaskSelection {
                provider: "pi".to_string(),
            }));
        }
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
        cfg.tasks.llm = Some(LlmSelection::Provider(TaskSelection {
            provider: "pi".into(),
        }));
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
        assert_eq!(loaded.tasks.llm.unwrap().provider_key(), "pi");
        match &loaded.providers["nano-banana"] {
            ProviderConfig::Builtin { api_key, .. } => assert_eq!(api_key, "k123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn legacy_llm_migrates_to_pi_provider() {
        let yaml = "tasks:\n  llm:\n    model: google/gemini-2.5-pro\nproviders: {}\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, yaml).unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.tasks.llm.unwrap().provider_key(), "pi");
        match &cfg.providers["pi"] {
            ProviderConfig::Builtin { kind, extra, .. } => {
                assert_eq!(*kind, BuiltinKind::Pi);
                assert_eq!(extra["model"], "google/gemini-2.5-pro");
            }
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
