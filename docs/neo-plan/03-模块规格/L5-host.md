# L5 · 宿主层规格

**crates**：`neo-host-tui`、`neo-host-desktop`、`neo-host-web`、`neo-exec`、`neo-code-cli`
**铁律**：**宿主不含任何业务逻辑，只做 渲染 + 输入解析 + 事件消费**

---

## 一、Electron 的替代：系统 webview，不是「另一个壳」

早期版本只写了一句「Web 宿主不需要 Electron」，但**桌面应用仍需要一个窗口**——那句没回答「那是什么」。

正确做法按 `Proteus方法论-语义核心与后端SPI.md` 第五节：**宿主本身是 SPI**，所以不做二选一，而是定义 `HostBackend` + 给多个后端。

| 宿主后端 | 技术 | 场景 | 二进制量级 |
|---|---|---|---|
| `neo-host-tui` | ratatui + crossterm | 终端 / SSH / CI 旁路 | ~2–5 MB |
| **`neo-host-desktop`** | **wry（系统 webview）** | **桌面主入口** | **~5–10 MB** |
| `neo-host-web` | axum + WebSocket | 远程 / 多端浏览器 | ~5–10 MB |
| `neo-exec` | 无头，零交互 | CI / 脚本 / 管道 | ~2–5 MB |

对比 Electron 的 **~78 MB+**：系统 webview 既不捆绑 Chromium，也不捆绑 Node。

### 为什么选系统 webview 而不是纯原生 GUI

> ⚠️ **本节的前提已失效（2026-09-14 核查）**：原文的理由是"**保住已有投资**：现有
> React + 液态玻璃界面在 webview 里原样可用"。但那套 React 界面在 `legacy/`，
> **本 crate 实际加载的是 `neo-host-web` 那份 184 行内联页面**（无 React、
> 无液态玻璃），`crates/` 里对 `legacy/` 的引用数为 **0**。
> 也就是说 wry 的"保住已有 UI"收益实际为 0，而"重写 UI"的代价早已发生。
>
> **现行决策见 [`ADR-0006-桌面原生GUI.md`](../02-架构设计/ADR/ADR-0006-桌面原生GUI.md)**：
> 新增 Rust 原生 GUI 宿主，wry/webview **保留为第二 `HostBackend`**（不押注单一方案）。
> 实现计划见 [`docs/desktop-plan.md`](../../desktop-plan.md)。
> 下表保留原始权衡记录，供追溯。

| 方案 | 壳重 | 保住现有 UI | 代价 |
|---|---|---|---|
| **wry（系统 webview）** | ~1/10 | ✅ 零改动（**前提已失效**，见上） | 依赖 OS webview；三平台有差异 |
| egui / iced（纯 Rust GUI） | 最小 | ❌ 须用 Rust 重写 UI | 液态玻璃要自己写着色器 |
| gpui（Zed 的 GPU UI） | 最小 | ❌ 同样重写 | 生态较新；组件为 GPL 不可复用 |

**原推荐**（wry 作主入口、egui 作未来第二后端）**已成为现行方案**，只是主次对调：
原生 GUI 作主入口，webview 作第二后端。

---

## 二、`HostBackend`：SPI 契据

宿主是**后端实现细节**，内核只认这个契据。

```rust
/// 宿主后端：消费内核事件流，提交内核 Op。
/// 实现在 L5；契据在 L0/L2。宿主之间不得互相依赖。
pub trait HostBackend {
    /// 启动宿主，直到用户退出或内核终止。
    ///
    /// - `events`: 内核事件流（唯一事实来源），宿主只读消费
    /// - `submit`: 提交 Op 的句柄（唯一写入口）
    ///
    /// 实现**不得**持有 Session 状态副本；渲染所需状态一律由事件流推导。
    fn run(
        &mut self,
        events: EventStream,
        submit: OpSubmitter,
    ) -> Result<HostExit, HostError>;

    /// 宿主能力自描述，供内核按需降级（例如 TUI 无法显示图片）。
    fn capabilities(&self) -> HostCapabilities;
}

/// 宿主能力声明：让工具知道「呈现形式」的可选项，
/// 而不是让工具去猜宿主是谁。
pub struct HostCapabilities {
    pub images: ImageSupport,      // 无 / 内联 / 外链
    pub rich_text: bool,           // 是否支持 markdown 与 ANSI 之外的样式
    pub interactive_prompt: bool,  // 能否弹交互式审批（exec 为 false）
    pub diffs: DiffSupport,        // 无 / 文本 / hunk 级
}
```

