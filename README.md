# Media Factory · 自媒体内容工厂

[English](./README_EN.md) | 中文

把一段**参考文案**变成**可直接发布的自媒体内容包**：爆款改写文案 + AI 配图 + 双人播客音频 + 成品视频（含字幕），一条命令完成。

```
参考文案 ──► ① 改写（爆款文案） ──► ② 生图（配图） ──► ③ 播客（双人对话） ──► ④ 分镜（动态画面脚本） ──► ⑤ 视频
```

视频有两种画面模式：**动态解释画面**（图解与音频同步，需一台算力机）或 **封面图贯穿**（最快，单机即可）。

## 核心特点

- **五步全自动流水线** — 参考文案进，`文案 + 配图 + 播客 + 分镜 + 视频` 出；支持一键全流程，也支持任意单步执行与失败续跑
- **动态解释画面** — 依据字幕时间轴自动生成与音频**逐段同步**的图解画面（要点卡 / 数据卡 / 柱状图 / 金句 / 章节页），画面比例自动跟随配图（9:16 / 16:9 / 1:1）；也可一键切回"封面图贯穿"静态模式
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

> 动态解释画面额外需要一台算力机（Docker 部署渲染 worker，见下文），**普通用户与服务端都不需要**安装 Node/Chromium。

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
# 分步：rewrite / image / podcast / scenes / video，产物在 output/<任务id>/
```

<details>
<summary>Web 界面要点</summary>

- **侧边栏**：任务列表（状态圆点 + 自动提取的标题），点击回放任意任务的完整时间线；支持单任务删除 / 全部清空
- **步骤卡片三态**：投料态（填该步骤可选输入）→ 思考态（日志流式滚动）→ 完成态（产物内联：文案可编辑、图可放大、音视频可播放）
- **重跑**：每个步骤卡片「↻ 重跑」预填上次配置；顶栏「↻ 重跑全流程」整体重跑；改写重跑会提示覆盖手动编辑
- **生图选项**：生图要求 + 📎 多张参考图 + 尺寸 + 免责声明勾选，全部任务级生效
- **分镜卡片**：选择画面模式（动态解释画面 / 封面图贯穿）；生成的 `scene.json` 可编辑，改完重跑视频即可
- **渲染进度**：动态模式下，算力机的实时进度（下载素材 → 渲染 % → 合成 → 回传）流式显示在卡片里

</details>

<details>
<summary>CLI 完整参考</summary>

```bash
media-factory run <input> [--id ID] [--ref IMG]... [--prompt S] [--image-prompt S]
                  [--podcast-prompt S] [--disclaimer] [--size square|portrait|landscape]
media-factory rewrite <input> [--prompt S]
media-factory image   [--id ID] [--ref IMG]... [--prompt S] [--disclaimer] [--size ...]
media-factory podcast [--id ID] [--script] [--prompt S]
media-factory scenes  [--id ID]                 # 生成分镜 scene.json（动态画面）
media-factory video   [--id ID]                 # 合成视频（动态/封面，按配置）
media-factory config      # 交互式配置向导
media-factory serve       # Web 服务（后台运行；--port 指定端口，--foreground 前台调试）
media-factory render-server --port 7788 --home /data   # 算力机渲染服务（供云端下发渲染任务）
```

失败续跑：`media-factory podcast --id <任务id>`（上游产物已落盘）。

</details>

## 动态解释画面（可选增强）

默认视频是「配图 + 音频 + 字幕」。开启动态模式后，流水线多出一步**分镜**：按字幕时间轴生成与音频内容**逐段同步**的图解画面。

```
改写 → 生图 → 播客 → 分镜 → 视频
                      │       │
                scene.json   动态画面 + 音频 + 字幕
               （可编辑）
