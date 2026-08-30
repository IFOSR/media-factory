# Media Factory Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use executing-plans to implement this plan task-by-task.

**Goal:** 构建一个 Rust CLI 工具，输入参考文案后自动完成「改写 → 生图 → 播客 → 合成视频」四步自媒体内容生产流水线。

**Architecture:** 方案 A —— 四个独立子命令（`rewrite` / `image` / `podcast` / `video`）+ `run` 串联全部 + `config` 交互式配置向导。每步产物落盘到 `output/<任务id>/`，可单步重跑。Provider 层按任务类型（改写 LLM / 生图 / TTS）抽象 trait，内置预设 + 用户自定义（OpenAI 兼容）。

**Tech Stack:** Rust, clap (derive), dialoguer, serde + serde_yaml, reqwest (rustls), tokio, wiremock (测试), ffmpeg (subprocess)

**设计文档：** `docs/plans/2026-08-30-media-factory-design.md`

---

## 全局约定

- 产物目录：`output/<任务id>/`，任务 id 默认 `chrono::Local::now().format("%Y%m%d-%H%M%S")`，也可用 `--id` 指定（用于续跑/重跑）
- 所有子命令共享参数：`--id <任务id>`（缺省时取 `output/` 下最新目录；`rewrite` 缺省时新建）
- ffmpeg 可用性检查：`which ffmpeg`，不存在则报错提示安装
- API 调用统一重试：指数退避（1s/2s/4s），最多 3 次，封装在 `src/retry.rs`

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
reqwest = { version = "0.12", features = ["json", "rustls-tls", "multipart"] }
tokio = { version = "1", features = ["full"] }
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

