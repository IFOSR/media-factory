# 动态解释画面（分镜渲染）设计方案

日期：2026-09-16
状态：待评审
关联：`docs/plans/2026-09-02-ux-redesign-design.md`（思考链路 + 步骤卡片）

## 一、目标

在现有四步流水线（改写 → 生图 → 播客 → 视频）基础上，让视频从"单张封面图贯穿全片"升级为：

**封面开场 + 后续全程与音频内容同步的动态解释画面**（图表、要点、数据卡、关键词），帮助听众理解音频内容。

```
改写 → 生图 → 播客 → 〔分镜〕 → 视频
                        │           │
                   scene.json      渲染（远端算力机 way）
                   （可编辑）       ↓
                              成品 video.mp4
```

## 二、已完成的可行性验证（Spike 结论）

| 项目 | 实测结果 |
|---|---|
| 渲染引擎 | HeyGen 开源 **HyperFrames** `0.8.41`（50.5k stars，Apache-2.0，专为 agent 设计：HTML → MP4） |
| 20s 1080p30 渲染 | Mac(i9/16核) **63s** ／ **way(Ryzen AI MAX+ 395/32线程) 9.2s**（6.8×） |
| 外推 4.4 分钟音频 | way 上约 **1.1~1.5 分钟**（24fps/16 workers 更快） |
| 跨平台一致性 | way 与 Mac 渲染帧亮度几乎一致，输出无差异 |
| 安装体积 | npm 依赖 732MB + Chromium 261MB ≈ **1GB**（仅算力机需要） |
| 国内可安装性 | npm/Chromium 全走 npmmirror，实测 Chromium **6.2s** 下完 |
| 画面形态 | 封面（复用现有 `image.png`）→ 要点列表 → 数据卡（含数字 count-up） |
| 字幕/免责声明 | 字幕仍由 ffmpeg 烧录（复用现有管线）；免责声明改为**全程常驻**叠加层 |

**关键工程坑（已解决，需写入安装脚本）**
1. `hyperframes init` 会挂（giget 拉 GitHub 模板）→ **不使用 init**，模板自持（组合就是普通 HTML）
2. 官方模板依赖 jsdelivr CDN 的 GSAP → **必须本地化**（`assets/gsap.min.js`）
3. `hyperframes browser ensure` 在 way 上卡死 → **自行 curl 镜像 + `HYPERFRAMES_BROWSER_PATH` 指定**
4. 字体栈必须含 `Noto Sans CJK SC`（Linux 无 PingFang），算力机需装 `fonts-noto-cjk`
5. 需关闭遥测与更新检查：`HYPERFRAMES_NO_TELEMETRY=1` / `HYPERFRAMES_NO_UPDATE_CHECK=1` / `HYPERFRAMES_NO_AUTO_INSTALL=1`

## 三、总体架构

