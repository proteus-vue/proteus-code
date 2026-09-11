# NEO —— 用 Rust 重构的编程 Agent 内核

> **一句话**：以 DeepSeek Harness（DSH）为起点，吸收 **ZCode（Z.ai）** 的交互范式与
> **OpenAI Codex CLI** 的运行时架构，用 **Rust** 重写内核，交付
> **TUI / Desktop（系统 webview）/ Web / Exec 四宿主共享同一内核** 的编程 Agent。
>
> **为什么是 Rust**：为了机制层面的两个硬要求 —— **高性能**（无 GC 停顿、零拷贝主循环）
> 与**内存安全**（零 `unsafe`、类型层面无数据竞争）。附带收益是去掉过重的壳
> （Electron ~78 MB → 系统 webview ~5–10 MB）。

---

## 当前状态（诚实标注）

| 层 | 状态 |
|---|---|
| 内核主循环（turn/step、审批挂起恢复、三维闸门） | ✅ **已实现**，16 项 conformance |
| 内存有界性（输出上限、UTF-8 安全截断、上下文上限） | ✅ **已实现**，6 项实测 |
| `ModelProvider` 真实后端（DeepSeek chat-completions） | ✅ **已实现**，HTTP 帧对真实服务器验证通过 |
| `SandboxBackend` 真实后端（macOS Seatbelt） | ✅ **已实现**，6 项**真机**越权拦截测试 |
| `SessionPersistence` 真实后端（JSONL append-only） | ✅ **已实现**，重开续号已验证 |
| `neo exec` 无头宿主（端到端闭环） | ✅ **已实现**，离线 + 真实 provider 双路径跑通 |
| `neo tui` 终端宿主（MiMo 风格 + @// 弹窗 + !shell + 审批 diff + 右侧面板 + Markdown 高亮 + 6 主题 + 信任门） | ✅ **已实现**，pty 逐项验证（含宽/窄终端下的面板与高亮） |
| `neo serve` Web 宿主（浏览器界面 + SSE 事件流 + 审批） | ✅ **已实现**，端到端验证（订阅→提交→审批→真落盘） |
| **T6 宿主语义等价**（同一事件流多宿主比对） | ✅ **铁律生效**：headless / TUI / desktop / **web** 四宿主事实完全等价 |
| Shell 工具真实执行（经沙箱） | ✅ **已实现**（`bash` 真跑，输出受限） |
| **真实模型完整推理** | ⚠️ **待验**：HTTP 链路已验证，但环境里的 `DEEPSEEK_API_KEY` 是占位符，未完成一次真实推理 |
| `apply_patch` 落盘 | ✅ **已实现**（经 `ctx.write_file` 走沙箱，唯一匹配校验，真机验证落盘） |
| Web 宿主（零依赖 HTTP + SSE） | ✅ **已实现**（`neo-host-web`；`POST /api/turn` 提交、`GET /api/events` SSE、`/api/approve` 审批） |
| Desktop 宿主（系统 webview 壳） | 🟡 **契据就绪**：`DesktopHost` 的 `HostBackend` 实现与 T6 覆盖已完成；wry window 层未接（原型阶段不拉入平台图形栈） |
| Goal 编排（L4） | ❌ **未实现** |
| Linux / Windows 沙箱 | ❌ **未实现**（**fail-closed**：受限档位拒绝执行，不降级放行） |

**一句话现状**：内核 + 真实沙箱 + 真实模型链路 + 落盘 + **三宿主（exec / TUI / Web）**
已闭环并跑通，T6 宿主等价铁律在四宿主上生效；剩 Desktop 的系统 webview 窗口层与
真实模型推理待验（后者卡在环境里的 key 是占位符）。

### 离线自验（不需要 API key）

三个内置桩 provider，各自覆盖不同链路。**没有 key 也能把界面与内核交互全走一遍**：

| provider | 它产出什么 | 能验证到哪些功能 |
|---|---|---|
| `mock` | 一句固定文本 | 信任门 · 首页 · `@` 文件弹窗（过滤/选择）· `/` 命令弹窗 · `/help` `/keys` `/status` · `ctrl+p` 面板 · `ctrl+t` 主题 · `ctrl+b` 侧栏 · `ctrl+r` 历史 · `!shell` 执行 · 普通对话 |
| `selftest` | 调一次 `apply_patch` 写文件 | 审批拦截 → **审批前 unified diff 预览** → `y` 批准 → 真实落盘 → 侧栏 Modified Files（+N -N） |
| `demo` | Markdown 正文 + `todowrite` | Markdown 渲染（标题/列表/行内代码/代码块**语法高亮**/引用）· 正文与侧栏的**任务清单**进度 |

