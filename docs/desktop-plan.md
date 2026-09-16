# 桌面版实现计划

> 状态：**阶段 A（工具链）与阶段 B（框架选型）已完成；阶段 C/D/E 未开始**。
> 前置问题 1、2、4 已解决，只剩第 3 条（`DesktopHost` 定位，已标注为测试替身）。
> 决策依据见 [`neo-plan/02-架构设计/ADR/ADR-0006-桌面原生GUI.md`](neo-plan/02-架构设计/ADR/ADR-0006-桌面原生GUI.md)。
>
> **框架已选定：egui**（阶段 B 实测，gpui 因构建期依赖完整 Xcode 的 Metal 工具链而退出对比）。
>
> **本文所有事实均来自实际核查**（读源码 + 查 crates.io registry + 原型实测），
> 凡未实测的一律标"待实测"。

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

## 阶段 B · 原型对比（✅ 已实测，结论已出）

**目的：不靠论证选框架。** 同一份最小需求（窗口 + 一段 Markdown + 一个 diff + 一个输入框 + 一段流式文本）用两条路各写一遍。

> **结论（2026-09-16 实测）**：
> - **路线 A（egui）走通** —— 121 行实现全部需求，编译成功、**真机开出窗口**。
> - **路线 B（gpui）在本机编不过** —— 卡在 `gpui` 的 build script 调 `xcrun metal` 编译
>   Metal 着色器，而 `metal` 编译器**只随完整 Xcode.app 分发**（本机只有 CommandLineTools）。
> - **按计划既定规则，gpui 退出对比**（"若 gpui 编不过，直接退出对比 —— 这本身就是结论"）。
>
> ⚠️ **失败原因与计划预判的不同，更值得注意**：计划预判的是 **MSRV**（Zed 钉 1.98 > 我们的 1.95），
> 而实际失败在**更早、更硬的一层** —— 构建期依赖 GPU 着色器工具链，与 Rust 版本无关。
> 也就是说：**升到 1.98 也解决不了这个问题**，它要的是装完整 Xcode。
> （本机 `xcodebuild -version` → "requires Xcode, but active developer directory
> is a command line tools instance"；`xcrun --find metal` → "unable to find utility"。）

### 实测数据（同一台机器、同一份需求）

| 指标 | 路线 A：egui | 路线 B：gpui |
|---|---|---|
| 能否编译 | ✅ 成功 | ❌ 失败（`xcrun metal` 缺失，装完整 Xcode 才行） |
| 能否开窗 | ✅ 进程存活、真窗口 | — 未到运行 |
| **传递依赖规模（实测）** | **约 165 个 crate**（`cargo tree -e normal`） | 未测得（构建前失败） |
| 首次编译时长 | **约 124 秒**（release，冷） | — |
| 增量编译 | 2 秒 | — |
| 二进制体积 | **13.4 MB**（release） | — |
| 实现行数 | **121 行**（含 Markdown 解析与 diff 渲染） | — |
| 构建产物占盘 | 592 MB | 1.1 GB（下载+编译到失败为止） |

> **计划里的依赖数被严重低估**：原表写"egui 约 16 条 normal；eframe 约 34 条"，
> 那是**各自的直接依赖**；实测 egui + eframe + pulldown-cmark + similar 的
> **闭包是约 165 个 crate**。相差约 5 倍 —— 做"零多余依赖"这类判断时必须用闭包数，
> 不能用直接依赖数。这条修正对阶段 C 的取舍也有影响。

### 实测踩到的 API 事实（写给阶段 D，避免再踩）

egui **0.36 相对常见教程是一次大改**，凭印象写必然编不过（本 spike 第一版 8 个编译错误全在此）：

| 常见写法（旧） | 0.36 实际 | 说明 |
|---|---|---|
| `impl App { fn update(&mut self, ctx, frame) }` | **`fn ui(&mut self, ui: &mut egui::Ui, frame)`** | `update` 已不是 trait 成员；另有 `logic()` 用于非绘制逻辑 |
| `egui::TopBottomPanel::top(id)` / `SidePanel::left(id)` | **`egui::Panel::top(id)` / `Panel::left(id)`** | 两者统一为 `Panel`，方向是构造器 |
| `panel.show(ctx, …)` | **`panel.show(ui, …)`** | 面板接 `&mut Ui` 而非 `&Context` |
| `Event::End(Tag::Heading(level))` | **`Event::End(TagEnd::Heading(level))`** | pulldown-cmark 0.13 的结束标签独立成 `TagEnd` |

