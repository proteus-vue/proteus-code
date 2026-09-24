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
- 新建任务（黑底主按钮，对齐截图）— **仅顶栏一处**，禁止再叠第二行「新建/搜索」
- 项目/仓库分组（目录名）— **可点开项目切换**：最近工作区列表 + 添加路径（会话库随 `workspace/.neo/sessions` 整组切换）
- 会话列表：**分区标题「会话」**；**单行纯标题**（对齐 Codex/MiMo）；未选中无边框无副行；选中浅灰底；悬停仅右侧纯图标。状态点/±行数/条数副行**不默认展示**（可进 title 提示）。**标题截断时悬停自动横向滚动**展示全文
- 会话管理入口：新建（顶栏）· 双击改名 · 悬停删除（均已有，分区展示）
- 搜索过滤 — **不设侧栏常驻搜索框**；全局用 ⌘K 命令面板（与 Codex 一致）
- 折叠 → **48–62px 图标轨**（☰ / ＋ / ⌕）

**不需要**：与窗口标题重复的大品牌块（可极简一行）；顶栏下再放工具双列。

### 2.2 Center（对齐你「去掉顶栏」的反馈）

| 有 | 没有 |
| --- | --- |
| 对话流从顶到输入区 | branch / model / mode / 已连接 / 重连 **顶栏芯片条** |
| **新建任务态**（无消息）：居中 Hero「接下来交给我吧」· Composer 上方 **项目/分支 chips** · 下方建议条 | **对话态输入框上方残留任何选择器/chips/建议条** |
| **任务开始选项目**：`projectMode = workspace \| none`；**入口 = Composer 上方项目芯片下拉**（ZCode：最近项目 / 打开文件夹 / **不在项目中工作**），不塞侧栏当唯一入口；none 用 `~/.neo/no-project`；**真重启 app-server 换目录** | 只画 chips 不换 workspace · 主入口放侧栏 |
| **对话态**：底部紧凑 Composer；生成中占位「继续输入以排队后续修改」 | 第二套全局工具栏 |
| Composer 内：模式 chip · 模型 · 发送 · @ / ⌘K | 打印 28 个 RPC 方法 |

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
| 浏览器 | RpBrowser | 地址栏 + 预览 + **点选元素入对话** + Wiki | fs/wiki · open_url · fetch_url |
| 文件 | RpFiles | 树 + 预览 + @引用 | `list_workspace` |
| 侧边聊天 | RpChat | 会话摘要 + 审批 | 同线事件 |
| 模拟器 | RpSimulator | 占位/连接态 | `session/configure` |
| Goal | 规格 D7 | 目标进度 | `goal/*` |
| 子智能体 | RpSubagents | `tools/list` 过滤 `agent_*` · 主对话调用 | — |
| Automations | RpAutomation | 壳侧 localStorage 排程 + 到点 `turn/start` | — |

**明确不做**：右侧常驻 7 图标 VS Code 活动栏；GOAL/连接卡片罗汉。

---

## 3. 消息流（规格 04.3 + MiMo `ThreadView`/`HarnessRow` + ZCode 连续阅读）

| 元素 | 要求 |
| --- | --- |
| 整体 | **连续文档流**（非一叠卡片）：无角色大标题、无助手描边框；垂直节奏统一 |
| 用户消息 | **右对齐中性灰紧凑气泡**（非高饱和橙）；hover 可编辑最后一轮 |
| 助手 | 左满宽 Markdown（16px）；**无边框无底色**；默认不显示 NEO 角色条（hover 可复制） |
| 工具 | **默认折叠成一行**弱化元数据（如「查阅 · 3 搜索」「编辑 path +46」）；点开才展开 IO；**不要** timeline 卡片壳 |
| 思考 | **默认折叠**；可搜索 |
| 轮摘要 | `已处理 38s · +tok/−tok`（左对齐细字，非胶囊） |
| 审批 | 展示确切内容 + 三档；**阻塞 Composer** |
| Diff | 行着色；可全屏并排（`v`）；列表里可先收成一行路径+增删 |