```

**质量保障机制**

| 机制 | 说明 |
|---|---|
| 时间轴零漂移 | 场景边界直接吸附字幕时间戳，不靠模型估算 |
| 画面与音频不脱节 | 场景必须**连续覆盖**全部字幕；遗漏段落会按该段原文补一张金句卡 |
| 数据防错 | 数据卡的数字必须能在原文中找到（支持「十亿」↔「10亿」），否则自动降级为要点卡 |
| 比例跟随配图 | 9:16 → 1080×1920、16:9 → 1920×1080、1:1 → 1080×1080；竖屏单独优化排版 |
| 失败不断供 | 算力机不可用 / 渲染超时 → 自动降级"封面图贯穿"，任务不失败 |

场景类型：封面 / 章节 / 要点列表 / 数据卡（数字滚动）/ 柱状图 / 金句 / 结尾。

### 开启方式

- **Web 端**：投料区「分镜」卡片 → 画面模式选「动态解释画面」
- **默认值**：⚙ 配置面板 →「视频画面」；或直接改配置（见下）

### 算力机（渲染 worker）

动态渲染需要有算力的机器（Chromium 逐帧渲染）。以 **Docker 容器**部署，**完全不动宿主环境**：

```bash
# 1) 在能访问 Docker Hub 的机器上构建镜像（定义见 scripts/render-worker/Dockerfile）
cd scripts/render-worker && docker build --platform linux/amd64 -t mf-render:0.3.4 .

# 2) 传到算力机并启动（约 2.7GB，局域网传输约 1 分钟）
docker save mf-render:0.3.4 | gzip -1 | ssh <算力机> 'gunzip | sudo docker load'
ssh <算力机> 'sudo docker run -d --name mf-render --restart unless-stopped \
  -p 7788:7788 -v /srv/mf-render:/data \
  -e HTTP_PROXY= -e HTTPS_PROXY= mf-render:0.3.4'
```

服务端配置（`~/.media-factory/config.yaml`）：

```yaml
video:
  mode: dynamic                 # dynamic（动态解释画面）| cover（封面图贯穿）
  dynamic:
    renderer_url: http://<算力机地址>:7788
    callback_base: http://<本服务公网地址>:8092   # 算力机据此下载素材、回传成品
    token: <与算力机 /data/render-token 内容一致>
    fps: 24                     # 24 | 30
    quality: looks              # draft | looks | delivery
    on_unavailable: cover       # 算力机不可用：cover 先保交付 | queue 排队等待
```

> 成品回传成功后，**算力机会自动删除本地成品与中间文件**（只在云端保留），不占用算力机磁盘。
> 实测：4.4 分钟音频渲染约 1~1.5 分钟（32 线程机器）。
> 完整部署/升级/自检说明见 [`scripts/render-worker/README.md`](scripts/render-worker/README.md)。

## 配置（`~/.media-factory/config.yaml`）

- **语言模型**：默认 `pi`（用 `pi auth login` 认证，模型由 pi 管理）；或自定义 OpenAI 兼容 provider（如 Deepseek：填 BaseURL + API Key + 模型）
- **生图**：`nano-banana`（官方 Gemini，默认）/ `openai-image`（gpt-image）/ `doubao-seedream`（豆包 Seedream 4.0，方舟 Ark：支持多图参考、真实 9:16/16:9、自动关水印，可填模型名或 ep- 端点）/ 自定义 OpenAI 兼容（如 ModelGate）
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
  ├── scene.json                # 分镜脚本（动态画面，可编辑后重跑渲染）
  └── video.mp4                 # 成品视频（动态解释画面 或 封面图贯穿）
```

## 平台支持

| 平台 | 预编译包 |
|------|-----------|
| macOS Apple Silicon / Intel | ✅ |
| Linux x64（含 WSL，需 ffmpeg + fonts-noto-cjk） | ✅ |
| Windows x64（`media-factory.exe`，serve 后台运行不适用，请用 `--foreground`） | ✅ |

## 开发

```bash
cargo test    # 67 个测试（协议 wiremock / ffmpeg 真实合成 / 分镜覆盖性 / 端到端流水线）
cargo clippy  # 0 警告
media-factory serve --restart   # Web 服务管理：--stop / --restart / --status
```

设计文档见 `docs/plans/`。

## License

MIT