**检索方式**（下次直接查，别猜）：读本机 registry 里的源码，如
`~/.cargo/registry/src/*/eframe-0.36.2/src/epi.rs` 的 `pub trait App`、
`egui-0.36.2/src/containers/panel.rs` 的公开构造器。

### 原型源码的去向

按计划它是"一次性 spike，可丢弃"，因此**不并入 workspace**（避免死代码与依赖膨胀）：
原型建在仓库外的独立 `cargo` 工程，测完即删。**留档的是上面那张 API 事实表与实测
数据**，而不是 121 行源码 —— 前者是阶段 D 真正会用到的东西（凭印象写 egui 0.36 必然
编不过），后者用一张表就能复现。

**若阶段 D 采用 egui，第一步就是照着上表搭骨架**：`Panel`（不是 SidePanel）、
`App::ui`（不是 `update`）、面板收 `&mut Ui`。这三点是本 spike 用 8 个编译错误换来的。

| | 路线 A：egui | 路线 B：gpui |
|---|---|---|
| 框架 | `eframe` 0.36.2（**MSRV 1.95，与当前工具链吻合**） | `gpui` + `gpui-component` |
| Markdown | `pulldown-cmark` 0.13.4 / `egui_commonmark` 0.25 | gpui-component 内置 |
| diff | `similar` + 自绘 | gpui-component 内置 `diff` |
| 依赖规模 | egui 约 16 条 normal；eframe 约 34 条 | gpui 0.2.2 约 **109** 条（多 target 门控） |
| 生态 | 23.5M 下载 / 1193 反向依赖 | 276k 下载 / 129 反向依赖 |
| 风险 | 渲染为 CPU tessellation，样式上限低 | pre-1.0、**无 MSRV 声明**、平台栈重 |

**必量的指标**：能否在 1.95 下编译成功 · 依赖条目数 · 首次编译时长 · 二进制体积 · 实现行数 · 中英混排与流式追加的观感。
（实测结果见上方结论表。**"中英混排与流式追加的观感"未量**——它要真人看窗口，机器测不出；
egui 路线已确认能开窗，观感留待阶段 D 实现时由真人验收。）

### 路线 B 的已知障碍（记录，不预设结论）

> 实测后回填：**实际拦住的是第 5 条（构建成本），不是第 1 条（MSRV）**。
> 1 从未被触发（构建在 Metal 着色器阶段就失败）；这条差异很重要 ——
> 它意味着"升级到 1.98"这个看似可行的绕法**根本救不了 gpui**。

1. ~~**无 MSRV 声明**，而 Zed 主线钉 **1.98.1**~~ —— 未能验证（构建更早失败），**且已证明非关键**。
2. **crates.io 上的 `gpui` 已停滞**：最新 0.2.2（2025-10-22），且与 main 已漂移（`Application::new` vs 新的 `gpui_platform::application`），而 **`gpui_platform` 并未发布**。用新版只能走 git 依赖。
3. **Zed 的 UI 组件拿不到**：`ui` / `editor` / `markdown` / `diff` / `component` 均为 **GPL-3.0-or-later 且 `publish = false`**，本项目 MIT —— 复用不了。
   （`gpui` 框架本体是 **Apache-2.0**，早年的 GPL 依赖 `zlog`/`ztracing` 已被官方改掉。）