```
┌──────────────────────────── 云端 huoshan（公网 14.103.216.193，2核/3G）────────────────────────┐
│ media-factory serve :8092                                                                     │
│  ├─ Web UI / REST API（现有）                                                                  │
│  ├─ 流水线：改写 → 生图 → 播客 → 分镜 → 视频                                                     │
│  ├─ 分镜步骤：LLM 生成 scene.json（新，可编辑中间产物）                                            │
│  ├─ 渲染分发：video.mode = dynamic（算力机）| cover（本机静态）                                 │
│  └─ 接收回传：/api/render-results（成品落盘 output/<id>/video.mp4）                              │
└───────────────────────────────┬───────────────────────────────────────────────────────────────┘
      控制面：Tailscale（100.84.242.125 → 100.75.20.123，直连 5-7ms）
                                 │  POST /render  { scene.json + 素材URL + 回传地址 }
                                 ▼
┌──────────────────── 算力机 way（内网 192.168.1.4，Ryzen AI MAX+ 395 32线程/122G）───────────────┐
│ media-factory render-server :7788                                                             │
│  ① 下载素材（封面/音频/字幕，公网 HTTPS，约 4MB）                                                │
│  ② 模板填充：scene.json → composition.html（+ 本地 GSAP）                                        │
│  ③ hyperframes render → 视觉层 mp4                                                             │
│  ④ ffmpeg 合成：视觉层 + 音频 + 字幕（含常驻免责声明）→ 成品 video.mp4                             │
│  ⑤ 回传：POST 成品到云端 /api/render-results（公网 HTTPS，实测 6 MB/s）                          │
│  ⑥ 云端确认收到 → 立即删除本地成品与中间产物 ★（不占空间）                                        │
│  ⑦ 进度回调：POST /api/render-jobs/<id>/progress（接入云端思考链路）                             │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 为什么控制面与数据面分开走

实测（20MB 上传对比）：

| 路径 | 吞吐 | 说明 |
|---|---|---|
| 公网 HTTPS（way → 云端） | **6.09 MB/s** | 成品回传走这条 |
| Tailscale 直连 | 1.83 MB/s | 仅用于小数据量控制信令 |
| Tailscale 中继（DERP 香港） | < 1 MB/s | 会不定期退化到中继，大文件不可用 |

结论：**Tailscale 解决"够得着"（NAT 穿透/固定地址/加密），公网解决"搬得动"**。

## 四、新增步骤：「分镜」

在 `podcast` 与 `video` 之间插入第 5 步 `scenes`：

| 状态 | 卡片内容 |
|---|---|
| 投料态 | **画面模式（动态解释画面 / 封面图贯穿）**、风格（dark-tech/light-clean/…）、场景密度（每 N 秒一个视觉锚点）、时长上限、是否启用图表 |
| 思考态 | 日志流式：`读取字幕时间轴（105 段）` → `LLM 规划场景` → `校验 3 个数据场景` → `已生成 12 个场景` |
| 完成态 | `scene.json` 可编辑（复用现有「文案可编辑 + 保存」机制）+ 场景缩略预览；`cover` 模式下显示"已跳过（静态模式）" |

理由：`scene.json` 是可编辑中间产物，独立成步才能单独编辑/重跑/查看进度，符合现有"每步可投料、可重跑、产物可干预"的产品哲学。
同时渲染慢（分钟级）且属于 `video` 步骤的重活，抽出来能让 `video` 保持轻快、失败重试成本低。

## 五、scene.json 数据契约

```json
{
  "version": 1,
  "style": "dark-tech",
  "meta": { "title": "美联储降息，金价怎么走？", "total_duration": 265.8, "scene_count": 12 },
  "scenes": [
    { "type": "cover",     "from_entry": 0,  "to_entry": 0,  "title": "美联储降息，金价怎么走？", "subtitle": "三种可能的走势推演" },
    { "type": "chapter",   "from_entry": 1,  "to_entry": 1,  "title": "三种可能的走势" },
    { "type": "bullets",   "from_entry": 2,  "to_entry": 5,  "heading": "三种可能", "items": ["降息落地 → 冲高回落", "鹰派降息 → 承压震荡", "衰退交易 → 避险推升"] },
    { "type": "metric",    "from_entry": 6,  "to_entry": 8,  "value": "10", "unit": "亿", "label": "ChatGPT 周活跃用户",
                           "source_quote": "ChatGPT 每周的活跃用户已经突破 10 亿" },
    { "type": "bar_chart", "from_entry": 9,  "to_entry": 14, "heading": "五年份额变化", "unit": "%",
                           "data": [{ "label": "2019", "value": 12 }, { "label": "2024", "value": 38 }],
                           "source_quote": "……" },
    { "type": "quote",     "from_entry": 15, "to_entry": 17, "text": "……", "speaker": "嘉宾" }
  ]
}
```

### 两条硬性约束（防错设计）

1. **时间用字幕索引，不用时间戳**：`from_entry` / `to_entry` 指向 `subtitle.srt` 的条目序号，服务端据此算出精确 `start` / `end`。LLM 无法写错时间，且场景边界天然吸附语音。
   - 校验：场景必须**有序、不重叠、覆盖全时间轴**（空隙自动并入前一个场景）
2. **数据类场景必须带 `source_quote`**：服务端校验数字确实出现在原文中，否则**该场景自动降级**为 `bullets` 或 `quote`，防止 LLM 编造数据（财经/知识内容高危点）

## 六、渲染服务协议

### 6.1 云端 → way（控制面，Tailscale）

```
POST http://100.75.20.123:7788/render
X-Render-Token: <shared secret>

