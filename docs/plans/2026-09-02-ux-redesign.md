# UX 重构实现计划（Agent 思考链路版）

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 把 Media Factory 前端重构为"侧边栏 + 思考时间线"的商用级交互，后端为每步埋"思考日志"事件并流式推送。

**Architecture:** 后端在现有 SSE `Event` 枚举上新增 `Log` 变体并在 `TaskEvents` 加 `log()`，各 cmd 在关键节点埋点；前端整体重写 `web/index.html` 为"步骤卡片三态合一"结构（投料态/思考态/完成态）。

**Tech Stack:** Rust + axum + tokio broadcast（SSE），原生 HTML/CSS/JS（无框架，include_str! 内嵌）。

**设计文档：** `docs/plans/2026-09-02-ux-redesign-design.md`

---

## Task 1: 后端新增 Event::Log + TaskEvents::log

**Files:**
- Modify: `src/task.rs`（Event 枚举 + TaskEvents）

**Step 1: 新增 Log 变体**

在 `Event` 枚举中 `Step` 之后插入：

```rust
    /// 思考链路动作日志（流式展示后台处理细节）
    Log { step: String, text: String },
```

**Step 2: 新增 log() 方法**

在 `TaskEvents` impl 中 `chunk` 附近插入：

```rust
    /// 发送思考链路动作日志；CLI 模式下同时 println
    pub fn log(&self, step: Step, text: &str) {
        if self.sender.is_none() {
            println!("  · {}", text);
        }
        self.emit(Event::Log { step: step.as_str().into(), text: text.into() });
    }
```

**Step 3: 序列化测试**

`task.rs` 加测试模块（若无则建）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_event_serializes() {
        let ev = Event::Log { step: "rewrite".into(), text: "读取参考文案".into() };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(s, r#"{"type":"log","step":"rewrite","text":"读取参考文案"}"#);
    }
    #[test]
    fn log_local_mode_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let ev = TaskEvents::local(dir.path(), "t1");
        ev.log(Step::Rewrite, "测试");  // 不应 panic
    }
}
```

**Step 4: 运行测试**

Run: `cargo test log_ -v` → Expected: PASS（2 个）

**Step 5: Commit**

```bash
git commit -m "feat: add Event::Log + TaskEvents::log for thinking-chain streaming"
```

---

## Task 2: 各 cmd 埋点思考日志

**Files:**
- Modify: `src/cmd/rewrite.rs` `src/cmd/image.rs` `src/cmd/podcast.rs` `src/cmd/video.rs`

在关键节点调用 `events.log(Step::X, "...")`。改写文案不再发 chunk（移除 typewriter 数据源）：

**rewrite.rs run_with：**
- 写 input.md 前：`events.log(Step::Rewrite, "读取参考文案与改写要求");`
- `llm.complete` 前：`events.log(Step::Rewrite, "调用语言模型生成改写稿");`
- 成功后：`events.log(Step::Rewrite, &format!("已生成改写稿（{} 字）", out.chars().count()));`
- 删除 `events.chunk(Step::Rewrite, &out);`（前端不再打字机）

**image.rs run_with：**
- distill_prompt 前：`events.log(Step::Image, "提炼图像 prompt");`
- generate 前：`events.log(Step::Image, &format!("调用生图服务（尺寸 {}，参考图 {} 张）", size_label, reference.len()));`
- disclaimer 分支：`events.log(Step::Image, "叠加免责声明");`
- 成功后：`events.log(Step::Image, "已保存配图 image.png");`

**podcast.rs run_with：**
- 开头：`events.log(Step::Podcast, &format!("整理播客输入（{} 人）", n));`
- 模式B脚本生成：`events.log(Step::Podcast, "生成双人对话脚本");`
- 合成前：`events.log(Step::Podcast, "调用播客服务合成音频");`
- 字幕分支：`events.log(Step::Podcast, "捕获字幕时间轴");`
- 成功：`events.log(Step::Podcast, "已合成播客音频");`

**video.rs：**
- 开头：`events.log(Step::Video, "校验配图与音频");`
- make_video 前：`events.log(Step::Video, "ffmpeg 合成视频（图片+音频+字幕）");`
- 成功：`events.log(Step::Video, "已生成成品视频");`

**Step: 运行测试 + clippy**

Run: `cargo test` → 30+ 全过；`cargo clippy` → 0 警告。

**Commit:** `feat: instrument thinking-chain logs across four pipeline steps`

---

## Task 3: 前端整体重写 — 新视觉 token + 布局骨架

**Files:**
- Rewrite: `web/index.html`（整体重写，保留配置弹窗逻辑函数可复用）

**Step 1: 设计 token + 基础样式**

写入 `:root` / `[data-theme="light"]` 新 token（见设计文档视觉系统节），重置 `*`，body 无外边距、flex 布局占满屏。

**Step 2: 布局骨架 HTML**

```html
<body>
  <aside id="sidebar">
    <button id="btnNew">＋ 新建任务</button>
    <div id="taskList"></div>
  </aside>
  <main id="main">
    <header id="topbar">…主题/配置按钮…</header>
    <div id="timeline"><!-- 四张步骤卡片 JS 渲染 --></div>
  </main>
  <!-- 配置弹窗 + provider 弹窗 + 图片灯箱，沿用原逻辑，套新样式 -->