4. **可用组件来自第三方 fork**：`gpui-component`（Longbridge，Apache-2.0，60+ 组件，含 `MessageScroller` / `diff` / markdown / editor）依赖 **`gpui-pre`**——即 Longbridge 自己重发布的 Zed 快照。**采用它 = 把第三方 fork 当渲染底座**，这是需显式接受的供应链选择。
5. **构建成本**：macOS 需完整 Xcode；`bindgen` 需 libclang。← **✅ 实测确认：就是这一条把 gpui 挡在门外。**
   `gpui-0.2.2/build.rs` 的 `compile_metal_shaders()` 无条件调 `xcrun -sdk macosx metal`
   编译 `src/platform/mac/shaders.metal`，失败即 `process::exit(1)`；**没有任何环境变量开关可跳过**
   （该函数里只有 `GPUI_FXC_PATH` 用于另一处，与此无关）。而 `metal` 编译器只随
   **完整 Xcode.app** 分发：本机 `xcodebuild -version` 报 "requires Xcode, but active
   developer directory is a command line tools instance"，`xcrun --find metal` 报
   "unable to find utility"。装完整 Xcode 是 ~10 GB 级的额外前置。

> **若 gpui 在 1.95 下编不过，直接退出对比**——这本身就是结论。
> **✅ 已按此规则执行：gpui 退出对比，本阶段选型为 egui。**

---

## 阶段 C · `neo-text`（把自写换成成熟 crate）

项目现有 **4 份自写实现共约 1436 行**，多数可被成熟 crate 替掉：

> **执行进度（2026-09-16）**：`diff.rs` ✅ 已完成（`6d1eb234`）；`markdown.rs` ✅ 已完成。
> `syntax.rs` 与 `width.rs` 未动（理由见下）。**实测修正了本表的收益预估**，见"收益修正"。

| 自写 | 行数 | 替换为 | 收益 | 约束 |
|---|---|---|---|---|
| `neo-host-tui/markdown.rs` | 431 | ✅ **`pulldown-cmark` 0.13.4**（MIT，`default-features=false` 下只 `bitflags`/`unicase` 是新增，`memchr` 本就在树里） | **正确性**（见"收益修正"第 1 条）；解析与渲染解耦 | 用 `Parser::new`（`Options::empty()`），**刻意不开** tables/GFM 扩展位 |
| `neo-host-tui/syntax.rs` | 385 | **`syntect` 5.3.0 + `two-face`** | 语言覆盖大增，**且解决自写版明确放弃的"跨行字符串/注释"** | **必须走 `regex-fancy`（纯 Rust）**，默认后端 `onig` 是 C 依赖。⚠️ **实测闭包 36 个 crate**（`pulldown-cmark` 只 3 个）——这是本阶段最贵的一项，动前需再确认是否值得 |
| `neo-capability/diff.rs` | 300 | ✅ **`similar` 3.2.0**（Apache-2.0） | 算法升级到 Myers/Patience；**依赖闭包实测为零** | **"换算法不换策略"**：`MAX_ALIGN_LINES` / `MAX_HUNKS` / `truncated` 必须自己保留——**`similar` 不提供任何上限保证**，会展开整个 diff（已保留，并补了摘要路径的上限） |
| `neo-host-tui/width.rs` | 320 | 终端专用，**GUI 不需要** | — | TUI 侧可换 `unicode-width`；但"按显示宽度换行 + 列布局"是真实的终端宿主逻辑，自写合理 |

**注意 feature 是加法**：依赖图里任一方开了 `regex-onig`，最终就会编译 onig，无法靠别处开 fancy 抵消。

### 收益修正（实测，与本表预估不符 —— 据实记录）

本表原写 markdown 替换能"**省掉自写子集**"、diff 替换能"**省约 200 行**"。实测**都不成立**：

1. **markdown 实现从 288 行涨到 457 行**（不含测试），不是减少。原因是"先解析、再带着色调折行"这一步需要额外的协调代码（`wrap_spans` 与逐段色调映射）。
   **但换来的是真正确性** —— 实测复现了老实现的一个 bug：老流程"先折行、再对每个片段跑行内解析"，当 `**加粗**` **跨越折行边界**时，两侧片段各自都不含配对的 `**`，解析失败，**字面星号直接留在屏幕上**：
   ```text
   这是
   **一段很长的加粗文      ← 星号没被剥掉
   字需要折行** 结束
   ```
   新实现从根上消除（已加回归测试钉住）。另外还免费获得 `_强调_`、嵌套列表、链接文字、CommonMark 行内解析的正确性。
   **教训：把"换 crate"的收益写成行数减少是错的预估方式** —— 收益在正确性与覆盖面，而适配层本身要花行数。
