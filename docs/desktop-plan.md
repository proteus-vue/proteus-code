# 桌面版实现计划

> 状态：**计划已定，实现未开始**。本轮只完成阶段 A（工具链升级）。
> 决策依据见 [`neo-plan/02-架构设计/ADR/ADR-0006-桌面原生GUI.md`](neo-plan/02-架构设计/ADR/ADR-0006-桌面原生GUI.md)。
>
> **本文所有事实均来自实际核查**（读源码 + 查 crates.io registry），凡未实测的一律标"待实测"。

---

## 起点：桌面宿主今天是什么样

| 项 | 事实 | 依据 |
|---|---|---|
| 窗口层代码 | **111 行**（`lib.rs` 58 + `window.rs` 53） | `crates/neo-host-desktop/src/` |
| 实际界面 | `neo-host-web` 的**184 行**内联 HTML + 原生 JS | `crates/neo-host-web/src/page.rs` |
| 界面能力 | 纯文本日志 + `y`/`n` 审批 + goal 栏 | `page.rs:102-180` |
| **没有**的界面能力 | Markdown、diff、代码高亮、思考块、工具卡片、侧栏、模式切换 | `page.rs` 全文 grep 无命中 |
| 复用机制 | 桌面起本地 HTTP（`127.0.0.1:0`）+ webview 指向它 | `neo-code-cli/src/main.rs:291,312` |
| 测试覆盖 | HTTP 层**零集成测试**；`neo-host-desktop` 无测试 | `crates/neo-host-web/` 无 `tests/` 目录 |

**关键发现**：富界面所需数据**早就在事件流里流着，只是前端扔掉了**——

| 事件 | 带什么 | 184 行页面是否处理 |
|---|---|---|
| `PatchProposed { path, diff }` | 审批前的完整 diff（内核调 `tool.preview()` 生成，`neo-core/src/lib.rs:1396`） | ❌ 未处理 |
| `ReasoningDelta { delta }` | 思考过程 | ❌ 未处理 |
| `ToolCallBegin { name, arguments }` | 工具名 + 参数 | 只用 `name` |
| `FileChanged { path, additions, deletions }` | 改动文件 +N −N | ❌ 未处理 |

即：**界面重建是纯前端工作，零后端改动**。

---

## 对标基准（Codex 桌面版 / ZCode 桌面版）

逐条契约、差距清单、落地顺序见 **[`desktop-parity.md`](desktop-parity.md)**。这里只记三个影响全局的结论：

### ① 两个对标目标都是「Web 技术的 GUI + 协议后端」，**不是 Rust 原生 GUI**

| 目标 | GUI 技术 | 证据 |
|---|---|---|
| **ZCode 桌面** | Electron + React 19 + Radix UI + node-pty | 本机 `dev.zcode.app` v3.11.2；asar 内 `NSPrincipalClass = AtomApplication` |
| **Codex 桌面** | 闭源，**不在 `openai/codex` 仓库内** | `codex-rs` 的 `Cargo.lock` **全文无任何 GUI 依赖**；仓库只有 TUI(`ratatui`) + 安装器/`codex://` 深链 + `app-server` RPC |
| **Zed** | **GPUI（纯 Rust GPU 框架）** | 本机装有 `dev.zed.Zed`——**"Rust 原生能达到什么水准"的存在性证明** |

**含义**：
- "对标 Codex 桌面版"**只能对标 UX 与功能**，不能对标代码（GUI 闭源）。
- 两者架构都是 **GUI 消费一个协议**（Codex：GUI ← `app-server`；ZCode：renderer ← `@zcode/rpc`）。
- **本项目已是同构形态**（内核 + `Op`/`EventMsg` + `neo-host-web`），差别只在"谁来渲染"。
- 因此"用 Rust 原生 GUI 对标两个 Web 技术的 GUI"这件事，**其可行性证据是 Zed/GPUI，而不是 Codex/ZCode**——这点必须在选型时认清，不能含糊。

### ② 最该照抄的一段：审批语义（ZCode 官方文档）