</body>
```

**Step 3: 时间线 / 卡片 / 日志 CSS**

`.step-card`（圆角+微光边框）、`.step-card[data-state]` 三态、`.log-line`（等宽小号、淡入动画）、时间线竖线 + 状态圆点、`.artifact-card`。

**Commit:** `feat(web): new design tokens + sidebar/timeline layout skeleton`

---

## Task 4: 步骤卡片三态状态机

**Step 1:** JS 定义 `STEPS` 配置（每步 id/name/input 字段定义/产物映射）。

**Step 2:** `renderTimeline(mode)`：mode='input' 渲染投料态表单卡片；mode='run' 渲染思考态。

**Step 3:** `setCardState(step, state)` 切换 `data-state` + 内容区（input form / log area / artifact area）。

**Step 4:** 表单收集 `collectInputs()` 生成 POST body（复用现有字段逻辑：text/prompt/image_prompt/podcast_prompt/speaker_*/ref_images/disclaimer/size）。

**Commit:** `feat(web): step card three-state machine (input/thinking/done)`

---

## Task 5: 思考日志流式渲染

**Step 1:** `handleEvent` 新增 `case 'log'`：向对应卡片日志区 `appendLog(step, text)`，自动滚动到底，新行动画淡入。

**Step 2:** step running 时卡片切思考态并清空日志区；done 时切完成态。

**Step 3:** 移除 `typewriter()` 与 `finalizeEditable` 的旧调用路径；改写文案仅在 artifact 事件渲染（修复重复 bug）。

**Commit:** `feat(web): streaming thinking-log rendering per step`

---

## Task 6: 产物卡片交互

**Step 1:** `renderArtifact(step, name, url)` 按类型渲染：文案→可编辑 textarea+保存；图→缩略图点击开灯箱；音频/视频→内联播放器；srt→只读。

**Step 2:** 灯箱 lightbox 弹层组件（点图放大，点遮罩关闭）。

**Step 3:** 文案保存复用 `PUT /api/files/:id/:name`，保存成功 toast。

**Commit:** `feat(web): inline artifact cards with preview/edit/lightbox`

---

## Task 7: 侧边栏历史任务

**Step 1:** `loadTasks()` 渲染侧边栏列表项（状态圆点 + id + 当前步骤），点击 `loadTaskIntoTimeline(id)`。

**Step 2:** `loadTaskIntoTimeline`：`GET /api/tasks/:id` → 按 steps 状态设置卡片三态 → 渲染已有产物 → 若 running 则 `subscribe`。

**Step 3:** 任务终态时刷新侧边栏（`loadTasks`）。

**Commit:** `feat(web): sidebar history list + task replay into timeline`

---

## Task 8: 配置弹窗迁移 + 集成验证

**Step 1:** 配置/provider 弹窗逻辑保留，套新视觉类名。

**Step 2:** 全量回归：`cargo test` + `cargo clippy` + `cargo build --release` + `./serve.sh restart`。

**Step 3:** 端到端手测：新建任务→投料→全流程→日志流式→产物编辑保存→历史回放→主题切换→配置保存。

**Commit:** `feat(web): migrate config modal to new design; e2e verified`

---

## 验收标准

- 新建任务：主区出现四张投料卡片，每步可填可选 input。
- 执行：思考日志逐行流式滚动；产物紧跟对应步骤卡片出现。
- 改写文案只渲染一次（无重复 bug）。
- 产物可点击预览、文案可编辑保存、图片可灯箱放大。
- 历史任务在侧边栏可点击回放完整时间线。
- 视觉达到深色商用质感（Linear/Vercel 风），支持主题切换。
