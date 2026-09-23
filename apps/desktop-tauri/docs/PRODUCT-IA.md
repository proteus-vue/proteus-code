# NEO Desktop · 产品形态真源（IA）

> **唯一依据**（全部本地，未编造）：
>
> | 来源 | 路径 | 用途 |
> | --- | --- | --- |
> | Codex 安装包 | `/Volumes/data1/work/office-applications/Codex.app` | `app.asar` webview CSS/JS 结构量 |
> | ZCode 安装包 | `…/ZCode.app` | 安装布局、`macos-window-bounds` |
> | MiMo 安装包 | `…/Xiaomi MiMo.app` | Electron 版本、`app.asar` 结构 |
> | MiMo 解包逆向 | `/Volumes/data1/work/WorkBuddy/2026-09-22-10-34-07/MiMo-逆向工程报告.md` | 312 IPC、快捷键、preload 桥 |
> | MiMo 渲染 chunk 名 | `…/mimo-unpacked/…/out/renderer/assets/*` | 右栏 `Rp*` 模块、侧栏/输入组件 |
> | Codex webview CSS | `…/codex-unpacked/webview/assets/app-initial-*.css` | toolbar/tab/panel/sidebar 令牌 |
> | 三家规格包 | `…/codex-style-agent-desktop-spec/{01–08}` | 实测尺寸/组件/护栏 |
> | 本仓 parity | `docs/desktop-parity.md` | D1–D12 与内核对应关系 |
>
> **改 UI 前先改本文；禁止只凭单张截图打补丁。**
> 实现时**自己写代码**，只对齐尺寸/交互/结构，不拷贝对方源码。

---

## 0. 一句话产品形态

**聊天优先的三区 Agent 桌面壳**（Electron/Tauri 均可；**不是** VS Code 活动栏 IDE）：

```
┌─ Sidebar 256–275 ─┬─ Center（对话主舞台）──────────┬─ Panel ≥320（可选）─┐
│ 项目 / 会话 / 搜索 │  无大段状态芯片顶栏              │ 浏览器式标签 + 内容  │
│ 可折叠 → 图标轨    │  Message stream 顶到底          │ 审查/终端/文件/…    │
│                    │  Composer 贴底（模式/模型/发送）  │ + 打开标签页        │
└────────────────────┴───────────────────────────────┴─────────────────────┘
```

规格 `03.1` 原文骨架 = Sidebar | Toolbar+Stream+Composer | Panel。  
**本项目对 Toolbar 的调和**（用户反馈 + 截图）：中栏**不显示** branch/model/状态芯片条；模型与模式**只在 Composer**；全局入口走 `⌘K/⌘,`。

---

## 1. 进程与技术（三家对照 + 本项目）

| 层 | Codex `[C]` | ZCode `[Z]` | MiMo `[M]` | NEO Desktop |
| --- | --- | --- | --- | --- |
| 外壳 | Electron + 闭源 GUI | Electron | Electron **41.7.2** + electron-vite | **Tauri 2**（有意不同） |
| 引擎 | 独立 Rust `codex` 二进制 | 内嵌/子进程 | 主进程 `node.mjs` 36MB（OpenCode fork） | **`neo app-server` 子进程** stdio JSON-RPC |
| 前端 | 闭源 webview | React 19 + shadcn | React 19 + Tailwind + 自研组件 | React 18 + 自研 CSS 令牌 |
| 终端 | — | node-pty | `@lydell/node-pty` + xterm | **命令台** `command/exec`（非 PTY，诚实标注） |

**架构判定**：三家都是 **GUI 消费一个协议**（Codex: app-server；ZCode: rpc；MiMo: IPC bridge）。  
NEO 已是同构形态，差的主要是**壳的完成度与观感**，不是再重写内核。

合规：只对齐结构/尺寸/交互，**不复制**对方 CSS/JS/图标。

---

## 2. 窗口与三区布局（实测量）

| token | 实测来源 | 值 | 本项目 |
| --- | --- | --- | --- |
| Toolbar 高 | `[C] --height-toolbar` | **46px**（另有 36/40 变体） | 中栏**无芯片条**；若将来加，用 46 |
| Sidebar 宽 | `[C] --codex-sidebar-preferred-width ≈275px`；`[M]` 256（62+194） | **256–275** | `--sidebar-width: 256`，折叠 48 轨 |
| Panel 宽 | 规格 ≥320；可拖 | **≥320** | `min(380px, 36vw)` |
| 消息流 max | `[C]` 40rem | **40rem** | `--content-max-width: 40rem` |
| 对话字 / UI 字 | `[C]` 16 / 14 | **16 / 14** | 已分 |
| Tab 条 | `[C] --app-shell-tab-background`；规格 Panel 顶 | 标签条 + 内容 | **浏览器式标签**（× / + 下拉） |