```bash
neo tui --provider mock        # 界面与交互全览
neo tui --provider selftest    # 审批 / diff 预览 / 落盘（批准后会写 ./selftest.txt）
neo tui --provider demo        # Markdown 高亮 + 任务清单
```

**离线验不到的**（必须真实模型）：真实推理质量、多轮工具编排、
模型是否真的会调 `todowrite` 维护清单、`/compact`（未实现）。
界面本身与 provider 无关，所以这些以外的交互都能离线确认。

### 亲测可用

**先装到 PATH**（否则 `neo` 提示 command not found —— 它只在 `target/` 里）：

```bash
cargo install --path crates/neo-cli --locked   # 装到 ~/.cargo/bin/neo
```

```bash
# 交互式（TUI）：首次进入某目录会先问"是否信任"（落盘 ~/.neo/trusted.json）
# 首页：星场 + 居中 logo + 带边框输入框（内含 模式 ⏵ 模型）
#   @          文件弹窗（↑↓ 选择 / 实时过滤 / Enter 确认）
#   /          命令弹窗（help / keys / status / theme / new / compact / exit）
#   审批时会先显示 unified diff（不再盲批）
#   右侧面板：Context（token 占用）/ Todo（进度）/ Modified Files（+N -N）
#   ctrl+b 开关侧栏（<96 列自动隐藏）
#   !cmd       直接执行 shell（走同一沙箱），输出进会话
#   ctrl+p     命令面板    ctrl+t 切换主题（6 套，落盘记忆）
#   @file#12-40 引用文件的行范围
#   助手回复按 Markdown 渲染（代码块语法高亮、行内代码、标题、列表）
#   Tab 补全 / ctrl+r 历史 / ctrl+g 编辑器 / ctrl+l 清屏 / esc 关弹窗
neo tui --provider selftest --mode default
# 等价写法：neo --provider selftest --mode default（裸选项默认进 TUI）

# Web 宿主：浏览器打开 http://127.0.0.1:8787（SSE 实时事件流 + 页面内审批）
neo serve --provider selftest --mode default

# 离线跑通（不需要 API key）—— 验证装配 → 内核 → 沙箱 → 落盘
cd /tmp && neo exec "列出文件" --provider mock --mode auto-edit

# 接真实模型（需要一个有效的 DEEPSEEK_API_KEY）
export DEEPSEEK_API_KEY=sk-...
neo exec "用一句话回答 1+1" --mode plan
```

## 快速开始

```bash
# 需要 Rust（stable）
cargo run -p neo-cli -- tui --provider mock    # 不装 PATH，直接用 cargo 跑 TUI
cargo install --path crates/neo-cli --locked   # 或装成全局命令 neo

cargo test --workspace     # 212 个测试（内核 conformance / 内存有界性 / SPI / 宿主 / TUI / Web）
cargo check --workspace    # 17 个 crate，零 unsafe、零 warning

bash scripts/verify.sh     # 全套门禁：架构守卫 / 协议 / 会话 / 配置 / 模式矩阵 / SPI / 测试
```

## 仓库结构

```
proteus-code/                  ← 项目本体是 Rust 内核
├── Cargo.toml                 workspace（17 crates）
├── crates/                    ★ 内核与宿主
│   ├── neo-protocol/          L0 线协议（Op / EventMsg / 双轴枚举），零业务依赖
│   ├── neo-sandbox/           L1 平台（命令包裹：Seatbelt / Landlock+bwrap / ACL）
│   ├── neo-platform/          L1 平台（进程加固 / fs notify / git）
│   ├── neo-core/              ★ L2 内核：turn/step 主循环 + 三维闸门 + 4 个 SPI 契据
│   ├── neo-capability/        L3 能力（Shell-First 工具集：bash / apply_patch / 提问）
│   ├── neo-orchestration/     L4 目标编排（Goal 引擎）
│   ├── neo-host-tui/          L5 宿主：终端
│   ├── neo-host-desktop/      L5 宿主：系统 webview（替代 Electron）
│   ├── neo-host-web/          L5 宿主：浏览器（零依赖 HTTP + SSE，含内置页面）
│   ├── neo-exec/              L5 宿主：无头 / CI
│   ├── neo-cli/               `neo` 入口（multitool）
│   ├── neo-session/           会话真相源（append-only）
│   ├── neo-config/            四级配置 + 模式解析
│   └── neo-mock/              test-support：各 SPI 的 Mock 后端 + 反例后端
├── docs/
│   ├── neo-plan/              ★ 设计方案：架构 / 模块规格 / 里程碑 / 验证套件
│   │   ├── 02-架构设计/       ★ 方法论与 SPI 纲领（先读这个）
│   │   ├── 03-模块规格/       逐层可落地规格
│   │   ├── 04-落地计划/       M0–M6 里程碑与验收
│   │   └── 05-验证/           可执行验证套件（Python + golden）
│   └── spi-first-methodology/ 方法论原文（跨 16 次生产泛化）
├── scripts/verify.sh          全套门禁入口
└── legacy/                    旧实现（Electron + DSH 插件），仅作参考
```

