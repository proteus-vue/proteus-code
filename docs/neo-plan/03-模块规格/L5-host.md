# L5 · 宿主层规格

**crates**：`neo-host-tui`、`neo-host-desktop`、`neo-host-web`、`neo-host-appserver`、`neo-exec`、`neo-code-cli`
**铁律**：**宿主不含任何业务逻辑，只做 渲染 + 输入解析 + 事件消费**

---

## 一、Electron 的替代：系统 webview，不是「另一个壳」

早期版本只写了一句「Web 宿主不需要 Electron」，但**桌面应用仍需要一个窗口**——那句没回答「那是什么」。

正确做法按 `Proteus方法论-语义核心与后端SPI.md` 第五节：**宿主本身是 SPI**，所以不做二选一，而是定义 `HostBackend` + 给多个后端。

| 宿主后端 | 技术 | 场景 | 二进制量级 |
|---|---|---|---|
| `neo-host-tui` | ratatui + crossterm | 终端 / SSH / CI 旁路 | ~2–5 MB |
| **`neo-host-desktop`** | **wry（系统 webview）** | 桌面 webview 入口（第二后端） | **~5–10 MB** |
| `neo-host-web` | **手写 HTTP/1.1 + SSE（零框架依赖）** | 远程 / 多端浏览器 | ~5–10 MB |
| `neo-host-appserver` | **stdio 上的 JSON-RPC 2.0（标准库）** | 编辑器 / IDE / 脚本接入 | ~0（纯标准库） |
| `neo-exec` | 无头，零交互 | CI / 脚本 / 管道 | ~2–5 MB |

> 表中两处与早期规格不同，均为**实现后的纠偏**：Web 宿主最终没有引 axum/WebSocket，
> 而是手写 HTTP + SSE（理由见 `docs/desktop-plan.md:391`）；新增的 `neo-host-appserver`
> 是"已有自己进程的客户端"的入口（早期规格只列了三个宿主，见 `00-执行摘要.md` 的 M6）。

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
pub trait HostBackend: Send {
    fn id(&self) -> &'static str;
    /// 宿主能力自描述，供内核按需降级（例如 TUI 无法显示图片）。
    fn capabilities(&self) -> HostCapabilities;
    /// 消费一条事件。返回 Err 表示本宿主无法处理该事件 —— T6 断言 (a) 的判据。
    fn consume(&mut self, event: &EventMsg) -> Result<(), String>;
    /// 本宿主已向用户传达的事实集合。**T6 断言 (b) 的比较对象。**
    fn facts(&self) -> Vec<Fact>;
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

> ⚠️ **纠正（2026-09-21 核查）**：本节曾写成一个"启动式"签名
> `fn run(&mut self, events: EventStream, submit: OpSubmitter) -> Result<HostExit, HostError>`。
> 代码里**从来没有**这个签名（`EventStream`/`OpSubmitter`/`HostExit`/`HostError` 四个类型都不存在），
> 实际落地的是上面这个"消费式"契据（`crates/neo-core/src/lib.rs` 的 `HostBackend`）。
> 差异不是笔误而是设计被证伪：`run` 把控制权交给宿主，于是**无法在同一进程里逐条喂事件**，
> 而 T6 的"同一事件流喂多个后端再比对事实"恰恰要求这件事。真正的驱动循环在各宿主的
> `start()` 里（Web/app-server 是"内核独占线程 + 渠道"），不在契据里。

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
- **与前端通信（实际实现，曾是规格漂移点）**：开本地**回环端口**复用 `neo-host-web` 的
  HTTP/SSE 与内置页面，**不**用 `wry` 自定义协议注入资源。取舍已写成设计决定
  （`neo-host-desktop/src/window.rs` 模块注释）：收益是 **T6 宿主等价天然成立**（桌面跑的就是
  Web 宿主，不存在第三套事件消费逻辑要证明等价）+ 零重复界面。
  ⚠️ 原规格写"优先用自定义协议、**不开端口**"，与实现不符，已按实现更正。
- **访问控制**：回环端口**不是**访问控制（本机进程可枚举端口、浏览器任意网页可跨源 POST）。
  窗口加载的是宿主打印的 `page_url()`（带访问令牌的 fragment），令牌见第六节。
- **窗口 / 菜单 / Dock**：由 Rust 侧（`tao` / `winit`）负责
- **视觉**：原规格写"复用现有 React + 液态玻璃"——**该前提已不成立**：那份 React 界面在
  `legacy/`，本 crate 实际加载的是 `neo-host-web` 的单文件页面。本项目已决定另建 Rust 原生
  GUI 宿主（见 `ADR-0006` 与 `docs/desktop-plan.md`），webview 保留为第二后端。

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

## 六、`neo-host-web`（零依赖 HTTP + SSE）

⚠️ 原规格写"axum + WebSocket"，**实际实现与之不符**，按实现更正：

- **HTTP 与 SSE 都是手写的**（`http.rs` / `broadcast.rs`），不引 axum / hyper。理由与本项目
  一贯立场一致：**调试链要浅**（需要的只有解析请求行与头、按 Content-Length 读体、写响应），
  引入生产级 HTTP 栈会带来十几层中间件抽象。事件流用 **SSE**（浏览器原生 `EventSource`）
  而非 WebSocket —— 单向推送够用，且省掉握手与帧协议。
- 端点：`GET /`（内置页面）、`POST /api/turn`、`GET /api/events`（SSE）、
  `GET /api/approve`、`POST|GET /api/goal`、`GET /api/facts`（诊断）。
- **访问令牌（必需）**：启动时生成 256 位随机令牌，**除内置页面外一切路径**都校验
  （含不存在的路径 —— 未鉴权者连路由都枚举不出）。令牌经 query `?token=`（浏览器侧
  唯一通用方式：`EventSource` 设不了请求头）或 `X-Neo-Token` 头传递；宿主打印的
  `page_url()` 把令牌放在 **fragment**，页面从 `location.hash` 取并自动接在每个请求上。
  理由：只绑回环**不是**访问控制（本机进程可枚举端口；浏览器任意网页可跨源 POST
  且请求会生效 —— 端点是简单请求，不触发预检）。见 `auth.rs` 与 `PROJECT_MEMORY §4.61`。
- **有界性**：请求头 / 体 / 并发连接都有上限；广播队列满了**丢弃慢订阅者**（不无限缓冲）；
  订阅者断开由服务端读侧探测（不等下一条事件）。
- 五档执行模式做成显式模式切换器（ZCode UX）
- `@ # / $` 输入解析器（ZCode 交互核心）
- Diff 审批面板：hunk 级接受/拒绝
- **诚实边界**：内置页面刻意极小（证明"同一内核、多宿主"不是纸面说法），不是产品级 UI；
  令牌是单进程生命周期的一次性令牌，无用户/角色概念；`--addr` 对外监听时流量仍是明文 HTTP。

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

- [x] 五宿主（TUI / desktop / web / app-server / exec）消费同一 `EventMsg` 流，
      业务逻辑零重复（T6 机器校验：`neo-mock/tests/conformance.rs`）
- [x] 宿主 crate 间无相互依赖（`check_architecture` 的 A3）
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
