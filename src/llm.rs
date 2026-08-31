use crate::config::{BuiltinKind, Config, ProviderConfig};

#[async_trait::async_trait]
pub trait LlmAgent: Send + Sync {
    /// 单轮无状态问答：发送 prompt，返回最终 assistant 文本
    async fn complete(&self, prompt: &str) -> anyhow::Result<String>;
}

/// OpenAI 兼容 /chat/completions 语言模型（Deepseek / OpenAI / Gemini 等 openai-compatible 自定义）
pub struct OpenAiCompatLlm {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiCompatLlm {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl LlmAgent for OpenAiCompatLlm {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
        });
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "LLM API 返回错误 {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
        let v: serde_json::Value = resp.json().await?;
        Ok(v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

/// 根据配置解析语言模型 agent：pi（内置）或 openai-compatible（自定义）
pub fn resolve_llm(cfg: &Config) -> anyhow::Result<Box<dyn LlmAgent>> {
    let key = cfg
        .tasks
        .llm
        .as_ref()
        .map(|l| l.provider_key().to_string())
        .unwrap_or_else(|| "pi".into());
    match cfg.providers.get(&key) {
        Some(ProviderConfig::Custom { base_url, api_key, model }) => {
            Ok(Box::new(OpenAiCompatLlm::new(base_url.clone(), api_key.clone(), model.clone())))
        }
        Some(ProviderConfig::Builtin { kind: BuiltinKind::Pi, extra, .. }) => {
            let model = extra.get("model").cloned();
            Ok(Box::new(crate::pi_rpc::PiRpcAgent::new(model)?))
        }
        _ => Ok(Box::new(crate::pi_rpc::PiRpcAgent::new(None)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pi_rpc::PiRpcAgent;

    /// 假 pi：读 stdin 行，prompt 命令 → 回 response + text_delta 事件 + agent_settled
    fn fake_pi(dir: &std::path::Path) -> std::path::PathBuf {
        let script = r#"#!/bin/bash
while IFS= read -r line; do
  case "$line" in
    *'"prompt"'*)
      echo '{"id":"req-1","type":"response","command":"prompt","success":true}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"改写后的"}}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"爆款文案"}}'
      echo '{"type":"agent_settled"}'
      ;;
  esac
done
"#;
        let p = dir.join("pi");
        std::fs::write(&p, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[tokio::test]
    async fn pi_rpc_complete_concatenates_text_deltas() {
        let dir = tempfile::tempdir().unwrap();
        let agent = PiRpcAgent::with_binary(fake_pi(dir.path()), None).unwrap();
        let out = agent.complete("改写这段话").await.unwrap();
        assert_eq!(out, "改写后的爆款文案");
    }

    #[tokio::test]
    async fn pi_rpc_errors_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("pi");
        std::fs::write(&bad, "#!/bin/bash\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let agent = PiRpcAgent::with_binary(bad, None).unwrap();
        assert!(agent.complete("x").await.is_err());
    }

    #[tokio::test]
    async fn openai_compat_llm_complete() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "回复内容"}}]
            })))
            .mount(&server)
            .await;
        let llm = OpenAiCompatLlm::new(server.uri(), "k".into(), "deepseek-chat".into());
        let out = llm.complete("你好").await.unwrap();
        assert_eq!(out, "回复内容");
    }
}
