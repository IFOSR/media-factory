#[async_trait::async_trait]
pub trait LlmAgent: Send + Sync {
    /// 单轮无状态问答：发送 prompt，返回最终 assistant 文本
    async fn complete(&self, prompt: &str) -> anyhow::Result<String>;
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
}