{
  "job_id": "20260916-120000-ab12",
  "callback": {
    "base": "https://14.103.216.193:8092",
    "token": "<per-job 一次性 token>",
    "progress_path": "/api/render-jobs/{job_id}/progress",
    "result_path": "/api/render-results/{job_id}",
    "failed_path": "/api/render-jobs/{job_id}/failed"
  },
  "composition": { "template": "explainer-v1", "style": "dark-tech", "scenes": [ ... ] },
  "media": {
    "cover":     "https://14.103.216.193:8092/api/files/<task>/image.png",
    "audio":     "https://14.103.216.193:8092/api/files/<task>/podcast.mp3",
    "subtitles": "https://14.103.216.193:8092/api/files/<task>/subtitle.srt"
  },
  "output": {
    "width": 1920, "height": 1080, "fps": 24, "quality": "looks",
    "burn_subtitles": true,
    "disclaimer": "以上内容仅代表个人观点。不构成投资建议。"
  }
}

→ 202 { "accepted": true, "eta_seconds": 90, "queue_position": 0 }
→ 409 { "error": "busy", "queue_position": 2 }     // 并发上限保护
```

素材由 way **主动下载**（公网 HTTPS，几 MB，简单可靠），不做 multipart 上传。

### 6.2 way → 云端（数据面，公网）

```
进度（多次）：  POST /api/render-jobs/<job_id>/progress
                { "stage": "capture", "percent": 37, "message": "渲染场景 5/12" }
                → 云端转成现有 SSE 事件推给 Web UI（复用思考链路）

成品（一次）：  POST /api/render-results/<job_id>   (multipart, file=video.mp4)
                → { "ok": true, "size": 25431821, "sha256": "..." }

失败（一次）：  POST /api/render-jobs/<job_id>/failed
                { "error": "hyperframes 渲染失败: ...", "log_tail": "..." }
```

云端收到成品后：写入 `output/<task>/video.mp4` → 记录 sha256 → 返回 `ok`。

### 6.3 算力机侧清理策略 ★（按你的要求）

```
渲染完成 → 上传成品 → 云端返回 ok 且 sha256 校验一致
        → 立即删除 way 本地：成品 mp4 + 视觉层 mp4 + 渲染帧缓存
        → 仅在内存/日志中保留统计（耗时、体积、场景数）

上传失败 → 保留本地成品 + 重试（指数退避，最多 5 次，间隔 10s/30s/2min/5min/10min）
        → 仍失败：本地落一份 pending 清单，下次服务启动时重传
        → 同时把失败状态回传云端（云端标记该任务 video 步骤失败，可手动重跑）

磁盘保护：
  - 每次任务前检查可用空间（< 10GB 拒绝新任务并告警）
  - 中间产物统一放 /tmp/mf-render/<job_id>/（任务结束即删，无论成功失败）
  - 保留策略：不保留历史成品（默认 0 份；可用环境变量 MF_RENDER_KEEP=n 调整）
```

## 七、画面模式：让用户选"动态效果"还是"封面贯穿"

**不暴露 `local` / `remote` 这类机器视角的命名**（用户不关心在哪台机器渲染，只关心画面效果），改为按**画面形态**命名：

| 模式 | 含义 | 在哪里合成 |
|---|---|---|
| **`dynamic`｜动态解释画面（推荐）** | HyperFrames 按 `scene.json` 渲染图表/要点/数据卡，与音频同步 | 算力机 way 的渲染服务 |
| **`cover`｜封面图贯穿（静态）** | 现有能力：封面图 + 音频 + 字幕，全程静态 | 服务本机 ffmpeg（云端 2 核也能跑，已在运行） |

```yaml
# ~/.media-factory/config.yaml
video:
  mode: dynamic                  # dynamic（默认）| cover
  dynamic:
    renderer_url: http://100.75.20.123:7788     # 算力机渲染服务
    token: "***"
    fps: 24                      # 24 | 30
    quality: looks               # draft | looks | delivery
    on_unavailable: queue        # 算力机不可用时：queue（排队等恢复）| cover（先出静态版）
