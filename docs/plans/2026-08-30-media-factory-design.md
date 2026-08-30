# Media Factory 设计文档

**日期：** 2026-08-30
**状态：** 已 review 确认（CLI 形态 / 方案 A 子命令流水线 / Rust 技术栈）

## 1. 产品概述

本地 CLI 内容工厂：输入一段参考文案，自动完成自媒体内容生产四步流水线：

1. **改写** — 参考文案 → 适合自媒体的爆款文案（中文为主，有钩子、有爆点）
2. **生图** — 基于改写文案生成体现核心意思的配图（可选提供参考图）
3. **播客** — 基于改写文案自动生成播客音频（内部自动生成对话脚本 + 多音色合成，文案侧不关心播客形式）
4. **视频** — 配图 + 播客音频合成视频，图片贯穿全片

## 2. 总体架构（方案 A：子命令流水线）

四个独立子命令 + 一个 `run` 串联全部；每步产物落盘，可单步重跑、人工中途改稿。

**关键决策：所有语言模型能力（改写、图像 prompt 提炼、播客脚本生成）底层都交给 pi agent**，通过 pi 的 RPC 模式集成（`pi --mode rpc --no-session`，JSONL over stdio，官方推荐的跨语言集成方式）。不自己再造 agent/模型抽象层：

- 模型与 provider 的选择、认证（API Key / OAuth）、自定义 OpenAI 兼容 provider，全部复用 pi 自身能力（`auth.json` / `models.json` / `pi auth login`）
- media-factory 只存一个 `provider/model[:thinking]` 模型字符串；向导里通过 RPC `get_available_models` 列出用户 pi 环境中已认证的模型供选择
- 生图与 TTS 不属于语言任务，仍由 media-factory 直接调专用 provider API

```
media-factory config   # 交互式配置向导
media-factory rewrite  # 步骤 1：改写
media-factory image    # 步骤 2：生图（--ref 可选参考图）
media-factory podcast  # 步骤 3：播客
media-factory video    # 步骤 4：合成视频
media-factory run      # 串联执行全部四步
```

产物目录结构：

```
output/<任务id>/
  ├── input.md       # 参考文案（输入）
  ├── rewritten.md   # 步骤 1 产物：改写文案
  ├── image.png      # 步骤 2 产物：配图
  ├── script.md      # 步骤 3 中间产物：播客对话脚本（可人工修改后重跑）
  ├── podcast.mp3    # 步骤 3 产物：播客音频
  └── video.mp4      # 步骤 4 产物：成品视频
```

## 3. 配置系统

- 配置文件：`~/.media-factory/config.yaml`
- `media-factory config` 进入交互式设置向导，配置完成保存退出
- 按**任务类型**分组，每个任务独立选择 provider：

```yaml
tasks:
  llm:     { model: "google/gemini-2.5-pro" }  # pi 模型字符串（provider/model[:thinking]），留空 = pi 默认模型
  image:   { provider: nano-banana }
  podcast: { provider: volc-tts }

providers:
  # 语言模型无需在此配置 —— provider/模型/认证全部由 pi 管理
  nano-banana: { api_key: "..." }              # 内置预设，只填 API Key
  volc-tts:    { api_key: "...", extra: { appid: "...", cluster: "..." } }
  my-image-api:                                # 用户自定义（Other，仅限生图/TTS 类）
    type: openai-compatible
    base_url: "https://..."
    api_key: "..."
    model: "..."
```

### 各任务的 provider 来源

| 任务 | provider 来源 |
|---|---|
| 语言模型（改写 / 图像 prompt 提炼 / 播客脚本） | **pi agent**：向导列出 pi 已认证的模型供选择；自定义 provider 走 pi 的 `models.json`，用户自行在 pi 侧配置 |
| 生图 | 内置：nano-banana（Gemini Image，原生支持参考图）/ OpenAI gpt-image / 豆包 Seedream；支持 Other 自定义 |
| 播客 | 内置：**火山播客大模型（volc-podcast，推荐默认，文案直接生成双人播客）** / Gemini TTS / OpenAI TTS / 火山豆包语音（volc-tts）；支持 Other 自定义 |