**`HostCapabilities` 为什么必要**：没有它，工具就得写 `if is_tui { … } else { … }`——那正是 Proteus 反对的「业务代码里出现平台分支」。有了它，**降级是数据驱动的**：工具输出规范值 + 能力声明，宿主自行选最合适的呈现。

---

## 三、共同约束

1. 只通过 `submit(Op)` / `next_event()` 与内核通信
2. 不得持有 Session 状态副本（唯一真相源在 L2 + JSONL）
3. 宿主 crate 之间**不得互相依赖**（`check_architecture` 校验）

---

## 四、`neo-host-desktop`（系统 webview）

- **技术**：`wry`（Tauri 的 webview 层，可单独使用，不必引 Tauri 全家桶）
- **形态**：**单二进制**——Rust 内核 + 内嵌 web 资源。无 Node、无 pnpm、无 profile 安装
- **与前端通信**：优先用 `wry` 自定义协议直接注入资源（**不开端口**，比回环服务更安全）；仅 `neo-host-web` 才起 axum 回环服务
- **窗口 / 菜单 / Dock**：由 Rust 侧（`tao` / `winit`）负责
- **视觉**：现有 React + 液态玻璃**直接复用**

**平台 webview 矩阵（诚实边界）**：

| 平台 | 引擎 | 是否需额外安装 | 已知风险 |
|---|---|---|---|
| macOS | WKWebView | 系统自带 | `backdrop-filter` 有历史 bug，需实测 |
| Windows | WebView2 (Chromium) | Win11 自带；**Win10 需装运行时** | 须检测并提示安装 |
| Linux | WebKitGTK | **需系统包** | 发行版差异大，打包须声明显示依赖 |

> **不得声称三平台视觉一致。** 只能声称「同一事件流下语义等价」。

---

## 五、`neo-host-tui`（ratatui + crossterm）

### 快捷键（对齐 Codex）
| 键 | 功能 |
|---|---|
| `Ctrl+L` | 清屏（保留历史） |
| `Ctrl+O` | 复制最新输出到剪贴板 |
| `Ctrl+R` | 搜索提示词历史 |
| `Ctrl+G` | 用 `$EDITOR` 打开外部编辑器 |
| `Tab` | 运行中排队下一轮输入（不打断） |
| `Esc Esc` | 编辑上一条消息 |
| `Shift+Tab` | 循环切换五档执行模式（ZCode） |
| `@` | 模糊文件搜索 |

### Slash 命令
`/goal` `/review` `/model` `/fork` `/permissions` `/compact` `/diff` `/status` `/agent <name>` `/debug-config`

### 方法论价值
TUI **无法显示图片、无法弹富交互**，因此是「内核是否真的与宿主解耦」的**最严苛试金石**。若 TUI 能完整跑通一次真实任务，说明内核没有偷偷依赖 Web 能力。

---

## 六、`neo-host-web`（axum + WebSocket）

- 渲染层任意前端框架（React/Vue/Svelte），经 WS 消费事件流
- **内核编译为单二进制后 `neo serve` 即本地服务**，浏览器直连；不需 Electron 也不需 webview
- 五档执行模式做成显式模式切换器（ZCode UX）
- `@ # / $` 输入解析器（ZCode 交互核心）
- Diff 审批面板：hunk 级接受/拒绝

