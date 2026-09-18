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
- **多行输入**：composer 支持换行编辑与多行提示词（贴代码、列要点）。
- 模型 picker `Ctrl/Cmd+M`；**Thought Level picker `Ctrl/Cmd+T`**；执行模式循环 **`Shift+Tab`**（输入框聚焦时）。
- 四种执行模式：Ask before changes（默认）/ Edit automatically / Plan / Full access。

> 对照本项目：这四档与我们内核的 `ExecMode` 五档（plan/confirm/default/auto-edit/full）**是同一类东西**，
> 可直接做映射，不需要发明新概念。
>
> **多行已对齐**（gpui 宿主）：Enter 提交、`Shift+Enter` 换行，高度
> `auto_grow(1, 6)`。细节见 §4.64(aw) —— 补的过程中抓到一个"回车既提交又留下
> 一个换行"的真缺陷（平台把 Enter 又当文本送了一遍）。

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

> **进度（2026-09-16）**：下表"我们"一列原本描述的是 `neo-host-web` 那份
> 184 行内置页面（那是 `neo desktop` 的旧实现）。新建的**原生 GUI 宿主**
> `neo-host-egui` 已实现 D2–D7、D10、D11 —— 详见每行的 ✅ 标注与括号里的实测结论。
> **未经视觉对标的项**（D1/D8/D9/D12）与"像不像"仍需后续逐条核对。

**前置（比任何 D 项都硬）**：`输入区`（含**自动聚焦**，`neo-ui-behavior::FocusIntent`
单一槽位 + `FocusHandle::is_focused` 确定性验证）。**键盘路径可自动化验证**：
需先 `bash scripts/make-app.sh --features gpui` 打包 —— 裸二进制的
`bundle_id` 为 null，WindowServer 投不进键盘事件（与 gpui 无关）。
 —— gpui 宿主此前是**纯展示的一行**，
键盘打了字不进任何地方（只能靠 `NEO_GUI_PROMPT` 喂任务，对不可用）。
已接入 `input::Input` + `InputState`，真机验证过「键盘输入 → 提交 → 内核收到 →
模型回复」全链路（证据是会话日志里的 `user_submitted`，不是截图）。