### 2.1 Sidebar（规格 04.1 + MiMo `sidebar-*.js`）

**必须有**：
- 新建任务（黑底主按钮，对齐截图）
- 项目/仓库分组（目录名）
- 会话列表：标题、相对时间、状态点、`+/-` 行数
- 搜索过滤
- 折叠 → **48–62px 图标轨**（☰ / ＋ / ⌕）

**不需要**：与窗口标题重复的大品牌块（可极简一行）。

### 2.2 Center（对齐你「去掉顶栏」的反馈）

| 有 | 没有 |
| --- | --- |
| 对话流从顶到输入区 | branch / model / mode / 已连接 / 重连 **芯片条** |
| Composer：模式 chip · 模型 · 发送 · @ 补全 · ⌘K | 中栏第二套全局工具栏 |
| 空态：Hero + 示例任务 + 快捷键提示 | 打印 28 个 RPC 方法 |

### 2.3 Panel = 工作台坞（**不是** VS Code 竖活动栏）

依据：MiMo renderer chunk **`RpReview / RpTerminal / RpFiles / RpBrowser / RpChat / RpSimulator / RpSubagents / RpAutomation / RpHtml / RpOffice`** + 你截图的「打开标签页」。

| 交互 | 规格 |
| --- | --- |
| 打开 | 工具栏/`⌘J` 或标签 **+** 下拉「打开标签页」 |
| 标签 | 浏览器式：`[标签 ×]…[+]`，可多开，激活高亮 |
| 内容 | **一屏一个 active**，满高 `wb-body` 滚动，**不卡片堆叠** |
| 关闭 | 标签 ×；全关则收起面板 |

**右栏标签集合（P0–P2）**：

| 标签 | 对标 | 内容 | 后端 |
| --- | --- | --- | --- |
| 审查 | RpReview | 改动文件 + Diff | `files_changed` / `patch_proposed` |
| 终端 | RpTerminal | `$` 命令台 | `command/exec`（非 PTY） |
| 浏览器 | RpBrowser | 本地预览 + Wiki | fs/wiki |
| 文件 | RpFiles | 树 + 预览 + @引用 | `list_workspace` |
| 侧边聊天 | RpChat | 会话摘要 + 审批 | 同线事件 |
| 模拟器 | RpSimulator | 占位/连接态 | `session/configure` |
| Goal | 规格 D7 | 目标进度 | `goal/*` |
| 子智能体 | RpSubagents | `tools/list` 过滤 `agent_*` · 主对话调用 | — |
| Automations | RpAutomation | 壳侧 localStorage 排程 + 到点 `turn/start` | — |

**明确不做**：右侧常驻 7 图标 VS Code 活动栏；GOAL/连接卡片罗汉。

---

## 3. 消息流（规格 04.3 + MiMo `ThreadView`/`HarnessRow`）

| 元素 | 要求 |
| --- | --- |
| 用户消息 | **右对齐紧凑气泡**；hover 可编辑最后一轮 |
| 助手 | 左满宽 Markdown（16px）；默认无边框正文感 |
| 工具 | **默认折叠**成一行「已运行 N 条命令 / 工具名 + 摘要」；展开完整 IO |
| 思考 | **默认折叠**；可搜索 |
| 轮摘要 | `已处理 38s · +tok/−tok`（非居中虚线胶囊） |
| 审批 | 展示确切内容 + 三档；**阻塞 Composer** |
| Diff | 行着色；可全屏并排（`v`） |

原则 **04.8**：**默认折叠，按需展开**——信息密度是核心体验。

---

## 4. Composer（规格 04.5 + 截图）

```
[附件/技能 chips 可选]
[多行输入 …]
[模式 chip │ 提示 │ 模型 │ ●发送]
```

- Enter 发送 · Shift+Enter 换行 · Esc 中断
- `@` 文件补全（协议 `@ref`）
- 模式与「完全访问」风险态可见
- 生成中：停止 / 按钮变停止

---

## 5. 快捷键（规格 03.4 + MiMo 35 条中必须集）

| 键 | 动作 | 本项目 |
| --- | --- | --- |
| ⌘N | 新建会话 | ✅ |
| ⌘K / ⌘P | 命令面板 | ✅ |
| ⌘B | 侧栏 | ✅ |
| ⌘J | 右栏 | ✅ |
| ⌘L | 聚焦输入 | ✅ |
| ⇧Tab | 循环权限模式 | ✅ |
| Esc | 中断 | ✅ |
| ⌘, / ⌘⇧K / ⌘F | 设置 / 清空 / 查找 | ⌘, ✅ · 其余 ⬜ |

---

## 6. 能力地图（内核 ↔ 壳）

壳消费 **app-server 29 方法**；护栏十条已在内核（#1–#10）。  
壳负责：布局、事件渲染、审批 UI、标签坞、键盘。

