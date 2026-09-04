# 移动端适配 + 产物下载 设计方案

日期：2026-09-04
状态：待 Review
关联版本：v0.2.0（计划并入下一版本）

## 背景

Media Factory 部署到服务器后，使用场景从"本机 localhost"变成了"外网远程访问"，暴露出两个问题：

1. **移动端不可用**：Web 界面（`web/index.html`）是桌面端布局——左侧固定 264px 任务栏 + 右侧主区，唯一的响应式规则是 `@media (max-width:900px)` 把侧栏缩到 210px。在 375px 宽的手机上，主区只剩 ~165px，无法正常浏览和操作。
2. **产物无法下载**：生成物（图片 / 播客 mp3 / 成品视频 mp4）目前只能通过 `/api/files/:id/:name` 内联预览（`Content-Disposition: inline`），界面上没有任何下载入口。本机使用时用户可以直接去 `output/<id>/` 目录取文件，但服务器部署后普通用户没有服务器文件系统权限，等于"产物看得见、拿不走"。

## 目标

- 手机（≤768px 宽度）上可完整完成"新建任务 → 跑流程 → 预览 → 下载"闭环
- 桌面端布局、交互零回归
- 每个产物可单独下载；整个任务的产物可一键打包下载
- 不引入前端框架 / 构建工具，保持单文件 `web/index.html` 的架构

非目标（本期不做）：

- 不改 CLI 行为；不为 CLI 增加下载命令（服务器场景的下载走 Web 即可）
- 不做 PWA / 离线缓存
- 不解决公网无鉴权问题（另行单独处理）

---

## 方案一：移动端适配

### 总体思路

保持现有 DOM 结构不动，通过「断点 CSS + 少量 JS 控制类名」实现响应式。新增 **768px 手机断点**，侧栏从"固定栏"变为"抽屉（drawer）"。

### 断点体系

| 断点 | 范围 | 布局 |
|---|---|---|
| 桌面 | >900px | 现状：侧栏 264px 固定 |
| 平板 | 481–900px | 现状：侧栏 210px 固定（已有规则，保留） |
| 手机 | ≤768px | **新增**：侧栏隐藏为抽屉，主区占满全宽 |

（481–768 与 768–900 有重叠，以 max-width:768px 的规则优先，用更具体的媒体查询顺序保证。）

### 手机端（≤768px）具体改动

**1. 侧栏 → 抽屉**

- `#sidebar` 改为 `position:fixed; left:0; top:0; bottom:0; width:min(280px, 84vw); transform:translateX(-100%); transition:transform .25s; z-index:60`
- body 加 `sidebar-open` 类时 `transform:none`
- 同时显示半透明遮罩 `#backdrop`（`position:fixed; inset:0; background:rgba(0,0,0,.45); z-index:50`），点遮罩收起抽屉
- 顶栏左侧新增汉堡按钮 `#btnMenu`（仅手机断点显示，桌面 `display:none`），点击切换 `sidebar-open`

**2. 顶栏适配**

- `#topbar` 允许 `flex-wrap:wrap`，`padding` 从 `14px 26px` 缩为 `12px 14px`
- `#status` 从 `max-width:40%` 放宽为占满一行（`flex-basis:100%`），避免挤压当前任务名

**3. 主内容区**

- `#scroll` padding 缩为 `14px 14px 80px`
- `#tlWrap` 去掉 `max-width:820px` 限制（手机下自然 100%）
- 步骤卡片 `.card-head` 允许换行；`.step-actions` 按钮组 `flex-wrap:wrap`，主操作按钮在手机上 `width:100%`（触控目标 ≥44px 高）
- 时间线竖线和圆点（`.step-row::before` / `.tl-dot`）左移收紧（`left:4px` / `left:-2px`），卡片左边距同步缩小，把空间让给内容

**4. 触控目标与 hover 依赖修复（重点）**

现有代码中任务项删除按钮 `.tdel` 是 `opacity:0`、仅 `.task-item:hover` 时显示——**触屏没有 hover，手机用户永远无法删除任务**。手机断点下改为常显（`opacity:.6`）。同理排查所有 hover-only 交互（清空按钮 `.clearall` 已常显，无此问题）。

**5. 产物预览**

- `video / audio / img.prev` 已是 `width:100%`，无需改动
- 灯箱 `#lightbox` 图片加 `max-width:94vw; max-height:80vh`
- 文本产物 `textarea` 的 `rows` 在手机上适当减少（9→7）

**6. 配置弹窗（⚙ 面板）**

- 手机断点下从居中弹窗改为底部全宽 sheet：`width:100%; max-height:88vh; border-radius:14px 14px 0 0; align-self:flex-end`
- 表单行（BaseURL / Key / 模型选择）纵向堆叠

### JS 改动（很小）

