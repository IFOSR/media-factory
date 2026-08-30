use std::path::PathBuf;

use base64::Engine as _;

use crate::config::{BuiltinKind, Config, ProviderConfig};

pub struct ImageRequest {
    pub prompt: String,
    pub reference_image: Option<PathBuf>,
}

#[async_trait::async_trait]
pub trait ImageProvider: Send + Sync {
    /// 生成图片，返回 PNG 字节
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>>;
    fn supports_reference(&self) -> bool {
        false
    }
}

/// OpenAI 兼容 /images/generations 实现（nano-banana / openai-image / 自定义共用）
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
        anyhow::ensure!(resp.status().is_success(), "下载生成图片失败: {}", resp.status());
        Ok(resp.bytes().await?.to_vec())
    }
}

#[async_trait::async_trait]
impl ImageProvider for OpenAiImages {
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>> {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), self.model.clone().into());
        body.insert("prompt".into(), req.prompt.clone().into());
        body.insert("size".into(), "1024x1024".into());
        for (k, v) in &self.extra_body {
            body.insert(k.clone(), v.clone());
        }

        if self.supports_reference {
            if let Some(ref_path) = &req.reference_image {
                let bytes = std::fs::read(ref_path)?;
                let data_url = format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                );
                body.insert("image".into(), data_url.into());
            }
        } else if req.reference_image.is_some() {
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
        ProviderConfig::Builtin { kind, api_key, .. } => match kind {
            BuiltinKind::NanoBanana => {
                let mut extra = serde_json::Map::new();
                extra.insert("output_format".into(), "png".into());
                extra.insert("output_type".into(), "url".into());
                extra.insert("number_results".into(), 1.into());
                Ok(Box::new(OpenAiImages::new(
                    "https://mg.aid.pub/api/v1".into(),
                    api_key.clone(),
                    "google/nano-banana-pro".into(),
                    true,
                    extra,
                )))
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
            other => anyhow::bail!(
                "provider {:?} 不支持生图任务",
                other
            ),
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

    #[tokio::test]
    async fn nano_banana_with_reference_image() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/images/generations"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "model": "google/nano-banana-pro"
            })))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [{"content": base64::engine::general_purpose::STANDARD.encode(PNG_BYTES)}]
                })),
            )
            .mount(&server)
            .await;

        let p = OpenAiImages::new(
            server.uri(),
            "key".into(),
            "google/nano-banana-pro".into(),
            true,
            serde_json::Map::new(),
        );

        let dir = tempfile::tempdir().unwrap();
        let bytes = p
            .generate(&ImageRequest {
                prompt: "一只猫".into(),
                reference_image: Some(write_temp_png(dir.path())),
            })
            .await
            .unwrap();
        assert_eq!(bytes, PNG_BYTES);
    }

    #[tokio::test]
    async fn nano_banana_without_reference_omits_image_field() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/images/generations"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [{"content": base64::engine::general_purpose::STANDARD.encode(PNG_BYTES)}]
                })),
            )
            .mount(&server)
            .await;

        let p = OpenAiImages::new(
            server.uri(),
            "key".into(),
            "google/nano-banana-pro".into(),
            true,
            serde_json::Map::new(),
        );
        let bytes = p
            .generate(&ImageRequest {
                prompt: "一只猫".into(),
                reference_image: None,
            })
            .await
            .unwrap();
        assert_eq!(bytes, PNG_BYTES);

        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert!(body.get("image").is_none(), "无参考图时不应有 image 字段");
    }

    #[test]
    fn resolve_image_picks_nano_banana() {
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
