# 桌面版交互范式对照（desktop parity spec）

> **这份文档的用途**：把"对标 Codex 桌面版 / ZCode 桌面版"从**主观印象**变成**可逐条核对的事实**。
> TUI 侧曾有同样的问题（靠用户逐个截图报问题），靠 [`opencode-parity.md`](opencode-parity.md) 止住了；
> 桌面侧现在照同一条路子做。
>
> ⚠️ **换机器注意**：ZCode 的证据来自本机安装包
> （`/Volumes/data1/work/office-applications/ZCode.app`，**不在仓库里**），
> 不在那台机器上时，本文的 asar 引用复核不了。Codex / Zed 的证据来自公开仓库与文档。

---

## 0. 先说三个影响全局的事实（都经核查）

### 事实 1：**两个对标目标都是「Web 技术的 GUI + 协议后端」，不是 Rust 原生 GUI**

| 目标 | GUI 技术 | 证据 |
|---|---|---|
| **ZCode 桌面** | **Electron + React 19 + Radix UI + Lexical + node-pty** | 本机 `dev.zcode.app` v3.11.2，`app.asar` 内 `NSPrincipalClass = AtomApplication`（Electron）；`@zcode/client`/`@zcode/ui`/`@zcode/rpc`/`@zcode/server` 分层 |
| **Codex 桌面** | 闭源，**不在 `openai/codex` 仓库内** | `codex-rs` 的 `Cargo.lock` **全文无任何 GUI 依赖**（grep `egui`/`gpui`/`tauri`/`wry`/`iced`/`slint`/`winit`/`wgpu` 全部为空）；仓库里只有 TUI（`ratatui`）+ `cli/src/desktop_app/`（**安装器与 `codex://` 深链**）+ `app-server*`（RPC 后端） |
| **Zed**（作为"Rust 原生能到什么水准"的存在性证明） | **GPUI（纯 Rust GPU 框架）** | 本机装有 `dev.zed.Zed`；Zed 的 UI 全用自研 GPUI |

**含义**：
- "对标 Codex 桌面版"**只能对标 UX 与功能**，不能对标代码——它的 GUI 是闭源的，仓库里没有可参考的 GUI 实现。
- 两个目标的实际架构都是 **GUI 消费一个协议**（Codex：GUI ← `app-server` RPC；ZCode：renderer ← `@zcode/rpc` + `@zcode/server`）。
- **本项目已经就是这个形态**：内核（L2）+ `Op`/`EventMsg` 事件协议 + `neo-host-web` 的 HTTP/SSE 服务。也就是说我们的架构层次与两个对标目标同构，差别只在"谁来渲染"。

### 事实 2：Codex 的开放部分是 TUI，不是 GUI

`codex-rs/tui/`（包名 `codex-tui`）的结构可作为**信息层次**的参考：
`app.rs` · `chatwidget/` · `bottom_pane/` · `status/` · `diff_render.rs` · `history_cell/` · `keymap/` · `onboarding/` · `theme_picker.rs` · `worktree_browser.rs` · `file_search.rs` · `footer_hint.rs`。

**其中 `diff_render` / `history_cell` / `footer_hint` / `worktree_browser` 这几个分区，正是桌面版也应该有的**——这条比任何截图都可靠。

### 事实 3：ZCode 的桌面 UI 有官方逐项文档（对标的最佳依据）

