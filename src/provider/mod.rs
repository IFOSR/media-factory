use std::path::PathBuf;

use base64::Engine as _;

use crate::config::{BuiltinKind, Config, ProviderConfig};

/// 生图尺寸（比例）选择
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum ImageSize {
    #[default]
    Square,
    Portrait,
    Landscape,
}

impl ImageSize {
    pub fn parse(s: &str) -> Self {
        match s {
            "portrait" => ImageSize::Portrait,
            "landscape" => ImageSize::Landscape,
            _ => ImageSize::Square,
        }
    }
}

pub struct ImageRequest {
    pub prompt: String,
    pub reference_images: Vec<PathBuf>,
    pub size: ImageSize,
}

#[async_trait::async_trait]
pub trait ImageProvider: Send + Sync {
    /// 生成图片，返回 PNG 字节
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>>;
    fn supports_reference(&self) -> bool {
        false
    }
}

/// 官方 Gemini 图像生成 API（默认生图 provider）。
/// POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent
/// header `x-goog-api-key`；`generationConfig.responseModalities: ["TEXT","IMAGE"]`；
/// 参考图通过 `parts[].inline_data` 内联。
pub struct GeminiImage {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiImage {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    /// 测试用：覆盖 base_url
    #[cfg(test)]
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[async_trait::async_trait]
impl ImageProvider for GeminiImage {
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>> {
        let mut parts: Vec<serde_json::Value> = vec![serde_json::json!({"text": req.prompt})];
        for ref_path in &req.reference_images {
            let bytes = std::fs::read(ref_path)?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            parts.push(serde_json::json!({
                "inline_data": {"mime_type": "image/png", "data": b64}
            }));
        }

        let aspect = match req.size {
            ImageSize::Square => "1:1",
            ImageSize::Portrait => "9:16",
            ImageSize::Landscape => "16:9",
        };
        let body = serde_json::json!({
            "contents": [{"role": "user", "parts": parts}],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"],
                "imageConfig": {"aspectRatio": aspect}
            }
        });

        let resp = self
            .client
            .post(format!(
                "{}/models/{}:generateContent",
                self.base_url, self.model
            ))
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        anyhow::ensure!(
            resp.status().is_success(),
            "Gemini 生图 API 返回错误 {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );

        let v: serde_json::Value = resp.json().await?;
        for part in v["candidates"][0]["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(b64) = part["inlineData"]["data"].as_str() {
                return Ok(base64::engine::general_purpose::STANDARD.decode(b64)?);
            }
        }
        anyhow::bail!("Gemini 生图响应中未找到图片数据（inlineData）");
    }

    fn supports_reference(&self) -> bool {
        true
    }
}

/// OpenAI 兼容 /images/generations 实现（openai-image / 自定义 Other 共用）。
/// 第三方 nano-banana 服务（如 ModelGate）也走这里，由用户在 Other 中配置。
pub struct OpenAiImages {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub supports_reference: bool,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    client: reqwest::Client,
}

impl OpenAiImages {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        supports_reference: bool,
        extra_body: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            supports_reference,
            extra_body,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    async fn fetch_url(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let resp = self.client.get(url).send().await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "下载生成图片失败: {}",
            resp.status()
        );
        Ok(resp.bytes().await?.to_vec())
    }
}

#[async_trait::async_trait]
impl ImageProvider for OpenAiImages {
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>> {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), self.model.clone().into());
        body.insert("prompt".into(), req.prompt.clone().into());
        let size = match req.size {
            ImageSize::Square => "1024x1024",
            ImageSize::Portrait => "1024x1536",
            ImageSize::Landscape => "1536x1024",
        };
        body.insert("size".into(), size.into());
        for (k, v) in &self.extra_body {
            body.insert(k.clone(), v.clone());
        }

        if self.supports_reference {
            // OpenAI 兼容接口一般只支持单张参考图；多张时取第一张
            if let Some(ref_path) = req.reference_images.first() {
                let bytes = std::fs::read(ref_path)?;
                let data_url = format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                );
                body.insert("image".into(), data_url.into());
            }
        } else if !req.reference_images.is_empty() {
            anyhow::bail!("当前生图 provider 不支持参考图");
        }

        let resp = self
            .client
            .post(format!("{}/images/generations", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::Value::Object(body))
            .send()
            .await?;

        anyhow::ensure!(
            resp.status().is_success(),
            "生图 API 返回错误 {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );

        let v: serde_json::Value = resp.json().await?;
        let item = &v["data"][0];
        if let Some(url) = item["url"].as_str() {
            return self.fetch_url(url).await;
        }
        for key in ["content", "b64_json"] {
            if let Some(b64) = item[key].as_str() {
                return Ok(base64::engine::general_purpose::STANDARD.decode(b64)?);
            }
        }
        anyhow::bail!("生图响应中未找到图片数据（url/content/b64_json）");
    }

    fn supports_reference(&self) -> bool {
        self.supports_reference
    }
}