```

- **任务级可覆盖**：与"图片尺寸""播客音色"一致，在「分镜」卡片投料态选择，随任务保存
- **`cover` 模式自动跳过「分镜」步骤**：不调 LLM、不生成 scene.json，直接进视频合成（最快路径）
- **`dynamic` 模式**：分镜 → scene.json（可编辑）→ 算力机渲染 → 回传 → 出片

```rust
// src/render/mod.rs（新增）
pub enum VideoMode { Dynamic, Cover }

#[async_trait]
pub trait VideoComposer: Send + Sync {
    async fn compose(&self, plan: Option<&ScenePlan>, events: &TaskEvents) -> anyhow::Result<PathBuf>;
}
pub struct CoverComposer;     // 现有实现：image + audio + srt → mp4（本机 ffmpeg）
pub struct DynamicComposer;   // 调算力机渲染服务 + 接收回传 + 清理
```

### 降级链（保证不断供）

| 失败点 | 处理 |
|---|---|
| `dynamic` 且算力机不可用 | 按 `on_unavailable`：`queue` 排队等恢复（画面质量优先）／`cover` 先出静态版（交付优先） |
| 分镜 LLM 失败 | 退化为纯关键词卡场景；再失败则按上一条处理 |
| 单个数据场景数字校验失败 | 该场景降级为要点卡 |
| 算力机渲染失败 / 回传超时 | 重试 → 仍失败则按 `on_unavailable` 处理，并把失败原因写入任务错误 |
| `cover` 模式 | 不依赖算力机，本机 ffmpeg 直接完成 |

## 八、Web UI 变更

1. 新增第 5 张卡片「分镜」（投料/思考/完成三态，见第四节）
2. `video` 卡片：显示当前画面模式与渲染机（`动态解释画面 · way`）、ETA、实时进度（来自 way 的 progress 回调）
3. 渲染耗时提示：预计 `N 分钟`（基于 fPS + 音频时长 + 场景数估算）
4. `scene.json` 编辑后重跑：只重跑 `video` 步骤（不必重跑分镜 LLM）
5. 任务列表：显示"渲染中（way）"状态区分

## 九、算力机部署（way）

```bash
# 依赖（一次性）
sudo apt-get install -y ffmpeg fonts-noto-cjk fonts-noto-cjk-extra unzip nodejs npm
npm i -g hyperframes@0.8.41 --registry=https://registry.npmmirror.com    # 或本地 prefix
curl -fsSL -o /tmp/chrome.zip https://cdn.npmmirror.com/binaries/chrome-for-testing/152.0.7977.30/linux64/chrome-headless-shell-linux64.zip
unzip /tmp/chrome.zip -d ~/.media-factory-render/chrome && rm /tmp/chrome.zip
sudo apt-get install -y libnss3 libnspr4 libatk1.0-0t64 libatk-bridge2.0-0t64 libcups2t64 \
  libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 \
  libpango-1.0-0 libcairo2 libasound2t64 libatspi2.0-0t64 libxshmfence1

# 服务（systemd，Restart=always）
# /etc/systemd/system/mf-render.service
ExecStart=/usr/local/bin/media-factory render-server --port 7788 --token-file /etc/mf-render/token \
          --browser-path /home/way/.media-factory-render/chrome/chrome-headless-shell-linux64/chrome-headless-shell \
          --max-concurrent 1