权限门触发 → **当前任务暂停 + composer 被阻塞**（防止误把下一步排进队列）→
显示**将执行的确切内容**（命令/文件改动/工具动作）→ 三档 **Allow / Always Allow / Reject** →
**高风险或全自动模式下工具栏持续显示风险状态** → 审批**按任务作用域绑定**（切走再回来仍在）。

### ③ D2–D7 全是纯前端工作（数据已在事件流里，零后端改动）

Markdown 正文 · 思考轨迹 · 工具卡片 · **diff 渲染** · 轮摘要 · Goal 面板 ——
对应事件 `AgentMessageDelta` / `ReasoningDelta` / `ToolCallBegin{arguments}` /
`PatchProposed{path,diff}` / `TurnComplete` / `GoalUpdated`。
**这六个是投入产出比最高的一段。**

> ⚠️ **视觉层目前对标不到观感**：ZCode 的真实界面截不到（本机辅助功能/屏幕录制权限未授予），
> Codex 桌面文档 403。结构层面已够用（官方文档写得很细 + TUI 源码可读），
> 但**观感需要真人补图**，否则只能对到"结构"而到不了"像不像"。

---

## 阶段 A · 工具链升级（✅ 本轮已完成）

- `rust-toolchain.toml` / `Cargo.toml` / 三个 CI workflow：1.83.0 → **1.95.0**。
- 原先为 1.83 钉回的传递依赖**逐项解钉**（它们只在 `Cargo.lock` 里，不在任何 `Cargo.toml`）：
  - `idna_adapter` 1.1.0 → 1.2.2
  - `indexmap` 2.7.1 → 2.14.2（连带 `hashbrown` 0.15.5 → 0.17.1）
  - `num_enum` 0.7.3 → 0.7.6；`dlopen2_derive` 0.4.1 → 0.4.3
  - `hashbrown 0.15.5` 本就是该线终版（`rv=1.65`），不是"钉版"
- **已知收益**：`egui 0.36` 声明的 MSRV 恰为 **1.95**，与本工具链对齐。但这只是"解锁了可能性"，**不等于 egui 可用**——由阶段 B 实测。

---

## 阶段 B · 原型对比（一次性 spike，可丢弃）

**目的：不靠论证选框架。** 同一份最小需求（窗口 + 一段 Markdown + 一个 diff + 一个输入框 + 一段流式文本）用两条路各写一遍。

| | 路线 A：egui | 路线 B：gpui |
|---|---|---|
| 框架 | `eframe` 0.36.2（**MSRV 1.95，与当前工具链吻合**） | `gpui` + `gpui-component` |
| Markdown | `pulldown-cmark` 0.13.4 / `egui_commonmark` 0.25 | gpui-component 内置 |
| diff | `similar` + 自绘 | gpui-component 内置 `diff` |
| 依赖规模 | egui 约 16 条 normal；eframe 约 34 条 | gpui 0.2.2 约 **109** 条（多 target 门控） |
| 生态 | 23.5M 下载 / 1193 反向依赖 | 276k 下载 / 129 反向依赖 |
| 风险 | 渲染为 CPU tessellation，样式上限低 | pre-1.0、**无 MSRV 声明**、平台栈重 |

**必量的指标**：能否在 1.95 下编译成功 · 依赖条目数 · 首次编译时长 · 二进制体积 · 实现行数 · 中英混排与流式追加的观感。

### 路线 B 的已知障碍（记录，不预设结论）

1. **无 MSRV 声明**，而 Zed 主线钉 **1.98.1**——**高于**本项目的 1.95。能否编译**必须实测**。
2. **crates.io 上的 `gpui` 已停滞**：最新 0.2.2（2025-10-22），且与 main 已漂移（`Application::new` vs 新的 `gpui_platform::application`），而 **`gpui_platform` 并未发布**。用新版只能走 git 依赖。
3. **Zed 的 UI 组件拿不到**：`ui` / `editor` / `markdown` / `diff` / `component` 均为 **GPL-3.0-or-later 且 `publish = false`**，本项目 MIT —— 复用不了。
   （`gpui` 框架本体是 **Apache-2.0**，早年的 GPL 依赖 `zlog`/`ztracing` 已被官方改掉。）