**Step 2: 写 CLI 骨架**（`src/main.rs`，先全部子命令打印 "not implemented"）

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
        cfg.tasks.rewrite = Some(TaskSelection { provider: "gemini".into() });
        cfg.providers.insert("gemini".into(), ProviderConfig::Builtin {
            kind: BuiltinKind::Gemini, api_key: "k123".into(),
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.tasks.rewrite.unwrap().provider, "gemini");
        match &loaded.providers["gemini"] {
            ProviderConfig::Builtin { api_key, .. } => assert_eq!(api_key, "k123"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn custom_provider_roundtrip() {
        let mut cfg = Config::default();
        cfg.providers.insert("my-llm".into(), ProviderConfig::Custom {
            type_: CustomType::OpenAiCompatible,
            base_url: "https://api.example.com/v1".into(),
            api_key: "sk-x".into(),
            model: "my-model".into(),
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        match &loaded.providers["my-llm"] {
            ProviderConfig::Custom { base_url, model, .. } => {
                assert_eq!(base_url, "https://api.example.com/v1");
                assert_eq!(model, "my-model");
            }
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
    pub rewrite: Option<TaskSelection>,
    pub image: Option<TaskSelection>,
    pub podcast: Option<TaskSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSelection {
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "builtin")]
    Builtin { kind: BuiltinKind, api_key: String },
    #[serde(rename = "openai-compatible")]
    Custom {
        #[serde(rename = "type")]
        type_: CustomType,
        base_url: String,
        api_key: String,
        model: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinKind {
    Gemini,        // 改写 + TTS
    OpenAi,        // 改写 + gpt-image + TTS
    Doubao,        // 改写（火山方舟）
    NanoBanana,    // 生图（Gemini Image，支持参考图）
    DoubaoSeedream,// 生图
    VolcTts,       // 播客 TTS（火山豆包语音）
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomType { OpenAiCompatible }

impl BuiltinKind {
    /// 该内置 provider 支持哪些任务类型
    pub fn supports(&self, task: TaskKind) -> bool {
        use BuiltinKind::*; use TaskKind::*;
        matches!(
            (self, task),
            (Gemini, Rewrite) | (Gemini, Podcast)
            | (OpenAi, Rewrite) | (OpenAi, Image) | (OpenAi, Podcast)
            | (Doubao, Rewrite)
            | (NanoBanana, Image)
            | (DoubaoSeedream, Image)
            | (VolcTts, Podcast)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind { Rewrite, Image, Podcast }

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
Expected: 2 passed

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: config model with yaml roundtrip"
```

---

## Task 3: 交互式配置向导（`media-factory config`）

**Files:**
- Create: `src/wizard.rs`
- Modify: `src/main.rs`

**流程设计：**

```
主菜单（循环）：
  1. 配置「改写」provider
  2. 配置「生图」provider
  3. 配置「播客」provider
  4. 新增自定义 provider（Other）
  5. 保存并退出
  0. 不保存退出（确认提示）
```

- 配置任务：列出「支持该任务类型的内置 provider（未配 key 的标注 [未配置]）」+ 所有自定义 provider → `dialoguer::Select`
  - 选内置：若未存 key → `dialoguer::Password` 输入 API Key
  - 选自定义：直接绑定
- 新增自定义（Other）：依次 `Input` 名称（校验唯一、非空）、`Input` BaseURL（校验 http(s) 开头）、`Password` API Key、`Input` 模型名 → 写入 `providers`，随后自动进入"绑定到任务"选择（可多选绑定到 rewrite/image/podcast，自定义 provider 按 openai-compatible 约定可服务全部三类任务）

**Step 1: 写测试**（纯逻辑部分拆出来可测）

```rust
// wizard.rs
pub fn builtin_for_task(task: TaskKind) -> Vec<BuiltinKind> {
    use BuiltinKind::*;
    [Gemini, OpenAi, Doubao, NanoBanana, DoubaoSeedream, VolcTts]
        .into_iter().filter(|k| k.supports(task)).collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_lists_match_tasks() {
        assert!(builtin_for_task(TaskKind::Rewrite).contains(&BuiltinKind::Gemini));
        assert!(!builtin_for_task(TaskKind::Rewrite).contains(&BuiltinKind::NanoBanana));
        assert!(builtin_for_task(TaskKind::Image).contains(&BuiltinKind::NanoBanana));
        assert_eq!(builtin_for_task(TaskKind::Podcast).len(), 3);
    }
    #[test]
    fn custom_name_validation() {
        let mut m = std::collections::HashMap::new();
        assert!(validate_custom_name(&m, "").is_err());
        assert!(validate_custom_name(&m, "x").is_ok());
        m.insert("x".into(), ProviderConfig::Builtin { kind: BuiltinKind::Gemini, api_key: "k".into() });
        assert!(validate_custom_name(&m, "x").is_err());
    }
    #[test]
    fn base_url_validation() {
        assert!(validate_base_url("https://api.x.com/v1").is_ok());
        assert!(validate_base_url("api.x.com").is_err());
    }
}
```

**Step 2: 运行确认失败**

Run: `cargo test wizard`
Expected: 编译错误

**Step 3: 实现向导主体**（dialoguer 交互 + 上述纯函数）

```rust
pub fn run() -> anyhow::Result<()> {
    let path = Config::path();
    let mut cfg = Config::load(&path)?;
    loop {
        let items = vec![
            "配置「改写」provider", "配置「生图」provider", "配置「播客」provider",
            "新增自定义 provider（Other）", "保存并退出", "不保存退出",
        ];
        match dialoguer::Select::new().with_prompt("Media Factory 配置").items(&items).interact()? {
            0 => bind_task(&mut cfg, TaskKind::Rewrite)?,
            1 => bind_task(&mut cfg, TaskKind::Image)?,
            2 => bind_task(&mut cfg, TaskKind::Podcast)?,
            3 => add_custom(&mut cfg)?,
            4 => { cfg.save(&path)?; println!("已保存到 {}", path.display()); return Ok(()); }
            _ => {
                if dialoguer::Confirm::new().with_prompt("确定放弃修改？").interact()? { return Ok(()); }
            }
        }
    }
}

fn bind_task(cfg: &mut Config, task: TaskKind) -> anyhow::Result<()> {
    // 1. builtin_for_task(task) → 名称列表（已存 key 的标 ✓）
    // 2. 追加 cfg.providers 中所有 Custom 名称
    // 3. Select → 选内置则若无 key 用 Password 录入 key，写入 providers
    // 4. 更新 cfg.tasks 对应字段
    todo!()
}

fn add_custom(cfg: &mut Config) -> anyhow::Result<()> {
    // Input 名称（validate_custom_name）→ Input BaseURL（validate_base_url）
    // → Password API Key → Input 模型名 → 插入 providers
    // → 追问是否立即绑定到任务（MultiSelect: 改写/生图/播客）
    todo!()
}
```

**Step 4: 接线 main.rs**：`Commands::Config => wizard::run()?`

**Step 5: 测试 + 手动验证**

Run: `cargo test wizard` → PASS；`cargo run -- config` 手动走一遍流程，检查 `~/.media-factory/config.yaml` 内容正确

**Step 6: Commit**

```bash
git add -A && git commit -m "feat: interactive config wizard with custom provider support"
```

---

## Task 4: Provider 抽象层 + 重试机制

**Files:**
- Create: `src/provider/mod.rs`
- Create: `src/retry.rs`

**Step 1: 写失败测试**（trait 对象注册与按任务查找）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_resolves_task_provider() { /* 构造 Config，registry.resolve(TaskKind::Rewrite, &cfg) 返回正确 provider */ }
}
```

**Step 2: 运行确认失败** → `cargo test provider` 编译错误

**Step 3: 实现 trait 与注册表**

```rust
// src/provider/mod.rs
use async_trait::async_trait; // 加入 Cargo.toml: async-trait = "0.1"

pub struct RewriteRequest { pub source_text: String, pub prompt_template: String }
pub struct ImageRequest { pub prompt: String, pub reference_image: Option<std::path::PathBuf> }
pub struct TtsRequest { pub text: String, pub voice: String }

#[async_trait]
pub trait RewriteProvider: Send + Sync {
    async fn rewrite(&self, req: &RewriteRequest) -> anyhow::Result<String>;
}
#[async_trait]
pub trait ImageProvider: Send + Sync {
    async fn generate(&self, req: &ImageRequest) -> anyhow::Result<Vec<u8>>; // PNG bytes
    fn supports_reference(&self) -> bool { false }
}
#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, req: &TtsRequest) -> anyhow::Result<Vec<u8>>; // mp3/wav bytes
    /// 可用的双角色音色对（host, guest）
    fn default_voices(&self) -> (String, String);
}

/// 根据 Config 中任务选择构造具体 provider 实例
pub fn resolve_rewrite(cfg: &Config) -> anyhow::Result<Box<dyn RewriteProvider>> { todo!() }
pub fn resolve_image(cfg: &Config) -> anyhow::Result<Box<dyn ImageProvider>> { todo!() }
pub fn resolve_tts(cfg: &Config) -> anyhow::Result<Box<dyn TtsProvider>> { todo!() }
```

```rust
// src/retry.rs
pub async fn retry3<F, Fut, T>(mut f: F) -> anyhow::Result<T>
where F: FnMut() -> Fut, Fut: std::future::Future<Output = anyhow::Result<T>> {
    let mut delay = std::time::Duration::from_secs(1);
    let mut last = None;
    for _ in 0..3 {
        match f().await { Ok(v) => return Ok(v), Err(e) => last = Some(e) }
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
    Err(last.unwrap())
}
```

**Step 4: 测试通过 → Commit**

```bash
git add -A && git commit -m "feat: provider traits, registry, retry helper"
```

---

## Task 5: `rewrite` 命令 + Gemini 改写 provider + prompt 模板

**Files:**
- Create: `src/provider/gemini.rs`
- Create: `src/provider/openai_compat.rs`（同时服务 OpenAI / 豆包 / 自定义 Other）
- Create: `prompts/rewrite.txt`（爆款改写模板，用户可改）
- Create: `src/cmd/rewrite.rs`
- Test: wiremock 测试两个 provider 的请求构造

**Step 1: 写失败测试**（wiremock 拦截 Gemini generateContent 与 OpenAI chat/completions，断言请求体含原文与模板、解析出文本）

**Step 2: 运行确认失败**

**Step 3: 实现**

- `gemini.rs`：POST `{base}/v1beta/models/gemini-2.0-flash:generateContent`，header `x-goog-api-key`
- `openai_compat.rs`：POST `{base_url}/chat/completions`，`Authorization: Bearer <key>`，body `{model, messages}` —— OpenAI / 豆包（`https://ark.cn-beijing.volces.com/api/v3`）/ 自定义 都走它
- `prompts/rewrite.txt` 模板要点：保留核心信息与事实、口语化、开头 3 秒钩子（悬念/反差/数字冲击）、中段节奏短句、结尾互动引导；输出仅正文
- `cmd/rewrite.rs`：读输入（文件/stdin）→ 建任务目录 → `input.md` 落盘 → resolve rewrite provider → retry3 调用 → `rewritten.md` 落盘并打印

**Step 4: 测试通过 + 手动 `cargo run -- rewrite sample.md`（需真实 key 或 mock）**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: rewrite command with gemini and openai-compatible providers"
```

---

## Task 6: `image` 命令 + nano-banana 生图 provider（支持可选参考图）

**Files:**
- Create: `src/provider/nano_banana.rs`
- Create: `src/cmd/image.rs`
- Test: wiremock 测试（含参考图 base64 内联与不包含两种 case）

**Step 1: 写失败测试** — 断言：无参考图时 parts 只有 text；有参考图时 parts 含 `inline_data`（base64 + mime）

**Step 2: 运行确认失败**

**Step 3: 实现**

- 先用 rewrite provider（LLM）从 `rewritten.md` 提炼核心意象 → 图像 prompt（prompt 模板 `prompts/image_prompt.txt`：要求输出一段画面描述，突出核心意思、构图与风格）
- `nano_banana.rs`：Gemini Image 生成接口（`gemini-3-pro-image` 系列，generateContent + `responseModalities: ["IMAGE"]`）；参考图读文件 → base64 → `inline_data` part；解析响应 base64 → PNG bytes
- `cmd/image.rs`：校验 `rewritten.md` 存在 → 生成 prompt → resolve image provider → `--ref` 存在时若 `!supports_reference()` 打印警告降级 → `image.png` 落盘

**Step 4: 测试通过 + 手动验证**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: image command with nano-banana provider and optional reference image"
```

---

## Task 7: `podcast` 命令 —— 对话脚本生成 + 双音色 TTS + ffmpeg 拼接

**Files:**
- Create: `prompts/podcast_script.txt`
- Create: `src/provider/gemini_tts.rs`（Gemini TTS）
- Create: `src/provider/volc_tts.rs`（火山豆包语音）
- Create: `src/ffmpeg.rs`
- Create: `src/cmd/podcast.rs`
- Test: 脚本解析测试 + wiremock TTS 测试 + ffmpeg concat 用 2 个 1 秒静音 mp3 验证

**Step 1: 写失败测试**

```rust
#[test]
fn parse_dialogue_script() {
    let script = "主持人：欢迎收听本期节目！\n嘉宾：今天的内容太炸了。\n主持人：没错。";
    let segs = parse_script(script).unwrap();
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0].role, Role::Host);
    assert_eq!(segs[1].role, Role::Guest);
    assert_eq!(segs[2].text, "没错。");
}
#[test]
fn parse_rejects_unknown_role() {
    assert!(parse_script("路人：hello").is_err());
}
```

**Step 2: 运行确认失败**

**Step 3: 实现**

- `prompts/podcast_script.txt`：把文案改写成「主持人/嘉宾」双人对话播客脚本，口语化、有追问有 reaction，严格按 `主持人：...` / `嘉宾：...` 每行一句的格式输出
- `parse_script`：逐行解析，角色映射 Host/Guest，未知角色报错
- `cmd/podcast.rs` 流程：
  1. 若 `script.md` 已存在（人工改过）→ 直接用；否则 LLM 生成 → 落盘
  2. `parse_script` → 每段调 TTS（Host/Guest 用 `default_voices()` 两个音色）→ 段文件 `seg-000.mp3`…（retry3）
  3. `ffmpeg.rs::concat_mp3(segs, podcast.mp3)`：`ffmpeg -f concat -safe 0 -i list.txt -c copy out.mp3`
- Gemini TTS / Volc TTS 各自实现 `TtsProvider`；volc 走 HTTP 接口（appid/token/cluster 从 provider 配置扩展字段读取——`ProviderConfig::Builtin` 增加 `extra: HashMap<String,String>` 可选字段，向导中 volc 额外询问 appid/cluster）

**Step 4: 测试通过 + 手动验证播客可播放**

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: podcast command with dialogue script, dual-voice tts, ffmpeg concat"
```

---

## Task 8: `video` 命令 —— 静态图 + 音频合成

**Files:**
- Modify: `src/ffmpeg.rs`
- Create: `src/cmd/video.rs`
- Test: 用 1×1 PNG + 1 秒 mp3 验证输出 mp4 存在且时长 ≈ 音频时长（`ffprobe`）

**Step 1: 写失败测试**

```rust
#[test]
fn make_video_duration_matches_audio() {
    // 生成测试图片与音频（ffmpeg -f lavfi），调用 make_video，
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

**Step 1: 实现** — 依次调用四个 cmd 的公开函数（同一个任务 id 贯穿）；任一步失败即停止并提示可用 `--id <id> <失败子命令>` 续跑

```rust
pub async fn run(input: Option<String>, id: Option<String>, ref_: Option<String>) -> anyhow::Result<()> {
    let id = rewrite::run(input, id).await?;   // 返回任务 id
    image::run(Some(id.clone()), ref_).await?;
    podcast::run(Some(id.clone())).await?;
    video::run(Some(id.clone())).await?;
    println!("完成！产物目录: output/{id}/");
    Ok(())
}
```

**Step 2: 各 cmd 函数签名统一为 `pub async fn run(...) -> anyhow::Result<...>` 并接线 main.rs**

**Step 3: 手动端到端验证（mock 或真实 key）**

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: run command chaining full pipeline"
```

---

## Task 10: 收尾 —— 错误处理打磨 + README

**Files:**
- Modify: `src/main.rs`（启动时 ffmpeg 检查，仅 image/podcast/video/run 需要）
- Create: `README.md`（安装、config 向导、四步用法、`--ref`、续跑说明、内置 provider 列表）
- Test: 端到端集成测试（全 mock provider + 真实 ffmpeg）

**Step 1: 写集成测试** — 全程 wiremock + tempfile 任务目录，`run` 跑通且四个产物文件存在

**Step 2: 实现缺失部分使测试通过**

**Step 3: `cargo test` 全绿 + `cargo clippy` 无警告**

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: ffmpeg preflight check, readme, end-to-end integration test"
```

---

## 备注

- `ProviderConfig::Builtin` 的 `extra` 扩展字段（Task 7 引入）需在 Task 2 的测试中补一个 roundtrip case
- OpenAI TTS / 豆包 Seedream / OpenAI gpt-image 的实现完全复用 Task 5/6 已建立的 openai_compat 与图像模式，作为各任务内的同级小步骤完成
- 字幕、并发合成 TTS 段等优化明确不做（YAGNI），后续迭代再加
