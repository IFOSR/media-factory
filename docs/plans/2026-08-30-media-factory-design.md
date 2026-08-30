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
  rewrite: { provider: gemini }
  image:   { provider: nano-banana }
  podcast: { provider: volc-tts }

providers:
  gemini:      { api_key: "..." }              # 内置预设，只填 API Key
  nano-banana: { api_key: "..." }
  volc-tts:    { api_key: "..." }
  my-custom:                                   # 用户自定义（Other）
    type: openai-compatible
    base_url: "https://..."
    api_key: "..."
    model: "..."
```

### 内置 provider 预设

| 任务 | 内置 provider |
|---|---|
| 改写（LLM） | Gemini / OpenAI / 豆包（火山） |
| 生图 | nano-banana（Gemini Image，原生支持参考图）/ OpenAI gpt-image / 豆包 Seedream |
| 播客 TTS | Gemini TTS / OpenAI TTS / 火山豆包语音 |

### 自定义 provider

- 向导中选 "Other" → 填写名称、BaseURL、API Key、模型名
- 统一按 OpenAI 兼容接口约定接入
- 保存后出现在对应任务的 provider 可选列表中

## 4. 四步流水线实现方案

1. **改写**：参考文案（文件 / stdin）→ LLM + 爆款 prompt 模板（情绪钩子、反差、悬念、口语化）→ `rewritten.md`。prompt 模板为独立文件，用户可自行调整
2. **生图**：LLM 从改写文案提炼核心意象 → 生成图像 prompt → 调 image provider。`--ref <图片>` 可选传参考图；provider 不支持参考图时降级为纯 prompt 并打印警告
3. **播客**：改写文案 → LLM 生成对话脚本（主持人/嘉宾双角色）→ `script.md` 落盘（可人工修改）→ 按台词分段调 TTS（两个不同音色）→ ffmpeg 拼接 → `podcast.mp3`
4. **视频**：`image.png` + `podcast.mp3` → ffmpeg 静态图循环 + 音频 → `video.mp4`（时长 = 音频时长）。字幕本期不做（YAGNI）

## 5. 技术栈

- **Rust**
- CLI：`clap`（derive 模式）
- 交互式配置向导：`dialoguer`
- 配置：`serde` + `serde_yaml`
- HTTP 客户端：`reqwest`（blocking 或 tokio，视并发需要）
- ffmpeg：subprocess 调用系统 ffmpeg（依赖前置检查）

## 6. 错误处理

- 每步执行前校验上游产物存在，缺失时提示对应子命令
- API 调用失败：指数退避重试 3 次
- 缺少配置 / API Key：提示运行 `media-factory config`
- ffmpeg 不存在：启动时检测并提示安装

## 7. 测试策略

- provider 层：mock HTTP server（`wiremock`）测试请求构造与响应解析
- ffmpeg 合成：用短小的测试音频 + 图片验证产物可生成且时长正确
- prompt 模板：快照测试
- 配置系统：读写 roundtrip 测试
