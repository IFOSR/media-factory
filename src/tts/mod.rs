//! 通用 TTS（fallback 路径）：分段合成 + ffmpeg 拼接。
//! 只有 OpenAI 兼容 /audio/speech 已实现；gemini-tts / volc-tts 暂未实现。

use crate::config::{BuiltinKind, Config, ProviderConfig};

#[async_trait::async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>>;
    /// (host, guest) 双音色
    fn default_voices(&self) -> (String, String);
}

pub struct OpenAiTts {
    base_url: String,
    api_key: String,
    model: String,
    voices: (String, String),
    client: reqwest::Client,
}

impl OpenAiTts {
    pub fn new(base_url: String, api_key: String, model: String, voices: (String, String)) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            voices,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl TtsProvider for OpenAiTts {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let body = serde_json::json!({
            "model": self.model,
            "voice": voice,
            "input": text
        });
        let resp = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "TTS API 返回错误 {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
        Ok(resp.bytes().await?.to_vec())
    }

    fn default_voices(&self) -> (String, String) {
        self.voices.clone()
    }
}

pub fn resolve_tts(cfg: &Config) -> anyhow::Result<Box<dyn TtsProvider>> {
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
        ProviderConfig::Custom {
            base_url,
            api_key,
            model,
        } => Ok(Box::new(OpenAiTts::new(
            base_url.clone(),
            api_key.clone(),
            model.clone(),
            ("alloy".into(), "onyx".into()),
        ))),
        ProviderConfig::Builtin { kind, api_key, .. } => match kind {
            BuiltinKind::OpenAiTts => Ok(Box::new(OpenAiTts::new(
                "https://api.openai.com/v1".into(),
                api_key.clone(),
                "tts-1".into(),
                ("alloy".into(), "onyx".into()),
            ))),
            other => anyhow::bail!("TTS provider {:?} 暂未实现", other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn openai_tts_synthesize() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/audio/speech"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "voice": "alloy"
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(b"MP3"))
            .mount(&server)
            .await;

        let t = OpenAiTts::new(
            server.uri(),
            "k".into(),
            "tts-1".into(),
            ("alloy".into(), "onyx".into()),
        );
        let bytes = t.synthesize("你好", "alloy").await.unwrap();
        assert_eq!(bytes, b"MP3");
    }
}