原则 **04.8**：**默认折叠，按需展开**——信息密度是核心体验；阅读时像一份连续转录，不是 UI 组件堆。

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
| P3+ | 插件市场、Worktree、分享（默认关） | ✅ 插件市场接设置页（本地 marketplace/* · plugin/*）；Worktree ⬜ · 分享默认关 |

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
| IA-9 | 真 PTY / xterm | P2 | ✅ 内核 SessionManager（portable-pty + 沙箱包装）· `command/exec?session` + write/resize/terminate；桌面 xterm 接线 ⬜ |
| IA-10 | 图片附件、斜杠命令 | P2 | ✅ 斜杠壳侧（11 条）；图片 ⬜ 待 L0 多模态 |
| IA-11 | 分栏拖拽 persist | P2 | ✅ 侧栏/右栏 pointer 拖拽 + localStorage |
| IA-12 | 会话内容搜索、重命名/删除 | P1 | ✅ history 内容搜 · rename · delete（当前会话自动切走再删） |
| IA-13 | 设置页 ⌘, | P1 | ✅ 外观/模型/模式/快捷键 |
| IA-14 | 子智能体 / Automations 标签 | P2–3 | ✅ 子代理 `agent_*` · Automations 壳侧排程 |
| IA-15 | 图标布局重做 + 自绘图标集 + 去原生控件 | P1 | ✅ Icon · Select（含贴底向上翻） |
| IA-16 | 侧栏视觉重做（去重复入口/层级） | P1 | ✅ 顶栏一行 · 无双份工具 · 瘦搜索 · 行内确认 |
| IA-17 | 会话列表极简（对齐 Codex/MiMo） | P1 | ✅ 未选纯文本 · 选中灰底 · 悬停纯图标 |
| IA-18 | 去侧栏搜索框 + 柔化悬停 | P1 | ✅ 无搜索框（⌘K）· 悬停更轻 |
| IA-19 | 会话标题过长悬停自动横滚 | P1 | ✅ hover marquee（溢出才滚） |
| IA-20 | 项目管理 + 侧栏会话分区 | P1 | ✅ 最近工作区切换 · 项目下拉 · 会话列表分区 |
| IA-21 | 新建任务页 vs 对话 Composer 分态 | P1 | ✅ 空态 Hero+建议条在输入下 · 对话中排队占位 |
| IA-22 | 任务开始真选项目 / 不在项目中工作 | P1 | ✅ projectMode 真切换 · 专用空项目目录 · 仅欢迎页显示芯片 |
| IA-23 | 对话流连续阅读（去卡片割裂） | P1 | ✅ 无助手框 · 编辑行 +N−M · 工具「查阅·N 搜索」· 工作中 Xs |
| IA-24 | 项目下拉挂在 Composer 芯片（非侧栏） | P1 | ✅ ZCode 同构：芯片上方菜单 · 不在项目中工作在菜单内 |
| IA-25 | 修：点芯片无弹层 / 固定「选择项目」 | P1 | ✅ fixed 定位 · 芯片「选择项目/目录名」· 外点不抢关 |
| IA-26 | 根因：composer pointer-events:none 吃掉点击 | P1 | ✅ 芯片/菜单 pointer-events:auto |
| IA-27 | 打开文件夹（系统对话框）替代手输路径 | P1 | ✅ osascript choose folder · 无手输 |
| IA-28 | 修：悬停近黑 + 点选文件夹黑屏 | P1 | ✅ surface-hover · pick_folder 脱主线程 |
| IA-29 | 修：点 + 整窗灰屏（鼠标移出才恢复） | P1 | ✅ 去全屏 fixed backdrop · 外点/Esc 关 |
| IA-30 | 浏览器标签：地址栏 + 预览 + Wiki | P1 | ✅ URL 栏 · iframe/srcdoc · 系统打开 · Wiki |
| IA-31 | 标签关闭钮悬浮态 | P1 | ✅ 悬停才显示 · 浅灰 pill · 禁止重色块 |
| IA-32 | 浏览器选取网页元素加入对话 | P1 | ✅ 点选注入 Composer · 跨域先抓取为 srcdoc |
| IA-33 | 修：点选无高亮 —— 父页操纵同源 DOM + ZCode 检测器 | P1 | ✅ 蓝框 hover · 元素信息浮层 · 不靠页内 CSP 脚本 |
| IA-34 | 点选浮层样式对齐 Codex | P1 | ↩️ 已撤销（检测器回 IA-33；输入区 chip 仍为 IA-35） |
| IA-35 | 网页元素以附件 chip 进输入区（非塞 textarea） | P1 | ✅ 结构化 attachments · 发送时再拼进 turn |
| IA-36 | 进入对话后输入框上方无项目选择器 | P1 | ✅ 仅 isNewTask 显示 ctx-chips/建议条 |
| IA-37 | 修：点 + 标签菜单超出视口 | P1 | ✅ right 锚定 + 视口夹紧 + 空间不足上翻 |
| IA-38 | 点选浮层紧凑（标签+尺寸贴光标） | P1 | ✅ 小字单行 · 不大块居中 · 跟随指针 |
| IA-39 | 再收：单行 tooltip · 标签左 · 缩小宽度 | P1 | ✅ tag 左对齐 · 11px 单行主信息 · 贴光标 |
| IA-40 | 新增标签菜单：标题贴图标靠左 + 更紧凑 | P1 | ✅ menu-label 左对齐 · 收 item 字号/间距 |
| IA-41 | 问人反向通道：user_input_request → respond | P1 | ✅ 挂起卡 · Enter 发送 · 锁 composer · boot/resume 刷 thread/goal/get |
| IA-42 | view_image 多模态（内核侧） | P1 | ✅ 默认注册 · ImageAttachment · UserImage 编码 image_url · 事件仅路径 |
| IA-43 | Codex 控制面对齐（steer/skills/config/fs/items/turns） | P1 | ✅ 方法表 49 · turn/steer · skills/list · config/read · thread/items\|turns · fs/* 只读+沙箱写 |
| IA-44 | Codex 扩展批：archive/hooks/mcp 状态/fs copy · 方法表 61 | P1 | ✅ thread/archive\|unarchive · hooks 空表诚实 · mcpServerStatus · permissionProfile · fs/copy\|mkdir\|remove |
| IA-45 | Codex 批3：inject/revert/metadata/attachment + mcp 运行时 | P1 | ✅ 方法表 69 · inject 不驱动 · revert=rewind 换算 · 附件侧车 · mcpServer/tool\|resource 走既有 Tool seam |
| IA-46 | Codex 批4：plugin/* + marketplace/*（本地市场，无账号） | P1 | ✅ 方法表 79 · $NEO_HOME 市场/安装清单 · 只复制资源不执行 · plugin/share/* 绑身份仍不做 |
| IA-47 | Codex 批5：threadSection/config 写/skills 根/experimental/app 空表/fuzzy | P1 | ✅ 方法表 102 · 分区 sidecar · config.json 乐观版本 · skills 禁用+extraRoots · app 空表诚实 · 无 Guardian/Windows 沙箱点名拒绝 |
| IA-48 | Codex 批6：fs/watch 真事件 + externalAgent 迁移 + mcp oauth 边界 | P1 | ✅ 方法表 109 · EventBus 推 fs/changed · PathWatcher 有界合并 · 本地 agent 资产探测/导入 · 无浏览器 OAuth 点名拒绝 |
| IA-49 | 桌面接入：steer / 归档 / 分区 / 技能 / 插件市场 / 文件树监听 | P1 | ✅ busy Enter 转向 · thread/archive 筛选 · threadSection 分组 · 设置 skills+插件安装 · files_changed\|fs_changed 刷树 |
| IA-50 | 修：顶栏双击缩放（Overlay 标题栏被内容盖住） | P1 | ✅ 固定 titlebar-drag + drag-region · browser-tabs 让出 title 高 · toggleMaximize 权限 |

---

## 10. 变更纪律

1. 需求 → 先改 **§9 表** → 再改代码。  
2. 禁止单张截图直接改布局而不回填本文。  
3. 合规：不拷贝三家源码/图标；只抄**结构与量**。  
4. 回归：`apps/desktop-tauri` 的 `npm run build` + 内核门禁（若动协议）。