/// 根据 Config 中的 tasks.image 选择构造 ImageProvider。
/// nano-banana 默认 = 官方 Gemini 图像 API；第三方服务走 Other 自定义（openai-compatible）。
pub fn resolve_image(cfg: &Config) -> anyhow::Result<Box<dyn ImageProvider>> {
    let selection = cfg
        .tasks
        .image
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("未配置生图 provider，请先运行 `media-factory config`"))?;

    let provider = cfg
        .providers
        .get(&selection.provider)
        .ok_or_else(|| anyhow::anyhow!("生图 provider 不存在: {}", selection.provider))?;

    match provider {
        ProviderConfig::Custom {
            base_url,
            api_key,
            model,
        } => Ok(Box::new(OpenAiImages::new(
            base_url.clone(),
            api_key.clone(),
            model.clone(),
            false,
            serde_json::Map::new(),
        ))),
        ProviderConfig::Builtin {
            kind, api_key, extra, ..
        } => match kind {
            BuiltinKind::NanoBanana => {
                let model = extra
                    .get("model")
                    .cloned()
                    .unwrap_or_else(|| "gemini-3-pro-image-preview".into());
                Ok(Box::new(GeminiImage::new(api_key.clone(), model)))
            }
            BuiltinKind::OpenAiImage => Ok(Box::new(OpenAiImages::new(
                "https://api.openai.com/v1".into(),
                api_key.clone(),
                "gpt-image-1".into(),
                false,
                serde_json::Map::new(),
            ))),
            BuiltinKind::DoubaoSeedream => {
                anyhow::bail!("doubao-seedream（豆包生图）暂未实现，请换用其他生图 provider")
            }
            other => anyhow::bail!("provider {:?} 不支持生图任务", other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_BYTES: &[u8] = b"PNGDATA";

    fn write_temp_png(dir: &std::path::Path) -> PathBuf {
        let p = dir.join("ref.png");
        std::fs::write(&p, PNG_BYTES).unwrap();
        p
    }

    fn b64() -> String {
        base64::engine::general_purpose::STANDARD.encode(PNG_BYTES)
    }

    fn image_response() -> serde_json::Value {
        serde_json::json!({
            "candidates": [{"content": {"parts": [
                {"text": "ok"},
                {"inlineData": {"mimeType": "image/png", "data": b64()}}
            ]}}]
        })
    }

    #[tokio::test]
    async fn gemini_with_reference_image() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/models/gemini-3-pro-image-preview:generateContent"))
            .and(wiremock::matchers::header("x-goog-api-key", "key"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(image_response()))
            .mount(&server)
            .await;

        let p = GeminiImage::new("key".into(), "gemini-3-pro-image-preview".into())
            .with_base_url(server.uri());
        let dir = tempfile::tempdir().unwrap();
        let bytes = p
            .generate(&ImageRequest {
                prompt: "一只猫".into(),
                reference_images: vec![write_temp_png(dir.path())],
                size: ImageSize::Square,
            })
            .await
            .unwrap();
        assert_eq!(bytes, PNG_BYTES);

        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert!(body["contents"][0]["parts"][1]["inline_data"]["data"]
            .as_str()
            .is_some());
    }

    #[tokio::test]
    async fn gemini_without_reference_omits_inline_data() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(image_response()))
            .mount(&server)
            .await;

        let p = GeminiImage::new("key".into(), "gemini-3-pro-image-preview".into())
            .with_base_url(server.uri());
        let bytes = p
            .generate(&ImageRequest {
                prompt: "一只猫".into(),
                reference_images: vec![],
                size: ImageSize::Square,
            })
            .await
            .unwrap();
        assert_eq!(bytes, PNG_BYTES);

        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["contents"][0]["parts"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn openai_images_custom_roundtrip() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/images/generations"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [{"content": b64()}]
                })),
            )
            .mount(&server)
            .await;

        let p = OpenAiImages::new(
            server.uri(),
            "key".into(),
            "google/nano-banana-pro".into(),
            false,
            serde_json::Map::new(),
        );
        let bytes = p
            .generate(&ImageRequest {
                prompt: "一只猫".into(),
                reference_images: vec![],
                size: ImageSize::Square,
            })
            .await
            .unwrap();
        assert_eq!(bytes, PNG_BYTES);
    }

    #[test]
    fn resolve_image_defaults_to_official_gemini() {
        let mut cfg = Config::default();
        cfg.tasks.image = Some(crate::config::TaskSelection {
            provider: "nano-banana".into(),
        });
        cfg.providers.insert(
            "nano-banana".into(),
            ProviderConfig::Builtin {
                kind: BuiltinKind::NanoBanana,
                api_key: "k".into(),
                extra: Default::default(),
            },
        );
        let p = resolve_image(&cfg).unwrap();
        assert!(p.supports_reference());
    }
}
