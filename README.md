# Media Factory

本地 CLI 内容工厂：输入一段参考文案，自动完成自媒体内容生产四步流水线：

1. **改写** — 参考文案 → 适合自媒体的爆款文案（pi agent + 爆款模板）
2. **生图** — 基于改写文案生成体现核心意思的配图（可选参考图）
3. **播客** — 基于改写文案生成双人对话播客音频
4. **视频** — 配图 + 播客音频合成视频（图片贯穿全片）

## 安装与依赖

```bash
# 1. Rust 工具链（https://rustup.rs）
# 2. pi（语言模型层，负责改写/图像 prompt/播客脚本）
npm install -g @earendil-works/pi-coding-agent
# 3. ffmpeg（音频拼接与视频合成）
brew install ffmpeg          # macOS
# apt install ffmpeg          # Debian/Ubuntu

cargo build --release
```

## 快速开始

```bash
# 1. 配置（交互式向导，配好保存退出）
media-factory config

# 2. 一键跑完整流水线
media-factory run 参考文案.md [--ref 参考图.png]

# 3. 或分步执行（产物落在 output/<任务id>/）
media-factory rewrite 参考文案.md
media-factory image [--ref 参考图.png]
media-factory podcast [--script]
media-factory video
```

## 配置说明（`~/.media-factory/config.yaml`）

- **语言模型**：由 pi 管理。向导通过 pi RPC 列出你 pi 环境中已认证的模型供选择；
  认证用 `pi auth login`，自定义 provider 在 pi 的 `models.json` 中配置后自动出现在列表里。
- **生图**：
  - `nano-banana`（默认）— 官方 Gemini 图像 API，填 `GEMINI_API_KEY`
  - `openai-image` — OpenAI gpt-image
  - 第三方 nano-banana 服务（如 ModelGate）→ 选 "Other" 自定义，填 base_url / model
- **播客**：
  - `volc-podcast`（推荐默认）— 火山引擎「语音播客大模型」，文案直接生成双人播客
    - 需填 Access Token + appid（火山控制台开通：`console.volcengine.com/speech/service/10028`）
  - `openai-tts` 等通用 TTS（fallback：pi 生成脚本 → 分段合成 → ffmpeg 拼接）

## 播客两种模式

- **模式 A（默认）**：`rewritten.md` → 火山播客 API 一次调用 → 双人播客 `podcast.mp3`
- **模式 B（`--script`）**：先生成 `script.md` 供人工修改，修改后重跑
  `media-factory podcast --id <id>` 即按脚本合成

## 续跑 / 重跑

某步失败后，用 `--id <任务id>` 直接重跑失败的那步即可（上游产物已落盘）：

```bash
media-factory podcast --id 20260830-143000
```

## 产物目录

```
output/<任务id>/
  ├── input.md       # 参考文案
  ├── rewritten.md   # 改写文案
  ├── image.png      # 配图
  ├── script.md      # 播客脚本（模式 B 或 TTS fallback 时产生）
  ├── podcast.mp3    # 播客音频
  └── video.mp4      # 成品视频
```

## 开发

```bash
cargo test    # 23 个测试（含假 pi 协议、wiremock HTTP、ffmpeg 真实合成、端到端流水线）
cargo clippy  # 无警告
```

设计文档：`docs/plans/2026-08-30-media-factory-design.md`
实现计划：`docs/plans/2026-08-30-media-factory-implementation.md`
