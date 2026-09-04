# Media Factory · 自媒体内容工厂

[English](./README_EN.md) | 中文

把一段**参考文案**变成**可直接发布的自媒体内容包**：爆款改写文案 + AI 配图 + 双人播客音频 + 成品视频（含字幕），一条命令完成。

```
参考文案 ──► ① 改写（爆款文案） ──► ② 生图（配图） ──► ③ 播客（双人对话） ──► ④ 视频（图+音+字幕）
```

## 核心特点

- **四步全自动流水线** — 参考文案进，`文案 + 配图 + 播客 + 视频` 四件套出；支持一键全流程，也支持任意单步执行与失败续跑
- **Agent 思考链路可视化** — Web 端实时流式展示后台每一步在做什么（读取文案 → 调用模型 → 生成产物），像看 AI Agent 干活一样透明
- **产物全程可干预** — 每个中间产物（改写稿/播客脚本）可预览、**可编辑保存**，后续步骤自动基于修改后内容；任意步骤可**重跑**（自动预填上次配置，原地产物刷新，下游提示"上游已更新"）
- **多 Provider 可插拔** — 语言模型（内置 pi / OpenAI 兼容如 Deepseek）、生图（Gemini / OpenAI 兼容）、播客（火山「语音播客大模型」）自由组合，配置向导或 Web 面板随时切换
- **面向发布的细节** — 生图尺寸（1:1 / 手机竖屏 9:16 / 横屏 16:9）、多张参考图、**免责声明自动叠加**（投资类内容合规）、字幕英文词边界不拆词、双人播客主持/嘉宾角色自动识别、任务标题自动提取
- **CLI + Web 双模式** — 命令行一条命令；Web 端（默认 `http://localhost:8092`）提供侧边栏任务管理、步骤卡片、产物内联播放、深浅色主题
- **跨平台** — macOS（Apple Silicon/Intel）、Linux、Windows 预编译包，一条命令安装

## 快速安装

### 一条命令（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/IFOSR/media-factory/main/install.sh | bash
```

下载源策略：**自建镜像优先（国内友好，含 md5 校验）→ GitHub 回退 → 源码编译回退**（缺失 Rust 会自动安装 rustup）。

```bash
./install.sh --mirror   # 强制自建镜像
./install.sh --github   # 强制 GitHub
MF_MIRROR=https://你的镜像 ./install.sh   # 覆盖镜像地址
```

<details>
<summary>更多安装方式</summary>

```bash
git clone https://github.com/IFOSR/media-factory.git && cd media-factory
./install.sh                # Release 优先，自动回退源码
./install.sh --release      # 仅预编译包
./install.sh --source       # 仅源码编译
./install.sh --bin-dir /usr/local/bin   # 自定义目录（默认 ~/.media-factory/bin）
```

手动源码安装：安装 [Rust](https://rustup.rs) 后 `cargo build --release`。

</details>

### 运行依赖

| 依赖 | 必需性 | 安装 |
|------|--------|------|
| ffmpeg | 播客/视频步骤必需 | `brew install ffmpeg` / `apt install ffmpeg` / `winget install ffmpeg` |
| pi | 默认语言模型；可换自定义 provider 免装 | `npm install -g @earendil-works/pi-coding-agent` |

## 快速使用（3 步）

```bash
# ① 配置：交互式向导选模型、填密钥（也支持 Web 端 ⚙ 面板配置）
media-factory config

# ② 启动 Web 服务
media-factory serve            # 后台启动，打开 http://localhost:8092（服务器上外网用 http://<IP>:8092）

# ③ 在 Web 端：＋新建任务 → 填参考文案 → 🚀 一键全流程
```

或纯命令行：

```bash
media-factory run 参考文案.md --disclaimer --size portrait
# 分步：rewrite / image / podcast / video，产物在 output/<任务id>/
```

<details>
<summary>Web 界面要点</summary>

- **侧边栏**：任务列表（状态圆点 + 自动提取的标题），点击回放任意任务的完整时间线；支持单任务删除 / 全部清空
- **步骤卡片三态**：投料态（填该步骤可选输入）→ 思考态（日志流式滚动）→ 完成态（产物内联：文案可编辑、图可放大、音视频可播放）
- **重跑**：每个步骤卡片「↻ 重跑」预填上次配置；顶栏「↻ 重跑全流程」整体重跑；改写重跑会提示覆盖手动编辑
- **生图选项**：生图要求 + 📎 多张参考图 + 尺寸 + 免责声明勾选，全部任务级生效

</details>

<details>
<summary>CLI 完整参考</summary>

```bash
media-factory run <input> [--id ID] [--ref IMG]... [--prompt S] [--image-prompt S]
                  [--podcast-prompt S] [--disclaimer] [--size square|portrait|landscape]
media-factory rewrite <input> [--prompt S]
media-factory image   [--id ID] [--ref IMG]... [--prompt S] [--disclaimer] [--size ...]
media-factory podcast [--id ID] [--script] [--prompt S]
media-factory video   [--id ID]
media-factory config      # 交互式配置向导
media-factory serve       # Web 服务（后台运行；--port 指定端口，--foreground 前台调试）
```

失败续跑：`media-factory podcast --id <任务id>`（上游产物已落盘）。

</details>

## 配置（`~/.media-factory/config.yaml`）

- **语言模型**：默认 `pi`（用 `pi auth login` 认证，模型由 pi 管理）；或自定义 OpenAI 兼容 provider（如 Deepseek：填 BaseURL + API Key + 模型）
- **生图**：`nano-banana`（官方 Gemini，默认）/ `openai-image`（gpt-image）/ 自定义 OpenAI 兼容（如 ModelGate）
- **播客**：`volc-podcast`（火山语音播客大模型，推荐；需 Access Token + appid，[控制台开通](https://console.volcengine.com/speech/service/10028)）/ 通用 TTS（openai-tts 等，自动生成脚本→分段合成→拼接）
  - 火山播客为**双人对话**模型（说话人随机开场，角色自动识别）

播客两种模式：模式 A（默认）文案直接合成；模式 B（`--script`）先生成脚本供人工修改后重跑合成。

## 产物目录

```
output/<任务id>/
  ├── input.md / rewritten.md   # 参考 / 改写文案（可编辑，编辑后下游生效）
  ├── image.png                 # 配图
  ├── script.md                 # 播客脚本（模式 B / TTS）
  ├── podcast.mp3 / subtitle.srt# 播客音频 / 字幕
  └── video.mp4                 # 成品视频
```

## 平台支持

| 平台 | 预编译包 |
|------|-----------|
| macOS Apple Silicon / Intel | ✅ |
| Linux x64（含 WSL，需 ffmpeg + fonts-noto-cjk） | ✅ |
| Windows x64（`media-factory.exe`，serve 后台运行不适用，请用 `--foreground`） | ✅ |

## 开发

```bash
cargo test    # 41 个测试（协议 wiremock / ffmpeg 真实合成 / 端到端流水线）
cargo clippy  # 0 警告
media-factory serve --restart   # Web 服务管理：--stop / --restart / --status
```

设计文档见 `docs/plans/`。

## License

MIT