4. **可用组件来自第三方 fork**：`gpui-component`（Longbridge，Apache-2.0，60+ 组件，含 `MessageScroller` / `diff` / markdown / editor）依赖 **`gpui-pre`**——即 Longbridge 自己重发布的 Zed 快照。**采用它 = 把第三方 fork 当渲染底座**，这是需显式接受的供应链选择。
5. **构建成本**：macOS 需完整 Xcode；`bindgen` 需 libclang。

> **若 gpui 在 1.95 下编不过，直接退出对比**——这本身就是结论。

---

## 阶段 C · `neo-text`（把自写换成成熟 crate）

项目现有 **4 份自写实现共约 1436 行**，多数可被成熟 crate 替掉：

| 自写 | 行数 | 替换为 | 收益 | 约束 |
|---|---|---|---|---|
| `neo-host-tui/markdown.rs` | 431 | **`pulldown-cmark` 0.13.4**（MIT，依赖极小，输出事件流，天然适配"宿主自己画"） | 解析正确性 + GFM；省掉自写子集 | 需完整 GFM 才考虑 `comrak`，**且必须 `default-features=false`** 躲开 `syntect-onig` |
| `neo-host-tui/syntax.rs` | 385 | **`syntect` 5.3.0 + `two-face`** | 语言覆盖大增，**且解决自写版明确放弃的"跨行字符串/注释"** | **必须走 `regex-fancy`（纯 Rust）**，默认后端 `onig` 是 C 依赖 |
| `neo-capability/diff.rs` | 300 | **`similar` 3.2.0**（Apache-2.0，自带 `grouped_ops` 出 hunk） | 省约 200 行，升级到 Myers/Patience | **"换算法不换策略"**：`MAX_ALIGN_LINES` / `MAX_HUNKS` / `truncated` 必须自己保留——**`similar` 不提供任何上限保证**，会展开整个 diff |
| `neo-host-tui/width.rs` | 320 | 终端专用，**GUI 不需要** | — | TUI 侧可换 `unicode-width`；但"按显示宽度换行 + 列布局"是真实的终端宿主逻辑，自写合理 |

**注意 feature 是加法**：依赖图里任一方开了 `regex-onig`，最终就会编译 onig，无法靠别处开 fancy 抵消。

**迁移纪律**：新建 `neo-text` crate 承载**宿主中立的文本语义**（`Tone` 语义调色板从 TUI 抽出，**去掉 `StarDim`/`StarBright` 等终端专有变体**；markdown 解析；语法高亮）。
**TUI 迁移单独一步、单独验证**——它跑着 617 测试门禁且有视觉行为，不与新 GUI 混在一次改动里。迁移期会有短暂的双实现并存，如实记录。

---

## 阶段 D · `neo-host-egui`（L5 新宿主）

### 内核驱动（**必须照 TUI 的模式，不能想当然**）

| 事实 | 依据 |
|---|---|
| 内核**没有** `next_event()` | 全仓 grep 零命中 |
| 真实 API：`submit(op) -> Result<Vec<EventMsg>, KernelError>` | `neo-core/src/lib.rs:923` |
| 它**同步阻塞**（整轮含多次网络往返） | `submit` 内直接 `drive_steps()` |
| 即使 `Op::Pump` 也可能阻塞：片宽上限 `PUMP_SLICE=80ms` / `MAX_DELTAS_PER_PUMP=256`，但 deadline 只在 `next()` 返回**之后**检查——首个 token 迟迟不来（思考型模型 1–3s）就钉死线程 | `neo-core/src/lib.rs:55,57,1273` |

因此：
1. **内核独占工作线程 + mpsc**（照 `neo-code-cli/src/tui_driver.rs` 的模式；egui 版用 `try_recv` 而非 `recv_timeout`，`update()` 一帧都不该等）。
2. **每帧发 `Op::Pump`**（**不是** `UserTurn`——那会一次跑完整轮，界面冻结）。
3. **维护 `driving` 边界守卫**（`TurnStarted` 开、`TurnComplete`/`ApprovalRequest` 关），丢弃越界 Pump。**否则空闲内核上会多起一步 = 多一次真实模型请求**，`tui_driver.rs:279` 有回归测试守着这个坑。
4. **审批走 `Op::ApproveStep`**（`Op::Approve` 会一次跑完剩余往返，界面又冻）。
5. **架构守卫 A3 禁止宿主互相依赖** → 驱动逻辑必须在 `neo-host-egui` 内实现，**不能** `use neo_code_cli::tui_driver`，也不能用 `neo_host_web::broadcast`。文本处理复用走阶段 C 的 `neo-text`。