2. **diff 替换确实是净减**（自写 LCS + 摘要 + hunk 切分整段删除），但顺带修掉两个既有缺陷（hunk 头行号、摘要无界），所以最终改动是 +269/-172 —— 减了实现、加了测试与修复。

**因此"迁移能省行数"这个预期本身要修正**：`neo-text` 的价值应按**正确性 + 宿主中立**衡量，不按行数。

**迁移纪律**：新建 `neo-text` crate 承载**宿主中立的文本语义**（`Tone` 语义调色板从 TUI 抽出，**去掉 `StarDim`/`StarBright` 等终端专有变体**；markdown 解析；语法高亮）。
**TUI 迁移单独一步、单独验证**——它跑着 600+ 测试门禁且有视觉行为，不与新 GUI 混在一次改动里。迁移期会有短暂的双实现并存，如实记录。
（本轮按此纪律只改了 `markdown.rs` 一个文件，并把 `diff.rs` 作为独立提交 —— 没有把多个替换混在一次改动里。）

### ✅ `neo-text` 抽取已完成（阶段 D 的前置）

**落到 `crates/neo-text`（L1 BASE，24 个 crate）**：`Tone` + `width` + `syntax` + `markdown`。
用 `git mv` 搬（保留历史），TUI 侧 `pub use neo_text::{markdown, syntax, width, Tone};`
re-export —— 现有 73 处 `width::*`、258 处 `Tone` 调用点**一行未改**。

`Tone` 的宿主中立化（就是本表要的"去掉终端专有变体"）：
- **删掉 `StarDim` / `StarBright`**。它们是终端星场的**装饰**，不是文本语义 ——
  GUI 宿主没有星场，也不会查 `theme.star_dim`。改由调用方经 `Tone::Rgb` 传具体
  颜色（TUI 侧新增一个 `star_tone(theme, brightness)` 从自己主题取值）。
  这是 PROJECT_MEMORY §4.32「装饰不得进入语义」在跨 crate 边界上的又一次应用。
- 顺带**删掉死代码 `Grid::fill_stars`**（16 行，无调用点，是星场重构后的遗留）。
  它的存在正好说明为什么该删：它和 `fill_background` 里的星场写入点**重复实现了
  同一件事**，留着就是下一个"改了这里忘了那里"的来源。

**顺带更正 TUI 的依赖声明**：`Cargo.toml` 原写"**零依赖**"，但文本语义下沉后依赖树里
必然多了 `pulldown-cmark`。改为"**终端控制零依赖**"并说明零依赖指的是哪一层 ——
把"零依赖"当成整个依赖树的属性是不准确的（阶段 C 引入）。

**架构守卫同步**：`check_architecture.py` 的 `LAYER` 登记 `neo-text: 1`（与 L1 PLATFORM
同级；放 `LAYER` 而非 `SIDE` 是为了让它的依赖**也受 A1 检查** —— `SIDE` 会跳过方向校验）。
已用"注入违规依赖"验证过守卫确实会拦（`neo-text(L1) -> neo-core(L2)` 报 A1 失败）。

---

## 阶段 D · `neo-host-egui`（L5 新宿主）

> **执行进度（2026-09-16）：骨架已落地，界面未验收。**
>
> | 项 | 状态 |
> |---|---|
> | 驱动层（工作线程 / 每帧 Pump / `driving` 守卫 / `ApproveStep`） | ✅ 已实现 + 3 项回归测试（含"越界 Pump 不触发多余模型请求"，去掉守卫即失败） |
> | D2 Markdown 正文 · D3 思考轨迹 · D4 工具卡片 · D5 diff · D6 轮摘要 · D7 Goal 面板 | ✅ 数据通路已接通（`ui::Transcript` 纯数据层，可无窗口测试） |
> | D10 审批（三档 Allow/Always/Reject + 未决时**阻塞输入**） | ✅ 已实现 |
> | 复用 `neo-text`（`blocks()` + `palette`）+ 防漂移守卫 | ✅ |
> | 中文字体加载 + 字形自检门禁 | ✅ 修掉「界面中文全是豆腐块」（机器测不出的缺陷） |
> | 端到端界面测试（真内核 → 驱动 → Transcript） | ✅ 6 项，0.04 秒 |
> | 登记（workspace / `LAYER` / `HOSTS` / T6 conformance） | ✅ 宿主后端现为 4 个 |
> | 接线 `neo desktop`（默认原生）/ `--webview` | ✅ 两条路都真机开窗验证过 |
> | D1/D9 任务栏与命令中心 · D8/D11/D12 终端/模式/文件树 | ⬜ 未做（需新后端能力，按计划排在后面） |
> | **观感验收** | ✅ **已做**（2026-09-16 授权后真机截图）—— 中文与 D2–D7 全部正确渲染；过程中抓到「中文豆腐块」缺陷并修掉 |

