# Media Factory Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** 构建一个 Rust CLI 工具，输入参考文案后自动完成「改写 → 生图 → 播客 → 合成视频」四步自媒体内容生产流水线。

**Architecture:** 方案 A —— 四个独立子命令（`rewrite` / `image` / `podcast` / `video`）+ `run` 串联全部 + `config` 交互式配置向导。每步产物落盘到 `output/<任务id>/`，可单步重跑。**所有语言模型能力（改写、图像 prompt 提炼、播客脚本生成）通过 pi agent RPC 子进程（`pi --mode rpc --no-session`，JSONL over stdio）执行**，模型/provider/认证全部由 pi 管理，media-factory 只存一个模型字符串。生图与 TTS 走专用 provider 直连 API（内置预设 + Other 自定义）。

**Tech Stack:** Rust, clap (derive), dialoguer, serde + serde_yaml, serde_json, reqwest (rustls), tokio, wiremock (测试), pi CLI (RPC 子进程), ffmpeg (subprocess)

**设计文档：** `docs/plans/2026-08-30-media-factory-design.md`

---

## 全局约定

- 产物目录：`output/<任务id>/`，任务 id 默认 `chrono::Local::now().format("%Y%m%d-%H%M%S")`，也可用 `--id` 指定（用于续跑/重跑）
- 所有子命令共享参数：`--id <任务id>`（缺省时取 `output/` 下最新目录；`rewrite` 缺省时新建）
- 前置检查：`pi` 与 `ffmpeg` 可执行文件存在性（`which`）；缺失时报错提示安装
- LLM 任务统一走 `LlmAgent` trait（生产实现 = pi RPC 子进程；测试实现 = mock），pi 的瞬时错误重试由 pi auto-retry 负责
- 生图 / TTS 的直连 API 调用：指数退避（1s/2s/4s）最多 3 次，封装在 `src/retry.rs`

### Cargo.toml 依赖（全任务共用）

```toml
[package]
name = "media-factory"
version = "0.1.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
dialoguer = "0.11"
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1"
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.24", features = ["rustls-tls-native-roots"] }  # 火山播客 WebSocket API
flate2 = "1"  # 播客协议 gzip 压缩
uuid = { version = "1", features = ["v4"] }  # X-Api-Request-Id
async-trait = "0.1"
anyhow = "1"
thiserror = "1"
base64 = "0.22"
chrono = "0.4"
dirs = "5"

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
```

---

## Task 1: Cargo 项目脚手架 + CLI 骨架

**Files:**
- Create: `Cargo.toml`（见上）
- Create: `src/main.rs`

**Step 1: 初始化项目**

```bash
cargo init --name media-factory
```

**Step 2: 写 CLI 骨架**（`src/main.rs`，全部子命令先打印 "not implemented"）

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "media-factory", about = "自媒体内容工厂：改写 → 生图 → 播客 → 视频")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 交互式配置向导
    Config,
    /// 步骤 1：改写参考文案
    Rewrite {
        /// 参考文案文件路径（缺省读 stdin）
        input: Option<String>,
        /// 任务 id（缺省新建）
        #[arg(long)] id: Option<String>,
    },
    /// 步骤 2：基于改写文案生成配图
    Image {
        #[arg(long)] id: Option<String>,
        /// 可选参考图
        #[arg(long)] r#ref: Option<String>,
    },
    /// 步骤 3：基于改写文案生成播客
    Podcast {
        #[arg(long)] id: Option<String>,
    },
    /// 步骤 4：图片 + 播客合成视频
    Video {
        #[arg(long)] id: Option<String>,
    },
    /// 串联执行全部四步
    Run {
        input: Option<String>,
        #[arg(long)] id: Option<String>,
        #[arg(long)] r#ref: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        _ => println!("not implemented"),
    }
    Ok(())
}
```

**Step 3: 验证编译**

Run: `cargo build`
Expected: 编译通过

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: cargo scaffold with clap CLI skeleton"
```

---

## Task 2: 配置模型 + YAML 读写（TDD）