### 界面（数据全部来自事件流，零后端改动）

**布局对标 ZCode 桌面版**（见 `desktop-parity.md` §1.1）：左侧任务/工作区栏 · 中央转录区 ·
右侧 summary/Goal 面板 · 底部终端面板 · 覆盖式命令中心。

**先做 D2–D7（纯前端、数据现成，投入产出比最高）**：
Markdown 正文（`AgentMessageDelta`）· 思考轨迹可折叠可搜索（`ReasoningDelta`，**当前被扔掉**）·
工具卡片含参数摘要（`ToolCallBegin{name,arguments}`）· **diff 渲染**（`PatchProposed{path,diff}`，
**内核审批前已生成**）· 轮摘要（`TurnComplete`）· Goal 面板（`GoalUpdated{snapshot}`）。
再照 ZCode 语义做 D10 审批（**阻塞 composer** + 三档 Allow/Always/Reject + 风险常驻）。
D1/D9（任务栏/命令中心）与 D8/D11/D12（终端/模式切换/文件树）需新后端能力，排在后面。

配色取自 `crates/neo-host-tui/src/theme.rs`，与 TUI 及官网同一视觉语言。

### 接线

`neo desktop` → 原生 GUI；`neo desktop --webview` → 保留原 wry 实现（ADR-0006 决定不押注单一方案）。

### 登记

workspace members + `docs/neo-plan/05-验证/checks/check_architecture.py` 的 `LAYER`（5）与 `HOSTS`。
`HostBackend` 契据实现：`id` / `capabilities` / `consume` / `facts`（参考 `neo-host-desktop/src/lib.rs:38-58`），保证 **T6 宿主等价**断言仍成立。

---

## 阶段 E · 窗口体验与打包（macOS）

- **窗口**：尺寸/位置记忆（`~/.neo/desktop.json`）、最小尺寸、原生菜单（Cmd+Q/W、关于）、Dock 图标、启动加载态。
- **图标**：512×512 PNG → `.icns`，从 `website/public/favicon.svg` 的菱形标志派生（品牌一致）。
- **`Info.plist`**：`CFBundleIdentifier` / `Name` / `Executable` / `IconFile` / `LSMinimumSystemVersion` / `NSHighResolutionCapable`。
- **`scripts/bundle-macos.sh`**：组装 `Neo.app/Contents/{MacOS,Resources}` → `.zip` / `.dmg`。
  **自写脚本、零新依赖**——把 bundler 加进 workspace 依赖树会带来新的传递依赖，可能再次引发版本解析问题。
- **CI**：`release.yml` 新增 macOS 打包 job（`needs: [resolve-version, build]`，与 `release` / `publish-npm` 并列）。
  `.app` / `.dmg` 用**新命名**，不碰 `install.sh` 与 `publish-npm.sh` 已依赖的 `neo-v<tag>-<target>.tar.gz` 约定。
- **签名与公证**：留接口不做（需 Apple Developer ID 证书）。

---

## 有待解决的前置问题（不解决会埋雷）

### 1. ~~安全网缺失：HTTP 层零集成测试~~ ✅ 已补

**现状**：`crates/neo-host-web/tests/http_endpoints.rs` 共 **16 项**集成测试，起真实监听端口、
走真实 `route()`，只把内核侧替换成记录 Op 的假内核（`start()` 本就接受注入的 `handle_op`，
因此不需要真的内核就能测 HTTP）。覆盖：内置页面 / 404 与错误方法 / `POST /api/turn`（引用
解析走协议层、空任务 400）/ `GET /api/events`（`subscribed` 确认、线格式扁平信封、多订阅者
扇出、断连回收）/ `GET /api/approve`（decision 映射、`%3D` 解码、缺 id 400）/
`POST|GET /api/goal`（设定、`advance|clear|pause|resume`、非法 action 400、无活动目标 409、
`current_goal` 跟踪）。