| # | 项 | 对标目标 | 我们 | 数据在事件流里吗 |
|---|---|---|---|---|
| D1 | 左侧任务/工作区栏（状态、`+/-` 行数、分组） | ZCode | ⚠️ **会话栏已完成**（列表 + 当前项高亮 + 新建 + 可切回；当前会话即使尚未落盘也显示）。**状态与改动行数已补**：这两项此前判为"需要扩展会话元数据"，实际**日志里本来就有** —— `turn_started`/`turn_complete`/`error` 足以推出结束状态，`files_changed` 就是改动行数（取**最后一次快照**，因为它是覆盖语义）。做成 `SessionStore` 的单遍扫描派生（不为每个字段各扫一遍），真机验证：失败=红实心●、未完成=黄半环◐、正常结束不画圆点、`+3 -1` 正常显示。<br>**仍未做**：按工作区分组（日志里没有工作区信息，需真加元数据，不编）、Grouped/Workspace/Timeline 视图、Archive、搜索 | 部分（分组需新元数据） |
| D2 | 转录区 Markdown 渲染 | 两者 | ✅ **已实现**（共享 `neo-text::markdown::blocks`，代码块带语法高亮、列表/引用/链接齐）。<br>另补上**引用解析结果**与**项目指令来源**两行（`RefsResolved` / `InstructionsLoaded`）：此前两个 GUI 宿主都不显示，用户看不到 `@文件` 到底读到没有、这次会话受哪些 `AGENTS.md` 约束、指令是否被 32 KiB 截断。判定在共享转录层，与 TUI 同一套文案 | ✅ `AgentMessageDelta` + `RefsResolved` / `InstructionsLoaded` |
| D3 | **思考轨迹**（可折叠、可搜索） | ZCode | ✅ **已实现**（可折叠 + 显示字数 + **搜索**）。搜索语义：大小写不敏感、**只搜思考块**（工具输出动辄上万行，全量扫不是用户要的）、命中块**自动展开**（否则看到"N 处命中"却在屏幕上找不到）、无命中给明确提示而非静默。判定在共享层 `neo-driver::transcript::search_reasoning`（两个宿主必须同一套）。真机验证过：中文"缩进"命中并高亮，不存在的词显示"思考轨迹里没有匹配" | ✅ `ReasoningDelta` |
| D4 | 工具调用**分组** + 参数摘要 | 两者 | ✅ **已实现**（gpui 侧：卡片含名字/状态/exit/参数摘要/输出/截断标注；**同轮连续调用折叠成组**，组头给「✓/✗/⏳ + N 次调用」，点开逐张展开。分组判定在共享层 `neo-driver::transcript::tool_runs` —— 两个宿主必须用同一套判定，否则同一个转录在两处长得不一样，所以逻辑从第一天就放在共享层。<br>**egui 侧只有卡片、未接分组**：egui 已冻结（只修 bug，等 gpui 覆盖齐了删除），不为它加新功能；将来若真需要，接的是同一个函数，不会长出第二套判定） | ✅ `ToolCallBegin{name,arguments}` |
| D5 | **diff 渲染** | 两者 | ✅ **已实现**（按行着色；顺带修掉"预览与执行可能不是同一文件"的缺陷）<br>**已补行背景带**（gpui 宿主）：新增/删除/hunk 头各一层底，让"改动落在哪几段"一眼扫得出 —— 此前只有逐行彩色文字，给不出那个形状。走渲染缝自绘（`diff_backdrop`），对齐由"底带与文字共用同一行高"构造保证，真机实测每行 26pt 严丝合缝（见 §4.64(ay)）。<br>**折叠未改区块：已生效** —— `neo-ui-behavior::fold` 只折连续 >6 行的上下文，而审批预览的上下文半径已由 3 提到 **10**（用户拍板），超过折叠阈值 6，故**已真正生效**：真机实测 25 行 diff 折成 14 行，`⋯ 未改 4 行`/`7 行` 与算法逐项对得上。两者耦合有断言守护（见 §4.64(bc)）。<br>**已补行内字符级高亮**：真正变化的字压一层更浓的底（用底色而非粗体 —— CJK 无真粗体字面，加粗等于没做；真机实测 29→89 差 60 色阶）。判定在共享层 `inline_emphasis`，两宿主共用。<br>**仍未做**：并排（side-by-side）视图、跳转上/下一处改动 | ✅ `PatchProposed{path,diff}` |
| D6 | 每轮**执行摘要 + 耗时** | ZCode | ✅ **两个 GUI 宿主都已实现**（token + 耗时。耗时**刻意不进共享转录模型**——挂钟时间会破坏回放确定性，故做成可注入时间的纯逻辑 `neo-ui-behavior::clock`） | ✅ `TurnComplete`（耗时由宿主计时） |
| D7 | 右侧 summary / Goal 面板 | ZCode | ✅ **两个 GUI 宿主都已实现**（目标卡片 + 子任务清单 + **设定入口** + 暂停/恢复/清除）。<br>补过两处"只做了一半"：gpui 侧原先**没有设定目标的入口**（面板写着"未设定（用 /goal 设定）"，而 `/goal` 是 TUI 的命令行语法，GUI 里走不通 —— 等于指了条死路），也**没有暂停/恢复/清除**（目标会持续自动推进并花真钱，却只能重开窗口才停得下来）。真机逐个验证过：三个按钮都真的发出对应 `Op`，日志里有 `goal_pause`/`goal_resume`/`goal_cleared`，按钮文案跟随内核状态在"暂停/恢复"间切换。<br>第三处**最隐蔽**：gpui 从不发 `Op::GoalAdvance`，于是目标**永远只有第一个子任务会跑** —— 界面看起来完全正常（目标、清单、按钮都在），只是停住不动。补上后真机验证到子任务 1 走完"规划→执行→审查→复盘"四轮并自动进入子任务 2 | ✅ `GoalUpdated{snapshot}` |
| D8 | 底部终端面板 | ZCode | ⚠️ **两宿主已做命令台**（`Cmd+K` → `/terminal` 开关；输入命令走 `Op::Shell`：**不经模型**、走沙箱、输出上限一致，真机验证日志为 `op {"shell"...}` + `tool_call_end exit 0`）。**与 ZCode 的关键差别**：它用 `node-pty` 起真 PTY（可跑 vim/htop 这类全屏交互程序），我们是**每条命令一个进程、无 PTY** —— 所以面板叫「命令台」不叫「终端」，边界如实呈现 | ✅ 无需新后端能力（原写「需新功能」是过时判断） |
| D9 | 命令中心（`Cmd+K`） | ZCode | ⚠️ **两宿主已做 commands 一类**（覆盖式面板 + 搜索过滤 + 键盘导航 + 11 条命令，每条都有可观察效果，有测试逐个守着）。**conversations / files 两类未做** —— 它们需要会话库与文件索引，属 D1 范畴 | 纯前端（已兑现） |
| D10 | 审批：**阻塞 composer** + 三档 + 风险常驻 | ZCode | ✅ **已实现**（三档 Allow/Always/Reject；未决时输入框 `disabled`；高风险档位在状态栏**常驻**风险提示） | 部分（`ApprovalRequest`） |
| D11 | 执行模式切换 | ZCode | ✅ **两个 GUI 宿主都已实现**（状态栏下拉 + `Shift+Tab` 循环；**模型 picker 一并做了**）<br>注：原表写"需新端点"是**过时的** —— `Op::ConfigureSession` 早已支持运行时改 `exec_mode` | ✅ 走 `ConfigureSession`（但**内核不发"模式已变"事件**，宿主须自行记状态） |
| D12 | 文件树 / 内置浏览器 / Repo Wiki | ZCode | ⚠️ **文件树已做**（gpui 宿主）：`/files` 命令 + 状态栏开关 → 覆盖式面板列出工作区文件；**点文件查看内容（预览）、引用是预览栏里的独立按钮**。索引在 `neo-platform::file_index`（遵守 `.gitignore`、跳过隐藏、不跟随符号链接、条目/深度/时间三重上限并如实上报 `truncated`）。补了 `neo-protocol::format_file_ref`（此前只有解析没有格式化 —— 含空格路径会静默丢引用）。**整条点击链路已真机验证**（补了 `role(Button)`+`aria_label` 后可语义点击，不必抢焦点）：点 `src/main.rs` → `@src/main.rs`；点 `my file.txt` → `@"my file.txt"`（引号正确）；再点 `plain.rs` → 追加而非覆盖；回车提交后会话日志为 `refs: [my file.txt, plain.rs]` + 两个 `已注入`。扫描准确性另与 `git ls-files` 交叉核对（342 vs 341，唯一差异是未跟踪的新文件）。<br>**内置浏览器（文件预览）已做**：面板两栏（列表 + 预览），点文件看内容 —— 文本逐行带行号、**二进制如实说明不当文本渲染**、读不出来给原因；「加入引用」是预览栏里的独立按钮。读取有界且限定工作区（两道独立的安全闸：入口检查 + canonicalize 比前缀，各自挡一种绕过，见 §4.64(bd)）。<br>**可折叠树已做**：默认全折叠（真实仓库上 342 个文件 → **16 行**：6 个根级目录 + 10 个根级文件）；目录带 ▸/▾ 与**直接子项数**、缩进渲染（层级封顶 8 级防名字被挤出边界）、同级内目录在前；面板有「展开全部 / 全部折叠」。层级推导在共享层 `neo-ui-behavior::tree`（纯逻辑，两个宿主共用）；展开态**按路径记**，重扫后仍保留（见 §4.64(bh)）。<br>**文件监听已做**：磁盘变化 → 树自动更新（事件驱动 + 容量 1 通道合并，无轮询；`.git/` 在事件层跳过、其余靠「扫完再比较」消噪——不重复实现一套忽略规则；启动在后台线程，因实测 FSEvents `watch()` 要 6.2 秒）。真机验证：外部建文件后树自动出现，无用户操作。<br>**仍未做**：Repo Wiki。egui 侧如实提示『仅 gpui 可用』 | 部分（Repo Wiki 待做） |