---

## 七、`neo-exec`（无头 / CI）

```
neo exec "fix failing tests" --json --sandbox workspace-write
```
零交互、可脚本化、可管道化。CI 的第一公民。

---

## 八、`neo` 入口（multitool）

```
neo                 → TUI（默认，最快）
neo desktop         → 桌面（系统 webview）
neo exec "<task>"   → 无头
neo serve           → Web 宿主
neo mcp-server      → 暴露为 MCP server
neo resume          → 从 checkpoint 恢复
```

---

## 九、为什么宿主必须是 SPI

Proteus 明说：**单个角色不构成 seam**。宿主只有一种实现时，「换 UI 不动内核」只是**未经验证的宣称**。

故要求 `HostBackend` **至少两个真实后端**（TUI + desktop），并用 **T6 铁律**机器验证：

> **T6 宿主语义等价**：把同一段记录的 `EventMsg` 流分别喂给 ≥2 个宿主后端，
> 断言 (a) 都能消费完，(b) 产出等价的用户可见事实集合，(c) 宿主 crate 之间零依赖。

这条就是 Proteus conformance 门禁在宿主层的落地：把「可替换」从口号变成 CI 能判的事实。

---

## 九点半、现状与目标状态的差距（SPI-First 审计结论）

耦合审计实测（见 [`05-验证/spi-first-audit.md`](../05-验证/spi-first-audit.md)）：

| | 现状 | 目标 |
|---|---|---|
| Electron 耦合 | **2 个文件**（`main.ts` 21 处 API、`probe.ts` 4 处） | 0（全部收进 `neo-host-desktop`） |
| `HostBackend` 实现 | **3 个**（`DesktopHost` 原型 + 2 个 Mock） | `tui` / `desktop` / `web` / `exec` 四个真实后端 |
| conformance | 已有（10 用例，含 3 负向） | 保持 + 接 CI |

**好消息**：Electron 只渗进 2 个文件，说明壳层**本来就已经隔离好了**。
`HostBackend` 的工作是把这条**已存在的好边界形式化**，而不是新建边界——改造风险很低。

**待修**：AP-04（业务绕过接口直调底层）的现存漏洞就在这 2 个文件里
（`import { app, BrowserWindow, dialog, shell } from 'electron'`），
是下一步试点的首选目标。

---

## 十、验收

- [ ] 四宿主消费同一 `EventMsg` 流，业务逻辑零重复（T6 机器校验）
- [ ] 宿主 crate 间无相互依赖（`check_architecture`）
- [ ] **桌面宿主不含 Electron、不捆绑 Chromium/Node**（体积门禁 < 15 MB）
- [ ] 单二进制：无 Node、无 pnpm、无 profile 安装步骤
- [ ] TUI 能完整跑通一次真实任务（证明内核与 Web 能力解耦）
- [ ] 平台 webview 缺失时给出**可操作的安装提示**，不是崩溃

---

## 十一、这一改动顺带消灭的复杂度

| 当前（Electron + Node 内核） | 换成 Rust 后 |
|---|---|
| DSH 包的 `file:` / symlink 解析陷阱 | **消失**（无 Node 包解析） |
| profile / pnpm / bundle 层叠 | **消失**（编译期静态链接） |
| Node ≥22 要求、nvm/Electron Node 回退链 | **消失**（单二进制） |
| bundle 改动须重启才生效 | **消失**（无运行时装载） |
| Electron 78 MB 壳 | **~5–10 MB** |

**但两件事不会消失**，必须诚实标注：

1. **DOM/CSS 的复杂度仍在**（webview 就是浏览器）。液态玻璃的 `backdrop-filter` 包含块副作用、哈希类名不可选等坑**依然存在**——这些是本会话实测过的。要彻底去掉，只能换 egui，代价是重写 UI。
2. **沙箱的三平台差异仍在**，且是 NEO 最重的部分（见方法论文档第六节）。
