use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::llm::LlmAgent;

/// pi agent 的 RPC 子进程客户端。
///
/// 一次 `complete()` 起一个一次性 `pi --mode rpc --no-session` 子进程：
/// 无状态、失败即进程退出、实现简单。协议为 JSONL over stdio（见 pi docs/rpc.md）。
pub struct PiRpcAgent {
    binary: std::path::PathBuf, // 默认 "pi"
    model: Option<String>,
}

impl PiRpcAgent {
    pub fn new(model: Option<String>) -> anyhow::Result<Self> {
        Self::with_binary("pi".into(), model)
    }

    pub fn with_binary(binary: std::path::PathBuf, model: Option<String>) -> anyhow::Result<Self> {
        Ok(Self { binary, model })
    }
}

/// 一次性 RPC 命令：spawn pi，发一条命令，读回对应 response（含 data 字段）。
/// 用于向导里的 get_available_models 等。
pub async fn rpc_once(
    binary: &std::path::Path,
    model: Option<&str>,
    command: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut cmd = Command::new(binary);
    cmd.args(["--mode", "rpc", "--no-session"]);
    if let Some(m) = model {
        cmd.arg("--model").arg(m);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("无法启动 pi（{}）: {e}", binary.display()))?;

    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all((command.to_string() + "\n").as_bytes())
        .await?;
    stdin.flush().await?;
    drop(stdin);

    let stderr = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut reader = tokio::io::BufReader::new(stderr);
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    });

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let mut result: Option<serde_json::Value> = None;
    let mut rpc_error: Option<String> = None;

    while let Some(line) = lines.next_line().await? {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        if v["type"] == "response" {
            if v["success"] == serde_json::Value::Bool(false) {
                rpc_error = Some(v["error"].as_str().unwrap_or("unknown error").to_string());
            } else {
                result = Some(v["data"].clone());
            }
            break;
        }
    }

    let status = child.wait().await?;
    let stderr_text = stderr_task.await.unwrap_or_default();

    if let Some(e) = rpc_error {
        anyhow::bail!("pi RPC 错误: {e}");
    }
    if !status.success() {
        let tail: String = stderr_text.chars().rev().take(2000).collect::<String>().chars().rev().collect();
        anyhow::bail!("pi 进程异常退出（{}）: {}", status, tail);
    }
    Ok(result.unwrap_or(serde_json::Value::Null))
}

#[async_trait::async_trait]
impl LlmAgent for PiRpcAgent {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        let mut cmd = Command::new(&self.binary);
        cmd.args(["--mode", "rpc", "--no-session"]);
        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "无法启动 pi（{}），请确认已安装 pi 并加入 PATH: {e}",
                self.binary.display()
            )
        })?;

        // 发送 prompt 命令
        let mut stdin = child.stdin.take().unwrap();
        let payload = serde_json::json!({"id": "req-1", "type": "prompt", "message": prompt});
        stdin
            .write_all((payload.to_string() + "\n").as_bytes())
            .await?;
        stdin.flush().await?;
        drop(stdin); // 关闭 stdin，通知 pi 无后续命令

        // 后台收集 stderr，避免 wait 后读阻塞
        let stderr = child.stderr.take().unwrap();
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut reader = tokio::io::BufReader::new(stderr);
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        });

        // 读 stdout 事件
        let stdout = child.stdout.take().unwrap();
        let mut lines = BufReader::new(stdout).lines();

        let mut out = String::new();
        let mut settled = false;
        let mut rpc_error: Option<String> = None;

        while let Some(line) = lines.next_line().await? {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)?;
            match v["type"].as_str() {
                Some("response") if v["success"] == serde_json::Value::Bool(false) => {
                    rpc_error = Some(
                        v["error"]
                            .as_str()
                            .unwrap_or("unknown error")
                            .to_string(),
                    );
                }
                Some("message_update")
                    if v["assistantMessageEvent"]["type"] == "text_delta" =>
                {
                    if let Some(d) = v["assistantMessageEvent"]["delta"].as_str() {
                        out.push_str(d);
                    }
                }
                Some("agent_settled") => {
                    settled = true;
                    break;
                }
                _ => {}
            }
        }

        let status = child.wait().await?;
        let stderr_text = stderr_task.await.unwrap_or_default();

        if let Some(e) = rpc_error {
            anyhow::bail!("pi RPC 错误: {e}");
        }
        if !status.success() {
            let tail: String = stderr_text.chars().rev().take(2000).collect::<String>().chars().rev().collect();
            anyhow::bail!("pi 进程异常退出（{}）: {}", status, tail);
        }
        if !settled {
            anyhow::bail!("pi 未返回 agent_settled 事件，会话异常结束");
        }
        Ok(out)
    }
}