```js
// 顶栏汉堡按钮
btnMenu.onclick = () => document.body.classList.toggle('sidebar-open');
// 遮罩关闭 + 手机上点选任务后自动收起抽屉
backdrop.onclick = closeSidebar;
// selectTask() 末尾：if (window.innerWidth <= 768) closeSidebar();
```

### 验证

- Chrome DevTools 设备模拟：iPhone SE（375×667）、iPhone 14 Pro Max（430×932）、iPad（768×1024）、桌面 1440px
- 真机验证一次（部署到服务器后手机浏览器直接访问）
- 检查清单：新建任务 / 填文案 / 跑全流程 / 展开折叠步骤卡 / 编辑保存文本产物 / 预览图音视 / 删除任务 / 配置面板保存

---

## 方案二：产物下载

### 现状

- 服务端已有 `GET /api/files/:id/:name`，响应头 `Content-Disposition: inline`（为内联播放服务），按扩展名设置 Content-Type，有路径穿越防护（拒绝 `..` `/` `\`）
- 前端产物渲染时只显示 `文件名 + 内联预览`，无下载按钮；`Artifact` 结构里已有 `size` 字段但界面未展示

### 设计

**1. 单文件下载（复用现有端点）**

服务端：`/api/files/:id/:name` 增加可选查询参数 `?download=1`——存在时响应头改为 `Content-Disposition: attachment; filename="..."`，其余逻辑（Content-Type、路径校验）不变。

前端：每个产物标题行（`.cap`）右侧追加小型下载链接：

```html
<span class="cap">成品视频 <span class="fname">video.mp4 · 12.4 MB</span>
  <a class="dl" href="/api/files/<id>/video.mp4?download=1" download>⬇ 下载</a>
</span>
```

- `size` 字段格式化展示（B/KB/MB），用户下载前对体积有预期（手机上尤其重要）
- `<a download>` + `attachment` 双保险：iOS Safari 对 inline 的 mp4/mp3 会强行打开播放器，`attachment` 可让它走"下载到文件"流程

**2. 整包下载（新增端点）**

`GET /api/tasks/:id/archive` —— 把 `output/<id>/` 下除 `task.json` 外的所有产物打成 zip 流式返回：

- 响应：`Content-Type: application/zip`，`Content-Disposition: attachment; filename="media-factory-<id前8位>.zip"`
- 实现：引入 `zip` crate（`zip = { version = "2", default-features = false, features = ["deflate"] }`）
  - mp3/mp4/png 本身已是压缩格式，zip 内用 `Stored`（不二次压缩，省 CPU）；md/srt/txt 用 `Deflated`
  - 复用 `list_artifacts()` 的过滤逻辑（排除 task.json）；复用文件名安全校验
- 不缓存 zip 文件到磁盘，直接写入响应 body（产物量小，单任务通常 <100MB，内存可接受；如后续有大文件需求再改流式）

前端入口两处：

- 顶栏当前任务区域加「⬇ 打包下载」按钮（选中任务且存在产物时可用）
- 「视频」步骤卡完成后，在视频产物下方显示同样的整包下载按钮（成品交付的自然位置）

**3. 下载链接的可分享性**

下载 URL 是稳定的 GET 地址（如 `http://<IP>:8092/api/files/<id>/video.mp4?download=1`），用户可以长按复制链接发给他人或粘贴到下载工具。界面上下载按钮同时支持右键/长按"复制链接"，无需额外开发。

### 工作量估计

| 模块 | 改动 | 估计 |
|---|---|---|
| 移动端 CSS | 新断点 ~80 行 + 既有规则微调 | 0.5 天 |
| 抽屉/遮罩 JS | ~20 行 | 含上 |
| 单文件下载 | server.rs ~5 行 + 前端渲染 ~10 行 | 0.5 天 |
| 整包下载 | zip 依赖 + archive handler ~50 行 + 前端按钮 ~15 行 | 含上 |
| 真机回归 | 两端各过一遍检查清单 | 0.5 天 |

合计约 1 人天。

### 风险与注意

1. **iOS Safari 差异**：mp4 即使有 `attachment`，部分 iOS 版本仍可能进播放器再"存储到文件"。属平台行为，文案上引导"若自动播放，可用分享按钮存储到文件"。
2. **大文件内存**：现有 download 是整文件读入内存，archive 同理。当前产物体量（几十 MB）无风险；后续若支持长视频再改流式。
3. **zip crate 依赖**：首次引入，会增加编译体积约 200KB，无系统依赖（纯 Rust），不影响四平台交叉编译。
4. **桌面端回归**：所有新 CSS 都限定在 `@media (max-width:768px)` 内，下载按钮是纯新增元素，理论上桌面端零影响，仍需过一遍桌面检查清单。

## 实施顺序建议

1. 先做方案二（下载）——改动小、价值直接（服务器用户当前完全拿不到产物）
2. 再做方案一（移动端）——纯前端，完成后连同下载按钮一起做真机验证
3. 合并后 bump v0.2.1，走 tag → CI → 镜像同步的发版流程