**Files:**
- Create: `src/config.rs`
- Test: `src/config.rs` 内 `#[cfg(test)] mod tests`

**Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let mut cfg = Config::default();
        cfg.tasks.llm = Some(LlmSelection { model: "google/gemini-2.5-pro".into() });
        cfg.tasks.image = Some(TaskSelection { provider: "nano-banana".into() });
        cfg.providers.insert("nano-banana".into(), ProviderConfig::Builtin {
            kind: BuiltinKind::NanoBanana, api_key: "k123".into(), extra: Default::default(),
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.tasks.llm.unwrap().model, "google/gemini-2.5-pro");
        match &loaded.providers["nano-banana"] {
            ProviderConfig::Builtin { api_key, .. } => assert_eq!(api_key, "k123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn custom_provider_roundtrip() {
        let mut cfg = Config::default();
        cfg.providers.insert("my-img".into(), ProviderConfig::Custom {
            base_url: "https://api.example.com/v1".into(),
            api_key: "sk-x".into(),
            model: "my-image-model".into(),
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        match &loaded.providers["my-img"] {
            ProviderConfig::Custom { base_url, model, .. } => {
                assert_eq!(base_url, "https://api.example.com/v1");
                assert_eq!(model, "my-image-model");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn volc_tts_extra_roundtrip() {
        let mut extra = std::collections::HashMap::new();
        extra.insert("appid".to_string(), "123".to_string());
        extra.insert("cluster".to_string(), "volcano_tts".to_string());
        let mut cfg = Config::default();
        cfg.providers.insert("volc-tts".into(), ProviderConfig::Builtin {
            kind: BuiltinKind::VolcTts, api_key: "tok".into(), extra,
        });
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
```

**Step 2: 运行确认失败**

Run: `cargo test config`
Expected: 编译错误（类型未定义）

**Step 3: 实现 `src/config.rs`**

```rust
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
/// 这里只为生图 / TTS 任务配置 provider。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "builtin")]
    Builtin {
        kind: BuiltinKind,
        api_key: String,
        #[serde(default)]
        extra: HashMap<String, String>, // volc-tts 需要 appid/cluster 等
    },
    #[serde(rename = "openai-compatible")]
    Custom { base_url: String, api_key: String, model: String },
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
        use BuiltinKind::*; use MediaTaskKind::*;
        matches!(
            (self, task),
            (NanoBanana, Image) | (OpenAiImage, Image) | (DoubaoSeedream, Image)
            | (VolcPodcast, Podcast) | (GeminiTts, Podcast) | (OpenAiTts, Podcast) | (VolcTts, Podcast)
        )
    }
    /// 是否为端到端播客 provider（走播客 API，不需脚本/拼接 fallback 路径）
    pub fn is_podcast_api(&self) -> bool { matches!(self, BuiltinKind::VolcPodcast) }
}

/// 需要直连 API 的媒体任务（语言模型任务不走这里）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTaskKind { Image, Podcast }

impl Config {
    pub fn path() -> PathBuf {
        dirs::home_dir().unwrap().join(".media-factory").join("config.yaml")
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        Ok(serde_yaml::from_str(&std::fs::read_to_string(path)?)?)
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
        std::fs::write(path, serde_yaml::to_string(self)?)?;
        Ok(())
    }
}
```

**Step 4: 运行测试确认通过**

Run: `cargo test config`
Expected: 3 passed

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: config model (pi-managed llm selection + media providers)"
```

---

## Task 3: pi RPC 客户端（`PiRpcAgent`）+ `LlmAgent` 抽象

**Files:**
- Create: `src/llm.rs`（trait）
- Create: `src/pi_rpc.rs`（生产实现）
- Test: 用临时目录里的 shell script 假 `pi` 可执行文件做协议测试

**Step 1: 写失败测试**

```rust
// src/llm.rs
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
        #[cfg(unix)] {
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
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let agent = PiRpcAgent::with_binary(bad, None).unwrap();
        assert!(agent.complete("x").await.is_err());
    }
}
```

**Step 2: 运行确认失败**

Run: `cargo test llm`
Expected: 编译错误

**Step 3: 实现 `src/pi_rpc.rs`**

设计要点：
- **一次 `complete()` = 一个一次性子进程**：`pi --mode rpc --no-session [--model <字符串>]`，天然无状态、无跨调用污染，失败即进程退出，实现最简单
- 写 stdin：`{"id":"req-1","type":"prompt","message":<prompt>}`，然后关闭 stdin 等待
- 逐行读 stdout（按 `\n` 分割，strip `\r`）：收集 `message_update` 中 `assistantMessageEvent.type == "text_delta"` 的 `delta`；直到 `agent_settled` 或进程退出
- 若 prompt response `success: false` → 报错（含 error 字段）
- 若进程退出码非 0 / 未收到 agent_settled → 报错（附 stderr 尾部）
- 模型字符串来自 `Config.tasks.llm`，`None` 时不传 `--model`（用 pi 默认）

```rust
pub struct PiRpcAgent {
    binary: std::path::PathBuf,   // 默认 "pi"
    model: Option<String>,
}

impl PiRpcAgent {
    pub fn new(model: Option<String>) -> anyhow::Result<Self> { Self::with_binary("pi".into(), model) }
    pub fn with_binary(binary: std::path::PathBuf, model: Option<String>) -> anyhow::Result<Self> {
        Ok(Self { binary, model })
    }
}

#[async_trait::async_trait]
impl crate::llm::LlmAgent for PiRpcAgent {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        // spawn → stdin 写 prompt 行 → 逐行读 stdout 收集 text_delta → 等 agent_settled
        // 错误路径：response.success==false / 进程非零退出 / EOF 前无 agent_settled
        todo!()
    }
}
```

**Step 4: 运行测试确认通过**

Run: `cargo test llm`
Expected: 2 passed

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: pi RPC client implementing LlmAgent"
```

---

## Task 4: 交互式配置向导（`media-factory config`）

**Files:**
- Create: `src/wizard.rs`
- Modify: `src/main.rs`

**流程设计：**

```
主菜单（循环）：
  1. 配置「语言模型」（pi agent）
  2. 配置「生图」provider
  3. 配置「播客」provider
  4. 新增自定义 provider（Other，仅限生图/播客）
  5. 保存并退出
  0. 不保存退出（确认提示）
```

- **配置语言模型**：spawn `pi --mode rpc --no-session` → 发 `{"type":"get_available_models"}` → 解析 `models[]` → `dialoguer::Select` 列出 `provider/model`（含名称）供选择 → 存 `tasks.llm.model`；列表为空时提示"先在 pi 中配置认证（`pi auth login`）"；提供第一项"使用 pi 默认模型"（存 None）
- **配置生图/播客 provider**：列出支持该任务的内置 provider（已配 key 标 ✓，volc-podcast 标注「推荐」）+ 所有自定义 provider → 选内置且无 key 时 `Password` 录入 access token（volc-podcast / volc-tts 追加 `Input` appid 等到 extra）→ 绑定任务
- **新增自定义（Other）**：`Input` 名称（唯一、非空）→ `Input` BaseURL（http(s) 校验）→ `Password` API Key → `Input` 模型名 → `MultiSelect` 绑定到 生图/播客TTS

**Step 1: 写测试**（纯逻辑部分）

```rust
// wizard.rs
pub fn builtin_for_task(task: MediaTaskKind) -> Vec<BuiltinKind> { /* 过滤 supports */ }

pub fn validate_custom_name(existing: &std::collections::HashMap<String, ProviderConfig>, name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() { anyhow::bail!("名称不能为空"); }
    if existing.contains_key(name) { anyhow::bail!("provider 名称已存在: {name}"); }
    Ok(())
}

pub fn validate_base_url(url: &str) -> anyhow::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        anyhow::bail!("BaseURL 必须以 http:// 或 https:// 开头");
    }
    Ok(())
}

/// 把 pi get_available_models 响应解析为 "provider/model" 列表
pub fn parse_available_models(resp: &serde_json::Value) -> Vec<String> {
    resp["data"]["models"].as_array().map(|ms| ms.iter()
        .filter_map(|m| Some(format!("{}/{}", m["provider"].as_str()?, m["id"].as_str()?)))
        .collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_lists_match_tasks() {
        assert_eq!(builtin_for_task(MediaTaskKind::Image).len(), 3);
        assert_eq!(builtin_for_task(MediaTaskKind::Podcast).len(), 4);
        assert!(builtin_for_task(MediaTaskKind::Podcast).contains(&BuiltinKind::VolcPodcast));
        assert!(!builtin_for_task(MediaTaskKind::Image).contains(&BuiltinKind::VolcPodcast));
    }
    #[test]
    fn custom_name_and_url_validation() { /* 同前 */ }
    #[test]
    fn parses_pi_model_list() {
        let v = serde_json::json!({"type":"response","success":true,"data":{"models":[
            {"provider":"google","id":"gemini-2.5-pro","name":"Gemini 2.5 Pro"},
            {"provider":"openai","id":"gpt-5","name":"GPT-5"}]}});
        assert_eq!(parse_available_models(&v), vec!["google/gemini-2.5-pro", "openai/gpt-5"]);
    }
}
```

**Step 2: 运行确认失败**

Run: `cargo test wizard`
Expected: 编译错误

**Step 3: 实现向导主体**（dialoguer 交互 + 上述纯函数；语言模型选择通过 `PiRpcAgent` 同款 spawn 逻辑发 `get_available_models`）

**Step 4: 接线 main.rs**：`Commands::Config => wizard::run()?`

**Step 5: 测试 + 手动验证**

Run: `cargo test wizard` → PASS；`cargo run -- config` 手动走流程，检查 `~/.media-factory/config.yaml`

**Step 6: Commit**

```bash
git add -A && git commit -m "feat: interactive config wizard (pi model picker + media providers + Other)"
```

---

## Task 5: `rewrite` 命令 + 爆款 prompt 模板

**Files:**
- Create: `prompts/rewrite.txt`
- Create: `src/cmd/rewrite.rs`
- Test: 用 mock `LlmAgent` 测试命令流程（产物落盘、模板变量替换）

**Step 1: 写失败测试**

```rust
struct MockLlm(String);
#[async_trait::async_trait]
impl LlmAgent for MockLlm {
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        assert!(prompt.contains("原始参考文案内容")); // 模板已注入原文
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn rewrite_writes_input_and_output() {
    let dir = tempfile::tempdir().unwrap();
    let id = rewrite::run_with(dir.path(), None, Some("task1".into()), &MockLlm("爆款文案".into())).await.unwrap();
    assert_eq!(id, "task1");
    assert_eq!(std::fs::read_to_string(dir.path().join("task1/rewritten.md")).unwrap(), "爆款文案");
    assert!(dir.path().join("task1/input.md").exists());
}
```

**Step 2: 运行确认失败**

**Step 3: 实现**

- `prompts/rewrite.txt`：爆款改写模板，含 `{{SOURCE}}` 占位符；模板要点：保留核心事实、口语化、开头 3 秒钩子（悬念/反差/数字冲击）、中段短句节奏、结尾互动引导、只输出正文
- `cmd/rewrite.rs`：
  - `run_with(output_root, input: Option<&str>, id: Option<String>, llm: &dyn LlmAgent) -> anyhow::Result<String>`（返回任务 id，便于测试与 run 复用）
  - 读输入（文件/stdin）→ 建 `output/<id>/` → `input.md` 落盘 → 渲染模板 → `llm.complete()` → `rewritten.md` 落盘并打印
  - 公开入口 `run(...)` 内部加载 Config → `PiRpcAgent::new(cfg.tasks.llm.map(|l| l.model))` → 调 `run_with`

**Step 4: 测试通过 + 手动 `cargo run -- rewrite sample.md`（真实 pi 环境）**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: rewrite command via pi agent with viral prompt template"
```

---

## Task 6: `image` 命令 —— pi 提炼图像 prompt + nano-banana 生图（支持可选参考图）

**Files:**
- Create: `prompts/image_prompt.txt`
- Create: `src/provider/mod.rs`（`ImageProvider` trait + resolve）
- Create: `src/provider/nano_banana.rs`
- Create: `src/cmd/image.rs`
- Test: wiremock 测试 nano-banana 请求构造（含/不含参考图两 case）+ 命令流程测试（mock LLM + mock HTTP）

**Step 1: 写失败测试**

```rust
// nano-banana：无参考图时 contents.parts 只有 text；有参考图时含 inline_data（base64+mime）
#[tokio::test]
async fn nano_banana_with_reference_image() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(b"PNG")}}]}}]
        }))).mount(&server).await;
    let p = NanoBanana::new("key".into()).with_base_url(server.uri());
    let bytes = p.generate(&ImageRequest {
        prompt: "一只猫".into(),
        reference_image: Some(write_temp_png()), // 1x1 png
    }).await.unwrap();
    assert_eq!(bytes, b"PNG");
}
```

**Step 2: 运行确认失败**

**Step 3: 实现**

- `prompts/image_prompt.txt`：让 pi 从 `rewritten.md` 提炼核心意象，输出一段画面描述（主体、构图、风格、情绪），`{{TEXT}}` 占位
- `provider/mod.rs`：

```rust
pub struct ImageRequest { pub prompt: String, pub reference_image: Option<std::path::PathBuf> }

#[async_trait::async_trait]
pub trait ImageProvider: Send + Sync {
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>>; // PNG bytes
    fn supports_reference(&self) -> bool { false }
}

pub fn resolve_image(cfg: &Config) -> anyhow::Result<Box<dyn ImageProvider>> { todo!() } // nano-banana / openai-image / seedream / custom
```

- `nano_banana.rs`：Gemini Image generateContent（`responseModalities: ["IMAGE"]`）；参考图 → base64 `inline_data` part；`supports_reference() = true`
- `cmd/image.rs`：校验 `rewritten.md` 存在 → pi 提炼 prompt → resolve image provider → `--ref` 存在且 `!supports_reference()` 时警告降级 → `image.png` 落盘

**Step 4: 测试通过 + 手动验证**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: image command (pi prompt distillation + nano-banana, optional reference)"
```

---

## Task 7: `podcast` 命令 —— 火山播客 API（默认）+ 通用 TTS fallback

**Files:**
- Create: `src/podcast/mod.rs`（`PodcastProvider` trait + resolve）
- Create: `src/podcast/volc_podcast.rs`（播客 API，WebSocket v3 二进制协议）
- Create: `src/podcast/volc_proto.rs`（二进制帧编解码，纯逻辑可单测）
- Create: `prompts/podcast_script.txt`（fallback 路径用）
- Create: `src/tts/mod.rs` + `src/tts/{volc_tts,gemini_tts,openai_tts}.rs`（fallback TTS providers）
- Create: `src/ffmpeg.rs`
- Create: `src/cmd/podcast.rs`
- Test: 帧编解码单测 + 假 WebSocket server 测试 + 脚本解析测试 + ffmpeg concat 测试

**背景：** 火山播客大模型 API（[文档](https://www.volcengine.com/docs/6561/1668014)）：`wss://openspeech.bytedance.com/api/v3/sami/podcasttts`，headers `X-Api-App-Id` / `X-Api-Access-Key` / `X-Api-Resource-Id: volc.service_type.10050` / `X-Api-App-Key: aGjiRDfUWi`。`action=0` 文本直接生成双人播客；`action=3` 按 `nlp_texts` 合成；`only_nlp_text=true` 只出脚本；`return_audio_url=true` 返回完整 mp3 链接。音频格式选 mp3。

**Step 1: 写失败测试**

```rust
// volc_proto.rs —— 二进制帧编解码（文档 2.1 节：4 字节 header + payload）
#[test]
fn encode_full_client_request_frame() {
    let frame = encode_text_frame(b"{\"action\":0}").unwrap();
    assert_eq!(frame[0] >> 4, 0b0001);        // protocol v1
    assert_eq!(frame[0] & 0xF, 0b0001);       // header size 4x
    assert_eq!(frame[1] >> 4, 0b0001);        // message type: full client request
    assert_eq!(frame[2] >> 4, 0b0001);        // JSON serialization
    assert!(frame.len() > 4);
}

#[test]
fn decode_audio_response_frame() {
    // 构造一帧 server 音频响应，断言解码出 payload bytes
}

// 脚本解析（fallback 路径与模式 B 共用）
#[test]
fn parse_dialogue_script() {
    let script = "主持人：欢迎收听本期节目！\n嘉宾：今天的内容太炸了。\n主持人：没错。";
    let segs = parse_script(script).unwrap();
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0].role, Role::Host);
    assert_eq!(segs[1].role, Role::Guest);
}

#[test]
fn parse_rejects_unknown_role() {
    assert!(parse_script("路人：hello").is_err());
}

// 模式 B：script.md 的 主持人/嘉宾 格式 → nlp_texts（speaker 用配置的两个发音人）
#[test]
fn script_to_nlp_texts_maps_speakers() {
    let segs = parse_script("主持人：你好\n嘉宾：嗨").unwrap();
    let nlp = to_nlp_texts(&segs, "speaker_a", "speaker_b");
    assert_eq!(nlp[0]["speaker"], "speaker_a");
    assert_eq!(nlp[1]["speaker"], "speaker_b");
}
```

**Step 2: 运行确认失败**

**Step 3: 实现**

- `volc_proto.rs`：帧编解码纯函数（protocol v1 / header 4 字节 / JSON / gzip 可选）
- `volc_podcast.rs`：
  - `PodcastProvider` trait：`async fn generate(&self, req: &PodcastRequest) -> anyhow::Result<PodcastResult>`，`PodcastRequest { text, nlp_texts: Option<Vec<NlpText>>, only_nlp_text: bool }`
  - WebSocket 建连（带鉴权 headers）→ 发送 request payload → 循环读下行事件：音频帧追加 buffer / `PodcastRoundEnd` / `PodcastEnd`（含 `audio_url`，用它下载完整 mp3 或直接用拼接 buffer）→ 连接关闭返回
  - 发音人：`speaker_info.speakers` 取配置 extra 中的两个音色，缺省用播客专属默认双音色
- `cmd/podcast.rs` 流程（volc-podcast provider 时）：
  - **模式 B**：`--script` 或 `script.md` 已存在 → 若无脚本先 `action=0 + only_nlp_text` 生成 → `script.md` 落盘退出并提示人工修改后重跑；若已有脚本 → `parse_script` → `to_nlp_texts` → `action=3` 合成 → `podcast.mp3`
  - **模式 A（默认）**：`action=0` 直接合成 → `podcast.mp3`
- fallback 路径（gemini-tts / openai-tts / volc-tts）：pi 生成对话脚本（`prompts/podcast_script.txt`，严格 `主持人：...` / `嘉宾：...` 格式）→ `script.md` 落盘 → 分段 TTS（`default_voices()` 双音色，retry3）→ `ffmpeg.rs::concat_mp3` 拼接 → `podcast.mp3`

**Step 4: 测试通过 + 手动验证（真实火山 key 生成播客可播放）**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: podcast command (volc podcast API mode A/B + generic tts fallback)"
```

---

## Task 8: `video` 命令 —— 静态图 + 音频合成

**Files:**
- Modify: `src/ffmpeg.rs`
- Create: `src/cmd/video.rs`
- Test: 1×1 PNG + 1 秒 mp3 → 输出 mp4 存在且时长 ≈ 音频时长（`ffprobe`）

**Step 1: 写失败测试**

```rust
#[test]
fn make_video_duration_matches_audio() {
    // ffmpeg -f lavfi 生成 1s 测试音频与测试图；make_video；
    // ffprobe 输出时长，断言与音频时长误差 < 0.2s
}
```

**Step 2: 运行确认失败**

**Step 3: 实现**

```rust
pub fn make_video(image: &Path, audio: &Path, out: &Path) -> anyhow::Result<()> {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loop", "1", "-i"]).arg(image)
        .arg("-i").arg(audio)
        .args(["-c:v", "libx264", "-tune", "stillimage",
               "-c:a", "aac", "-b:a", "192k",
               "-pix_fmt", "yuv420p", "-shortest"])
        .arg(out).status()?;
    anyhow::ensure!(status.success(), "ffmpeg 合成失败");
    Ok(())
}
```

- `cmd/video.rs`：校验 `image.png` 与 `podcast.mp3` 存在 → `make_video` → `video.mp4`

**Step 4: 测试通过 → Commit**

```bash
git add -A && git commit -m "feat: video command composing static image and podcast audio"
```

---

## Task 9: `run` 命令 —— 流水线串联

**Files:**
- Create: `src/cmd/run.rs`
- Modify: `src/main.rs`

**Step 1: 实现** — 四个 cmd 共用同一个 `LlmAgent` 实例与任务 id 依次执行；任一步失败即停止并提示可用 `--id <id> <失败子命令>` 续跑

```rust
pub async fn run(input: Option<String>, id: Option<String>, ref_: Option<String>) -> anyhow::Result<()> {
    let cfg = Config::load(&Config::path())?;
    let llm = PiRpcAgent::new(cfg.tasks.llm.clone().map(|l| l.model))?;
    let id = rewrite::run_with(Path::new("output"), input.as_deref(), id, &llm).await?;
    image::run_with(Path::new("output"), &id, ref_, &llm, &cfg).await?;
    podcast::run_with(Path::new("output"), &id, &llm, &cfg).await?;
    video::run_with(Path::new("output"), &id).await?;
    println!("完成！产物目录: output/{id}/");
    Ok(())
}
```

**Step 2: 各 cmd 统一提供 `run_with(...) -> anyhow::Result<...>` 可注入依赖的版本 + 薄封装 `run(...)`，接线 main.rs**

**Step 3: 手动端到端验证（真实 pi + 真实或 mock 的媒体 API）**

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: run command chaining full pipeline"
```

---

## Task 10: 收尾 —— 前置检查 + README + 端到端集成测试

**Files:**
- Modify: `src/main.rs`（启动时 `which pi` / `which ffmpeg` 检查；config 命令豁免 pi 检查）
- Create: `README.md`（安装 pi 与 ffmpeg、config 向导、四步用法、`--ref`、`--id` 续跑、provider 列表、pi 侧自定义模型说明 `models.json`）
- Test: 端到端集成测试（假 `pi` script + wiremock 媒体 API + 真实 ffmpeg，`run` 跑通且四个产物存在）

**Step 1: 写集成测试** — tempdir 假 pi + wiremock + tempfile output root，全流水线跑通

**Step 2: 实现缺失部分使测试通过**

**Step 3: `cargo test` 全绿 + `cargo clippy` 无警告**

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: preflight checks, readme, end-to-end integration test"
```

---

## 备注

- `PiRpcAgent` 每次 `complete()` 起一个一次性子进程是有意为之：无状态、隔离失败、实现简单；若后续性能成为问题，再演进为长驻进程 + `new_session` 复用
- OpenAI gpt-image / 豆包 Seedream / OpenAI TTS / Gemini TTS 完全复用 nano-banana / volc_podcast 建立的 HTTP/WS 模式，作为各任务内同级小步骤完成
- 火山播客 API 的断点续传（`retry_info`）首版不实现，失败后整体重试即可；片头/片尾音乐默认关闭
- 语言模型的自定义 provider 不在 media-factory 内实现 —— 引导用户在 pi 的 `models.json` 中配置后，向导的模型列表自动可见
- 字幕、TTS 段并发合成等优化明确不做（YAGNI），后续迭代再加