### 视觉验收抓到的一个缺陷（**只有看图才能发现**）

第一次真机截图：拉丁文字正常，**所有中文都是 `□`** —— egui 默认字体不含 CJK 字形
（官方文档明写），而本项目界面文案是中文。它躲过了编译、开窗、驱动层测试、T6 契约
**全部**机器检查，因为那不是逻辑错误，而是**字体缺字形**。

修法：① `fonts.rs` 按平台候选表运行时加载系统中文字体（**不打包字体** —— 候选
20 MB 量级且涉及再分发授权；Proportional 与 Monospace 都要装）；
② **把「能不能显示中文」变成机器门禁** `verify_cjk_renderable`（无头 `run_ui` 跑一帧
+ `Fonts::has_glyph` 逐个查），并配反例测试证明它有牙齿。
**「只能靠人眼」不是可接受的终局。**

### 落地时与计划不同的三点（据实记录）

1. **`neo-text` 又多了一个 API：`blocks()`。** 计划说"文本处理复用走 `neo-text`"，
   但没说清复用什么形态。`render()` 面向终端、按等宽列**折行**；egui 有自己的
   文本布局，用 `render()` 的结果会**折两次**（断点错位）。所以新增 `blocks()`：
   只做解析与语义分层（前缀缩进、项目符号、色调），**换行交给宿主**。
   两者共用同一个解析器，内容与色调由测试钉住一致，差别只在谁断行。
   调色板也是同理新增（`neo-text::palette`），并加了 TUI 侧的逐字段守卫测试防漂移。
2. **`egui` feature 进了 `default`。** 计划没提 feature 归属。理由：`neo desktop`
   按计划就该是原生 GUI，不在默认里会让用户拿到"未编译"。且 `neo-host-egui` 是
   workspace 成员，`cargo check --workspace` 无论如何都会编译它 —— 进默认
   **不新增 Linux 构建面**，只是让 CLI 的接线分支也进 CI（否则那段
   `#[cfg(feature = "egui")]` 无人编译，正是"腐烂"的成因）。
3. **驱动层的 `drain()` 是 `try_recv`，一帧都不等。** 计划写的是"照 TUI 模式，
   但用 `try_recv` 而非 `recv_timeout`"—— 已照做，并**量了时间**（内核忙时
   `drain` 返回耗时 < 50ms）而不是只断言"返回空批"。

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
  **实测规模**：egui 路线的传递依赖闭包是**约 165 个 crate**（计划原估 16/34 是直接依赖数，见阶段 B）。
- ~~本文档中的 gpui 结论待实测~~ → **已回填**（阶段 B）：gpui 因构建期调 `xcrun metal`（需完整
  Xcode）而编不过，按既定规则退出对比；**选型为 egui**。
- **观感已做基础验收，但未对标**：阶段 D 已真机截图确认中文与 D2–D7 都能正确渲染
  （并修掉了"中文豆腐块"）。但**没有**逐条核对 `desktop-parity.md` 那份界面契约的还原度 ——
  "能读"不等于"像对标产品"。中英混排的细节、流式追加的动感、与 ZCode/Codex 的观感差距
  仍需后续对照（那份契约里的 D1/D8/D9/D11/D12 也还没做）。
- **egui 的样式上限**：即时模式 + CPU tessellation，样式能力弱于 GPU 渲染的 gpui/Zed。
  这是选 egui 换来的确定性（能编、能跑）所对应的代价，需在阶段 D 有预期。