**同一轮补掉一个真实缺陷**：SSE 原先只在"写事件失败"时才发现客户端断开，空闲期间断开的连接
会一直占着订阅槽位与**连接线程**；并发上限 64，攒满即对新连接 503。已改为**事件驱动的断开
探测**（服务端读侧返回 0；不发心跳、不改线格式），回归测试钉住（撤掉修复即失败）。

### 2. ~~无鉴权~~ ✅ 已加访问令牌

**原问题**：只绑回环限制的是**网络来源**，不是**谁能访问**。本机任意进程可枚举端口调用端点
（`POST /api/turn` 写工作区、`GET /api/approve` 批准挂起审批、`POST /api/goal` 下发编排）；
更现实的是**浏览器里的任意网页** —— 端点全是简单请求，恶意页面可跨源 POST 且**请求会生效**（CSRF）。
桌面绑随机端口只是降低可见性，不是访问控制。

**已实现**（`neo-host-web/src/auth.rs`）：
- 启动时生成 256 位随机令牌（`/dev/urandom`）；**除内置页面外一切路径都要令牌**，
  含不存在的路径（未鉴权者连路由都枚举不出）。
- 传递方式：query `?token=`（浏览器侧唯一通用方式 —— `EventSource` 设不了请求头）
  或请求头 `X-Neo-Token`（程序化客户端）。
- 令牌由宿主打印的 `page_url()` 给出，放在 **fragment**（`#token=…`）：不发给服务器、
  不进 Referer、不进服务端日志；页面自身从 `location.hash` 取并自动接在每个请求上。
- 恒定时间比较；`--addr` 绑非回环时显式警告（令牌拦得住未授权调用，但流量是明文 HTTP）。
- 覆盖：7 项鉴权集成测试（含逐个端点的未带令牌 401、错令牌、路由不可探测、页面接线源码断言）
  + 真机 `curl` 验证六条路径。

**诚实边界**：单进程生命周期内的一次性令牌，不是多用户鉴权（无用户/角色概念）；进程重启即换新值。
它防"本机其它程序"与"浏览器恶意页面"，**不防**能读本进程内存或 `/dev/urandom` 的对手。

### 3. `DesktopHost` 是死代码

`crates/neo-host-desktop/src/lib.rs:38-58` 的 `DesktopHost` 只被 T6 等价测试使用（`neo-mock/tests/conformance.rs`），**真实桌面跑的是 Web 宿主**。新宿主实现时要么让它成为真实路径，要么明确标注其测试替身身份。

### 4. ~~规格漂移~~ ✅ 已同步

`L5-host.md` 有两处漂移，都已按实现更正：
- 原写桌面"优先用 `wry` 自定义协议直接注入资源（**不开端口**）"—— **实际实现**是开本地
  回环端口复用 Web 宿主页面（`window.rs:1-21` 已把该取舍写成设计决定）。
- 原写 `neo-host-web` 是"**axum + WebSocket**"—— **实际实现**是手写 HTTP + SSE
  （零依赖，理由：调试链要浅；SSE 单向推送够用，省掉握手与帧协议）。
- 顺带补上：桌面的"复用现有 React + 液态玻璃"前提已不成立（那份界面在 `legacy/`），
  以及 Web 端的访问令牌与有界性约定。

**剩余的前置问题**：第 3 条（`DesktopHost` 死代码，待新宿主实现时定夺）。

---

## 诚实边界

- **仅 macOS**：Windows 沙箱未实现（受限档位 fail-closed），不在 release 矩阵内。
- **无签名**：用户首次打开需右键"打开"绕过 Gatekeeper。
- **`page.rs` 那 184 行不在本次范围**：它是 `neo serve`（Web 宿主）的页面；桌面原生化后它不再影响桌面，但 **Web 宿主界面依然简陋，是另一条线**。
- **对"零多余依赖"的让步**：GUI 栈是数十个 crate（TUI 至今手写 ANSI、HTTP 手写）。此让步见 ADR-0006 的理由与约束。
- **本文档中的 gpui 结论待实测**：阶段 B 的原型会给出最终答案，届时回填。