> **`legacy/` 是什么**：本项目早期用 TypeScript 写的「DSH 插件 + Electron 壳」实现。
> 它**可运行**（129 个测试、真机验证过），但方向已改为 Rust 内核，故整体退到
> `legacy/` 作为**设计参考与经验来源**（哪些坑会随重写消失、哪些不会）。

---

## 设计立场（三句话）

1. **不是「一切皆插件」，但也不是「写死内核」。**
   DSH 的病不是插件太多，而是**约 90 个 seam 没有名字、没有门禁** ——
   无法静态分析、无法验证可替换性。NEO 把它们**收敛为 5 个有名 SPI**，
   每个都强制「契约 + ≥2 后端 + conformance」。这样**同时得到可替换性与可定位性**。

2. **宿主只是内核的一个消费者。**
   TUI / Desktop / Web / Exec 不持有业务状态，只消费同一条事件流（Op / EventMsg）。
   这是 Codex 多宿主设计的精髓，也是「改 UI 不动内核」的前提 ——
   并用 **T6 铁律**机器验证（同一事件流喂多个宿主后端，断言语义等价）。

3. **安全是正交双轴，且沙箱是内核的结构保证。**
   沙箱决定「能做什么」，审批决定「何时必须问」。二者必须独立配置。
   更关键的是：`Tool::execute` 接收由内核注入的 `ToolCtx`，**工具在类型层面就没有
   绕开沙箱的入口** —— 安全边界不能靠约定。

---

## 五个 SPI（可替换性的全部出口）

| SPI | 语义定义 | 已实现后端 | conformance |
|---|---|---|---|
| `ModelProvider` | 换模型（流式增量，支持工具调用） | deepseek / scripted / mock ×2 | ✅ |
| `SandboxBackend` | 换沙箱实现 | local(Seatbelt 真机) / mock ×2 | ✅ |
| `SessionPersistence` | 换会话存储介质 | jsonl(真落盘) / in-memory / tampering(反例) | ✅ |
| `HostBackend` | 换宿主（TUI/Desktop/Web/Exec） | desktop / tui / web / mock ×2 | ✅ 4 宿主 T6 等价 |
| `Tool` | 加能力（Shell-First） | 3 内置 + mock ×4 | ✅ |

**每个 SPI 都配了「坏后端」作为负向用例的被试** —— 一个不能被 conformance 抓住的
反例，等于套件没有牙齿。

---

## 文档导航

| 文档 | 内容 |
|---|---|
| [docs/neo-plan/README.md](docs/neo-plan/README.md) | 设计方案总览（先读执行摘要） |
| [docs/neo-plan/02-架构设计/Proteus方法论-语义核心与后端SPI.md](docs/neo-plan/02-架构设计/Proteus方法论-语义核心与后端SPI.md) | **方法论纲领**：为什么 5 个 SPI、为什么 Rust |
| [docs/neo-plan/03-模块规格/](docs/neo-plan/03-模块规格/) | 逐层规格（L0–L5） |
| [docs/neo-plan/04-落地计划/](docs/neo-plan/04-落地计划/) | M0–M6 里程碑与验收标准 |
| [docs/spi-first-methodology/](docs/spi-first-methodology/) | SPI-First 方法论原文与自检清单 |
| [PROJECT_MEMORY.md](PROJECT_MEMORY.md) | 项目记忆：决策依据、踩过的坑、被证伪的方案 |
| [AGENTS.md](AGENTS.md) | 在本仓库工作的约定（效率标准 + Rust 约束） |

---

## 协议

Apache-2.0。DSH 为 MIT。
