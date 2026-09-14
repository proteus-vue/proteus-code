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

Markdown 正文 · 思考块（默认折叠，对齐 TUI `/thinking`）· 工具卡片（名称 + 参数摘要 + 退出码）· **diff 渲染**（`PatchProposed`）· 审批面板（diff + 允许一次 / 总是允许 / 拒绝，对齐 opencode 三段式）· 侧栏（Context 占用 / Todo / Modified Files `+N -N`）· 输入框 · 状态栏。
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

### 1. 安全网缺失：HTTP 层零集成测试

`route()`、`/api/turn`、`/api/events`(SSE)、`/api/approve`、`/api/goal` **全无集成测试**（`neo-host-web` 只有 15 个单元测试，覆盖 HTTP 解析与广播；`crates/neo-host-web/tests/` 不存在）。
**新宿主也要复用这条 HTTP 链路**（`--webview` 路径），所以**动界面之前应先补这层网**。

### 2. 无鉴权（事实，非建议）

端口绑 `127.0.0.1` 只限制**网络来源**，**不限制本机进程**。端点无任何认证：`POST /api/turn` 直接交给内核写操作、`GET /api/approve` 可任意批准挂起审批、`POST /api/goal` 可下发编排。
桌面绑**随机端口**（`127.0.0.1:0`）只是**降低可见性，不是访问控制**——本机进程仍可枚举端口。
`PROJECT_MEMORY` 诚实清单已承认这一点。**打包成"应用"后用户会默认它安全**，因此建议在原生宿主阶段顺手加一次性 token（启动时生成、注入窗口、HTTP 校验）。

### 3. `DesktopHost` 是死代码

`crates/neo-host-desktop/src/lib.rs:38-58` 的 `DesktopHost` 只被 T6 等价测试使用（`neo-mock/tests/conformance.rs`），**真实桌面跑的是 Web 宿主**。新宿主实现时要么让它成为真实路径，要么明确标注其测试替身身份。

### 4. 规格漂移（顺手修）

`L5-host.md:87` 写"优先用 `wry` 自定义协议直接注入资源（**不开端口**）"，**实际实现**是开本地回环端口复用 Web 宿主页面（`window.rs:1-21` 已把该取舍写成设计决定）。规格未同步。

---

## 诚实边界

- **仅 macOS**：Windows 沙箱未实现（受限档位 fail-closed），不在 release 矩阵内。
- **无签名**：用户首次打开需右键"打开"绕过 Gatekeeper。
- **`page.rs` 那 184 行不在本次范围**：它是 `neo serve`（Web 宿主）的页面；桌面原生化后它不再影响桌面，但 **Web 宿主界面依然简陋，是另一条线**。
- **对"零多余依赖"的让步**：GUI 栈是数十个 crate（TUI 至今手写 ANSI、HTTP 手写）。此让步见 ADR-0006 的理由与约束。
- **本文档中的 gpui 结论待实测**：阶段 B 的原型会给出最终答案，届时回填。
