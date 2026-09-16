# ADR-0006：桌面宿主转向 Rust 原生 GUI（webview 保留为第二后端）

- **状态**：已接受（实现待排期）
- **日期**：2026-09-14
- **关联**：ADR-0003（多宿主共享内核）；`03-模块规格/L5-host.md` 第一节；`docs/desktop-plan.md`（实现计划）

## 背景

### 当年选 webview 的唯一理由

`L5-host.md` 与 `00-执行摘要.md` 都写着同一句：

> **为什么是系统 webview 而非纯原生 GUI**：为保住已有投资——现有 React + 液态玻璃界面在 webview 里**原样可用**（同一引擎，`backdrop-filter` 照常工作）。纯 Rust GUI（egui/gpui）则要求用 Rust 重写整个 UI，液态玻璃得自己写着色器。

也就是说，"用 `wry`（系统 webview）"这个决定的**全部**依据是：**复用已有的 React + 液态玻璃界面**。

### 那个前提已经被实现否定了

核查当前代码：

| 事实 | 证据 |
|---|---|
| `legacy/` 下确实有旧的 React 应用与 Electron 壳 | `legacy/bundle/src/client/index.tsx`、`legacy/desktop/src/main.ts` |
| **但 `crates/` 里对 `legacy/` 的引用数为 0** | `grep -rn legacy crates/ scripts/` 无命中 |
| 桌面实际加载的是 `neo-host-web` 的内联页面 | `neo-code-cli/src/main.rs:291` 起 `neo_host_web::start("127.0.0.1:0", …)` → `main.rs:312` 把该 URL 交给 `run_window` |
| 那份页面是 **184 行手写 HTML + 原生 JS**，无 React、无液态玻璃 | `neo-host-web/src/page.rs:8-184`，文件头注释自述"刻意保持极小……不是做一个产品级 UI" |

**结论："复用 React + 液态玻璃" 只对 `legacy/`（已不在 workspace）成立。** 桌面今天打开的是一个纯文本日志页。当年那个论据**已被它自己的实现否定**，`wry` 的收益（保住已有 UI）实际为 **0**。

### 还有一处过期注释

`crates/neo-host-desktop/src/lib.rs:6` 仍写着"保住现有 React + 液态玻璃界面零改动"——从设计文档抄来的陈述，与实际行为不符。

## 决策

1. **桌面宿主新增 Rust 原生 GUI 实现**。框架由原型实测决定（见 `docs/desktop-plan.md` 阶段 B）：
   **✅ 2026-09-16 实测结论为 `egui/eframe`** —— 原型 121 行走通全部最小需求、真机开窗；
   候选 `gpui` 因 `build.rs` 调 `xcrun metal` 编译着色器（`metal` 编译器只随完整 Xcode.app
   分发，无环境变量可跳过）而编不过，按既定规则退出对比。传递依赖闭包实测约 165 个 crate。
2. **保留 wry/webview 作为第二个 `HostBackend`**，不删除——`L5-host.md:33` 早就主张"不押注单一方案"，且那份实现已有真机交互验证（PROJECT_MEMORY §4.57）。`neo desktop` 走原生，`neo desktop --webview` 保留 webview 路径。
3. **工具链从 1.83.0 升到 1.95.0**（本 ADR 的配套前置）。1.83 钉版原本是为兜住 wry/tao 的 edition2024 依赖树；候选 GUI 框架的 MSRV 均高于 1.83，继续背着这个钉版只会让每个候选都要额外做依赖钉版维护。

## 理由

1. **原决定的前提消失**：见上。继续以"保住 React 界面"为理由维护 webview 路线，是在为一个不存在的收益付费。
2. **webview 的代价是真实的、且不会消失**：`L5-host.md:212` 自己写着——"DOM/CSS 的复杂度仍在（webview 就是浏览器）。液态玻璃的 `backdrop-filter` 包含块副作用、哈希类名不可选等坑**依然存在**……要彻底去掉，只能换 egui，代价是重写 UI。"**而 UI 其实早已需要重写**（今天只有 184 行日志页），所以"重写 UI"这个代价现在接近于零。
3. **原生 GUI 能表达 TUI 已有的信息层次**：桌面要与 TUI 对齐思考块、diff、工具卡片、审批三选项、侧栏这些结构；用即时模式/GPUI 直接画比在 webview 里再搭一层前端更直接。

## 代价（必须说清）

1. **这是对"零多余依赖"立场的实质让步。** TUI 手写 ANSI 渲染、HTTP 手写、刻意避开 ratatui/crossterm/axum——而 GUI 栈是**数十个 crate**（egui/eframe 34 条 normal 依赖；gpui 109 条）。此让步从"纯自写 GUI 不可行"这一现实出发，但代价如实记录。
   - **约束**：只走**纯 Rust** 路径，避开需要 C 工具链的方案（具体指 `syntect` 默认后端的 `onig`，以及 `tree-sitter` 的 `cc` 编译期依赖）。语法高亮用 `syntect` 的 `regex-fancy` 特性。
2. **unsafe 边界**：GUI 依赖含 FFI unsafe（与现有 wry/tao 同理）。"生产代码零 `unsafe`"的承诺**仍然成立**——引入的是依赖而非本项目的不安全代码，这条边界由门禁的 unsafe 扫描持续守护。
3. **工具链升级可能带来新的依赖解析冲突**。届时停下报告，**不用锁文件硬压**。

## 验证方式

- 工具链升级：`scripts/verify.sh` 全绿（617 测试 + 零 warning + 9 组守卫）+ 三宿主真跑（`neo tui` / `neo exec` / `neo serve`）+ `desktop` feature 仍可编译。
- 框架选型：**不靠论证，靠原型实测**（`docs/desktop-plan.md` 阶段 B）——同一份最小需求用两条路各写一遍，量"能否编译 / 依赖条目数 / 编译时长 / 二进制体积 / 实现行数"。
- 桌面实现：T6 宿主等价断言（新宿主必须与 TUI/Web/exec 事实等价）+ 离线桩 provider 端到端 + 视觉验收。

## 不做什么

- **不删除** webview 实现（保留为第二后端）。
- **不引入** GPL 组件：Zed 的 `ui`/`editor`/`markdown`/`diff`/`component` 是 **GPL-3.0-or-later 且未发布到 crates.io**，本项目 MIT，无法复用。`gpui` 框架本体是 Apache-2.0（可用），但组件不能拿。