**结论：D2–D7 全部是纯前端工作（数据已在事件流里），零后端改动。** 这是投入产出比最高的一段。

---

## 4. 落地顺序（按"影响面 × 被复用度"排）

1. ~~**D5 + D3 + D4**（diff / 思考轨迹 / 工具卡片）~~ ✅ 已完成
2. ~~**D2 + D6 + D7**（Markdown / 轮摘要 / Goal 面板）~~ ✅ 已完成（D6 的耗时待补）
3. ~~**D10**（审批三段式 + 阻塞 composer）~~ ✅ 已完成
4. ~~**D11**（执行模式切换）~~ ✅ 已完成（**比预期简单**：`ConfigureSession` 早已支持，不需新端点）
5. ~~**D9**（命令中心）~~ ✅ commands 一类已完成。
   ~~**D1**（左侧栏）~~ ⚠️ 会话栏部分已完成（会话列表/切换/新建）；其余子项需要
   会话元数据扩展（工作区、运行状态）。**下一步**：D6 耗时、D8 终端面板，或
   扩展会话元数据把 D1 剩下的状态圆点/分组补齐
6. **D6 补充（耗时）**：纯前端记账；~~D8 命令台~~ ✅ 已完成。
7. **D12**（文件树 / Repo Wiki）：需文件索引能力。

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