```

环境变量（服务内固定）：`HYPERFRAMES_BROWSER_PATH` / `HYPERFRAMES_NO_TELEMETRY=1` / `HYPERFRAMES_NO_UPDATE_CHECK=1` / `HYPERFRAMES_NO_AUTO_INSTALL=1` / `HYPERFRAMES_FFMPEG_PATH=/usr/bin/ffmpeg`

资源与并发：
- 单次渲染用 6~16 个 Chromium worker；**并发上限默认 1**（32 线程机器，避免多任务互相抢核）
- 内存：每 worker ~256MB，同时 2 个任务峰值约 8GB（122G 内存充裕）
- 排队：超出并发上限返回 `409 busy`，云端排入等待队列（FIFO）

## 十、安全

| 层 | 措施 |
|---|---|
| 网络层 | Tailscale ACL：仅允许 `huoshan → way:7788`（关闭默认全网互通） |
| 应用层 | `X-Render-Token` 共享密钥（way 侧校验）+ 云端 per-job 一次性 token（回传校验） |
| 传输层 | 控制面 WireGuard 加密；数据面 HTTPS（现有 Let's Encrypt 证书） |
| 回传校验 | 云端记录 sha256，way 只在收到 `ok` 后删除本地文件 |
| 密钥管理 | auth key 轮换（已暴露的需撤销）；render token 存 `/etc/mf-render/token`（600 权限） |

## 十一、模板库规划

| 场景类型 | 用途 | 优先级 |
|---|---|---|
| `cover` | 封面（复用 image.png + 标题 + 免责声明） | P0 |
| `chapter` | 章节标题/过渡 | P0 |
| `bullets` | 要点列表（逐条出现） | P0 |
| `metric` | 数据卡（大数字 + 单位 + 说明 + count-up） | P0 |
| `bar_chart` / `line_chart` | 趋势、对比 | P1 |
| `quote` | 金句/观点强调 | P1 |
| `compare` | 左右对比 / 前后对照 | P2 |
| `timeline` | 事件序列 | P2 |

视觉规范：沿用现有品牌色（`#0c0d11` 底 / `#6e7bff` 强调 / `#fbbf24` 免责声明），
字号与安全边距遵循"移动端可读"（1080p 下正文 ≥ 40px、标题 ≥ 64px）。

场景预算：每 **10~15 秒**一个视觉锚点，全程上限 **20 个**（长音频抽稀；>10 分钟建议只做章节级）。

## 十二、实施计划（分阶段）

| 阶段 | 内容 | 产出 |
|---|---|---|
| **M0 通信打通** | way 起 `render-server`（接收 job → 下载素材 → 渲染 → 合成 → 回传 → 删本地）；云端手动触发验证 | 端到端跑通一次真实任务 |
| **M1 分镜步骤** | scene.json schema + LLM prompt + 数字校验 + 模板库 P0 + UI 第 5 卡 | 可编辑 scene.json → 出片 |
| **M2 流水线集成** | `video.mode = dynamic\|cover` 抽象 + 降级链 + 进度回调接思考链路 | 全流程自动化 |
| **M3 工程化** | systemd/ACL/token/清理策略/磁盘监控/超时 | 可长期无人值守运行 |
| **M4 模板扩展** | bar_chart / quote / compare / timeline | 视觉表现力提升 |

## 十三、风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| LLM 分镜质量不稳定 | 场景与内容不匹配 | `source_quote` 校验 + 降级 + scene.json 人工可编辑 |
| 家庭宽带上行有限（实测 6MB/s） | 多任务并发回传排队 | 排队 + 成品体积控制（1080p24 CRF16，4.4 分钟约 15~30MB） |
| way 离线/断电 | 动态模式不可用 | 按 `video.dynamic.on_unavailable` 处理（queue 排队 / cover 先出静态版） |
| Tailscale 控制面境外波动 | 云端无法下发任务 | 小数据重试；必要时叠加 frp 备通道 |
| 超长音频（>10 分钟） | 场景过多、渲染过久 | 抽稀 + 章节级场景 + ETA 提示 |
| 算力机磁盘被占满 | 渲染失败 | 任务级临时目录 + 上传后即删 + 空间预检 |

## 十四、待确认事项

1. 分镜步骤是否作为独立第 5 步（本方案默认：是）；`cover` 模式自动跳过该步
2. 算力机不可用时的默认行为：`queue`（排队等恢复）还是 `cover`（先出静态版）
2. 成品视频规格：1080p / 24fps / CRF16（约 15~30MB per 4.4min）是否接受
3. 并发策略：way 上默认并发 1（排队），是否允许提到 2
4. 模板视觉：spike 样片（`~/Desktop/mf-spike-sample/`）观感确认后定稿