### 自定义 provider（Other）

- 仅面向生图 / TTS 任务（语言模型的自定义 provider 已由 pi 覆盖）
- 向导中选 "Other" → 填写名称、BaseURL、API Key、模型名
- 统一按 OpenAI 兼容接口约定接入
- 保存后出现在对应任务的 provider 可选列表中

## 4. 四步流水线实现方案

1. **改写**：参考文案（文件 / stdin）→ **pi agent** + 爆款 prompt 模板（情绪钩子、反差、悬念、口语化）→ `rewritten.md`。模板文件可让用户自行调整
2. **生图**：**pi agent** 从改写文案提炼核心意象 → 生成图像 prompt → 调 image provider（media-factory 直连 API）。`--ref <图片>` 可选传参考图；provider 不支持参考图时降级为纯 prompt 并打印警告
3. **播客**：两种模式，由 provider 能力决定：
   - **火山播客大模型（volc-podcast，默认）**：走播客 API（`wss://openspeech.bytedance.com/api/v3/sami/podcasttts`，[文档](https://www.volcengine.com/docs/6561/1668014)）
     - **模式 A（默认，端到端）**：`rewritten.md` → `action=0` 一次调用 → 模型自动分析生成双人对话播客 → `podcast.mp3`
     - **模式 B（`--script`，脚本可控）**：`action=0 + only_nlp_text=true` 先生成脚本 → `script.md` 落盘（可人工修改）→ 重跑时检测到 `script.md` 已存在，改用 `action=3` 按 `nlp_texts` 合成 → `podcast.mp3`
   - **通用 TTS（gemini-tts / openai-tts / volc-tts，fallback）**：**pi agent** 生成对话脚本（主持人/嘉宾双角色）→ `script.md` 落盘（可人工修改）→ 按台词分段调 TTS（两个不同音色）→ ffmpeg 拼接 → `podcast.mp3`
4. **视频**：`image.png` + `podcast.mp3` → ffmpeg 静态图循环 + 音频 → `video.mp4`（时长 = 音频时长）。字幕本期不做（YAGNI）

## 5. 技术栈

- **Rust**
- **语言模型层：pi agent RPC 子进程**（`pi --mode rpc --no-session`，JSONL over stdio；模型/认证由 pi 管理）
- CLI：`clap`（derive 模式）
- 交互式配置向导：`dialoguer`
- 配置：`serde` + `serde_yaml`
- HTTP 客户端：`reqwest`（生图 / TTS provider 直连 API 用）
- WebSocket：`tokio-tungstenite`（火山播客 API 的自定义二进制帧协议）
- ffmpeg：subprocess 调用系统 ffmpeg（依赖前置检查；通用 TTS 路径拼接、视频合成用）

## 6. 错误处理

- 每步执行前校验上游产物存在，缺失时提示对应子命令
- 启动时检测 `pi` 与 `ffmpeg` 可执行文件，缺失时提示安装
- pi RPC 调用失败 / agent 运行出错：读取 RPC 错误事件并清晰报错；pi 内建的 auto-retry 负责瞬时错误重试
- 生图 / TTS 的 API 调用失败：指数退避重试 3 次
- 缺少配置 / API Key：提示运行 `media-factory config`；pi 侧无可用模型时提示 `pi auth login`

## 7. 测试策略

- pi RPC 客户端：用模拟 JSONL 子进程（shell script 假 `pi`）测试协议读写；LLM 任务通过 `LlmAgent` trait 注入 mock 测试
- 生图 / TTS provider 层：mock HTTP server（`wiremock`）测试请求构造与响应解析
- ffmpeg 合成：用短小的测试音频 + 图片验证产物可生成且时长正确
- prompt 模板：快照测试
- 配置系统：读写 roundtrip 测试