| 壳能力 | 协议/事件 |
| --- | --- |
| 会话列表 | `thread/list` + `updated_ms` · 改名 `thread/rename` · 删 `thread/delete` · 内容搜 `thread/history` |
| 切换历史 | `thread/resume` 广播 EventMsg |
| 发送/中断 | `turn/start` · `turn/interrupt` |
| 审批 | `approval_request` → `approval/respond` |
| 模型/模式 | `models/list` · `session/configure` |
| 目标 | `goal/set|pause|resume|clear` + `goal_updated` |
| 命令台 | `command/exec` |
| 文件树 | Tauri `list_workspace` / `read_workspace_file` |
| Wiki | Tauri `list_repo_wiki`（敏感整篇排除） |

**刻意不做（对齐三家但本仓边界）**：账号/OTA 遥测红线（规格 07）；TCP app-server；多会话并行内核。

---

## 7. 视觉硬规则（规格 03.7 + 本仓设计系统）

1. 暗底不用 `#000`（`#0a0a0a`–`#181818`）
2. 对话 **16px** / UI **14px**
3. **仅一处高饱和**强调色（`#ff6a2b`）
4. 代码区 VS Code Dark+ 思路，不自创一套
5. 圆角一个 scale 变量
6. **图标**：自绘线性 SVG（`src/components/Icon.tsx`），24 viewBox / 1.75 stroke / round cap；**禁止** emoji 字符、系统符号（☰⚙▦…）当图标；**不拷贝**三家图标文件
7. **控件**：选择器/下拉用自绘 `Select`（或 button+menu），**禁止**裸原生 `<select>` 出现在产品 UI；输入/按钮走同一 border/radius/height 令牌

默认**浅色**（你截图的竞品主态）；`data-theme=dark` 可切。

### 7.1 控件尺寸（自绘 Select / Icon 按钮）

| token | 值 |
| --- | --- |
| 控件高 | 32px（紧凑 28px） |
| 图标按钮 | 28×28 · 线宽 1.75 |
| 下拉菜单 | 与 palette 同 elevated 面 + shadow-pop |

---

## 8. 阶段与验收（挂接规格 08）

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P0 | 三区骨架、对接 app-server、事件流、审批、会话 | ✅ |
| P1 | 浏览器标签坞、文件树、Wiki、@、编辑重发、密度 | ✅ 主体 |
| P1 余量 | 会话内容搜索/改名/删除、⌘, 设置 | ✅ 主体；图片附件 ⬜ |
| P2 | 真 PTY、图片附件 | ⬜ · 斜杠/拖拽/命令台/子代理/Automations 已做 |
| P3+ | 插件市场、Worktree、分享（默认关） | ⬜ |

**回归**：`npm run build` + 本文 §9 表勾选 + 内核护栏测试仍绿。

---

## 9. 差距清单（改 UI 用这张）

| ID | 差距 | P | 状态 |
| --- | --- | --- | --- |
| IA-1 | 中栏状态芯片条 | P0 | ✅ 已删 |
| IA-2 | 浏览器标签 + + 下拉 | P0 | ✅ |
| IA-3 | 下拉不裁切 / 外点关 | P0 | ✅ |
| IA-4 | 去 VS Code 竖活动栏 | P0 | ✅ |
| IA-5 | Panel 满高非卡片堆 | P1 | ✅ 结构；可再铺满 |
| IA-6 | 侧栏图标折叠 | P1 | ✅ |
| IA-7 | 工具/思考默认折叠 | P0 | ✅ |
| IA-8 | Composer @/模式/模型 | P0 | ✅ |
| IA-9 | 真 PTY / xterm | P2 | 🔄 命令台回显 stdout/stderr；真 PTY 仍 ⬜ |
| IA-10 | 图片附件、斜杠命令 | P2 | ✅ 斜杠壳侧（11 条）；图片 ⬜ 待 L0 多模态 |
| IA-11 | 分栏拖拽 persist | P2 | ✅ 侧栏/右栏 pointer 拖拽 + localStorage |
| IA-12 | 会话内容搜索、重命名/删除 | P1 | ✅ history 内容搜 · rename · delete（当前会话自动切走再删） |
| IA-13 | 设置页 ⌘, | P1 | ✅ 外观/模型/模式/快捷键 |
| IA-14 | 子智能体 / Automations 标签 | P2–3 | ✅ 子代理 `agent_*` · Automations 壳侧排程 |
| IA-15 | 图标布局重做 + 自绘图标集 + 去原生控件 | P1 | ✅ Icon · Select（含贴底向上翻） |

---

## 10. 变更纪律

1. 需求 → 先改 **§9 表** → 再改代码。  
2. 禁止单张截图直接改布局而不回填本文。  
3. 合规：不拷贝三家源码/图标；只抄**结构与量**。  
4. 回归：`apps/desktop-tauri` 的 `npm run build` + 内核门禁（若动协议）。