ZCode 官方文档把界面写得很细（[zcode.z.ai/en/docs](https://zcode.z.ai/en/docs)），
包括面板划分、快捷键、Goal 模式、审批语义。**这是本次对标中最具体、可逐条核对的来源。**

---

## 1. ZCode 桌面版的界面契约（官方文档 + 本机 asar 双证据）

### 1.1 整体骨架（六个区域）

| 区域 | 内容 | 快捷键 |
|---|---|---|
| **左侧栏** | 顶部 `New Task / Search / Skills`；任务按**工作区分组**；每行显示标题、相对时间、运行/未读/失败**状态圆点**、该任务的 **`+/-` 行数变化**；视图可切 `Grouped / Workspace / Timeline`；可搜索、可进 Archive | `Cmd/Ctrl+B` |
| **中央转录区** | "思考轨迹"（**可搜索、默认展开**）、工具调用**分组**、每轮结束的**执行摘要与耗时**；user 消息可 hover 出铅笔进 **Edit History**（仅最后一轮） | — |
| **右侧面板** | **summary panel**：Goal 模式的目标卡片、耗时、按 iteration 分组的 checklist、当前轮进度 | `Cmd/Ctrl+Alt+B` |
| **终端面板** | 内置 terminal（`node-pty`），显示 `zsh user@host` + Git 信息 | `Cmd/Ctrl+J` |
| **命令中心** | 覆盖式面板：commands / conversations / files 三类（新建 chat、打开文件夹、搜文件、设置、上下一个 chat、find、切主题、MCP、toggle terminal…） | `Cmd/Ctrl+K` |
| **内置浏览器** | desktop-only；可开本地 dev server / 线上页面，Agent 可驱动点击/表单/滚动/截图 | — |

另有**文件树面板 + Repo Wiki**（hover 工作区项出现 file-tree 按钮；文件树顶部有 Repo wiki 入口，"takes over the main work area"）。

### 1.2 审批（安全确认）的语义——**这是最该照抄的一段**

来源：[docs/safety-confirm](https://zcode.z.ai/en/docs/safety-confirm)

1. 权限门触发 → **当前任务暂停**，且 **composer 被阻塞**（防止误把下一步排进队列）。
2. 显示 Agent **将要执行的确切内容**（命令 / 文件改动 / 工具动作）。
3. 选项三档：**Allow / Always Allow / Reject**。
4. **高风险或全自动模式下，工具栏持续显示风险状态**（不是只在弹窗里提示）。
5. 审批请求**按任务作用域绑定**：切走再回来仍在；侧栏可显示"等待确认"。

> 本机 asar 里 `out/renderer/cua-permission-panel.html` 是一个**独立窗口**（Computer Use 权限面板），
> 其 CSS 注释直接引用 Codex 的实现（"codex 用的是静态 SF Symbol arrow.up"）——
> **说明 ZCode 自己也把 Codex 桌面版当参照物**。

### 1.3 Composer（输入区）与模式

- `@` 引用文件、"添加上下文"、"**变更前确认**"开关、模型选择、发送。
- 模型 picker `Ctrl/Cmd+M`；**Thought Level picker `Ctrl/Cmd+T`**；执行模式循环 **`Shift+Tab`**（输入框聚焦时）。
- 四种执行模式：Ask before changes（默认）/ Edit automatically / Plan / Full access。

> 对照本项目：这四档与我们内核的 `ExecMode` 五档（plan/confirm/default/auto-edit/full）**是同一类东西**，
> 可直接做映射，不需要发明新概念。

---

## 2. Codex 桌面版能对标的部分（受限于闭源）

| 能对标的 | 依据 |
|---|---|
| **信息分区**：diff 渲染、历史单元、底部键位提示、worktree 浏览器 | `codex-rs/tui/src/` 的实际模块划分 |
| **深链契约**：`codex://threads/new?path=<workspace>`；mac 端校验 `com.openai.codex` 签名 | `codex-rs/cli/src/desktop_app/{mac,windows}.rs` |
| **协议后端的形态**：`app-server` 系列 + `app-server-protocol` 用 `ts-rs` 生成 **TypeScript 类型**给 GUI 消费 | `codex-rs/app-server-protocol/schema/typescript/` |
| 有 onboarding 概念（`DesktopOnboardingEntrypoint`） | `app-server-protocol` schema |

**不能对标的**：面板布局、视觉、交互细节——文档 403，无公开截图。**不编造。**

**一个值得借鉴的架构点**：Codex 把"给 GUI 用的协议"单独做成 `app-server` 层并用 `ts-rs` 生成类型，
使 GUI 与 TUI 消费同一套契约。**我们的 `Op`/`EventMsg` 已经是这个角色**，且更强（有 golden 回放与 T6 等价断言）。

---

## 3. 差距清单（我们 vs 对标目标）

现状基线：桌面实际加载的是 `neo-host-web` 的 **184 行**内联页面（纯文本日志 + `y`/`n` 审批 + goal 栏）。

| # | 项 | 对标目标 | 我们 | 数据在事件流里吗 |
|---|---|---|---|---|
| D1 | 左侧任务/工作区栏（状态、`+/-` 行数、分组） | ZCode | ❌ 无 | 部分（`FileChanged`/会话列表可支撑） |
| D2 | 转录区 Markdown 渲染 | 两者 | ❌ 纯 `textContent` | ✅ `AgentMessageDelta` |
| D3 | **思考轨迹**（可折叠、可搜索） | ZCode | ❌ 未处理 | ✅ `ReasoningDelta`（**已流着但被扔掉**） |
| D4 | 工具调用**分组** + 参数摘要 | 两者 | 只显示名字 | ✅ `ToolCallBegin{name,arguments}` |
| D5 | **diff 渲染** | 两者 | ❌ 未处理 | ✅ `PatchProposed{path,diff}`（**内核审批前已生成**） |
| D6 | 每轮**执行摘要 + 耗时** | ZCode | 只有 token 数 | ✅ `TurnComplete`（需补耗时字段或前端计时） |
| D7 | 右侧 summary / Goal 面板 | ZCode | 纯文本摘要 | ✅ `GoalUpdated{snapshot}` |
| D8 | 底部终端面板 | ZCode | ❌ 无 | 需新功能（`Op::Shell` 已存在） |
| D9 | 命令中心（`Cmd+K`） | ZCode | ❌ 无 | 纯前端 |
| D10 | 审批：**阻塞 composer** + 三档 + 风险常驻 | ZCode | 只接受 `y`/`n` | 部分（`ApprovalRequest`；composer 阻塞是前端行为） |
| D11 | 执行模式切换 | ZCode | 启动时定死 | 需新端点（运行时切换） |
| D12 | 文件树 / 内置浏览器 / Repo Wiki | ZCode | ❌ 无 | 需新能力（非本阶段） |

**结论：D2–D7 全部是纯前端工作（数据已在事件流里），零后端改动。** 这是投入产出比最高的一段。

---

## 4. 落地顺序（按"影响面 × 被复用度"排）

1. **D5 + D3 + D4**（diff / 思考轨迹 / 工具卡片）——三个最影响"这像不像专业工具"的，且**数据现成**。
2. **D2 + D6 + D7**（Markdown / 轮摘要 / Goal 面板）——补齐信息层次。
3. **D10**（审批三段式 + 阻塞 composer）——安全交互，照 ZCode 语义。
4. **D1 + D9**（任务栏 / 命令中心）——工作区管理，需会话库支持。
5. **D8 + D11 + D12**（终端 / 模式切换 / 文件树）——需要新后端能力，放最后。

> **纪律**（沿用 TUI parity 的教训）：新界面必须复用同一套视觉原语（面板/选项条/对话框），
> **不得每个页面各长一副样子**——那正是"打地鼠"的温床。

---

## 5. 复核方式

| 来源 | 位置 |
|---|---|
| ZCode 官方文档 | `https://zcode.z.ai/en/docs`（install / ADE-tools / keyboard-shortcuts / goal / safety-confirm / browser-use / repo-wiki / task-management）· `https://zcode.z.ai/changelog` |
| ZCode 本机安装（Electron 栈的硬证据） | `/Volumes/data1/work/office-applications/ZCode.app` → `Contents/Resources/app.asar`（`Info.plist` 的 `NSPrincipalClass`、`out/renderer/*.html` 三个窗口入口、`node_modules` 里的 Radix/Lexical/node-pty） |
| Codex 公开仓库 | `github.com/openai/codex` → `codex-rs/tui/src/`、`codex-rs/cli/src/desktop_app/`、`codex-rs/app-server-protocol/`；`codex-rs/Cargo.lock` 无 GUI 依赖 |
| Zed / GPUI | 本机 `dev.zed.Zed`；`github.com/zed-industries/zed` 的 `crates/gpui`（Apache-2.0） |

**待补**：ZCode / Codex 桌面的**真实界面截图**。
当前无法取得——本机 ZCode 的辅助功能与屏幕录制权限未授予（详见下面"权限"一节），
Codex 桌面文档 403。**需要真人补图**，否则视觉层只能对标到"结构"而到不了"观感"。

### 权限（若要让 Agent 直接看这两个界面）

本机 `dev.zcode.app` 正在运行，但截图与 AX 读取被系统拒绝：

```
Accessibility not granted to ZCode.app
Screen Recording is denied for ZCode
→ 授权对象：/Users/kags/.zcode/computer-use/ZCode Computer Use.app
   在「系统设置 → 隐私与安全性」里给辅助功能 + 屏幕录制授权，然后完全退出并重开 ZCode
```

授好之后，桌面版可以直接对着真实界面做视觉对照（TUI 那轮就是这么做的，效果很好）。
