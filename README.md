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
| SPI 契约（5 个 seam + Mock 后端 + conformance） | ✅ **已实现**，10 项 |
| 协议 / 配置 / 会话格式 | 🟡 类型与校验已有，JSONL 持久化**未接线** |
| 真实模型 provider | ❌ **未实现**（只有 mock，内核尚无真机验证） |
| 工具执行（bash 真实执行、apply_patch 落盘） | ❌ **未实现**（契约与分类已定） |
| 宿主后端（TUI / Desktop / Web / Exec） | ❌ **未实现**（契据 `HostBackend` 已定） |
| Goal 编排（L4） | ❌ **未实现** |
| OS 沙箱（Seatbelt / Landlock+bwrap） | 🟡 命令包裹已写，未接真实执行 |

**一句话现状**：机制已验证（32 个测试），但还没有真实模型与真实执行。
下一步是**接真实 provider + 真实 bash 执行**，让内核跑通一轮真任务。

---

## 快速开始

```bash
# 需要 Rust（stable）
cargo test --workspace     # 32 个测试：内核 16 + 内存 6 + SPI conformance 10
cargo check --workspace    # 14 个 crate，零 unsafe、零 warning

bash scripts/verify.sh     # 全套门禁：架构守卫 / 协议 / 会话 / 配置 / 模式矩阵 / SPI / 测试
```

## 仓库结构

```
proteus-code/                  ← 项目本体是 Rust 内核
├── Cargo.toml                 workspace（14 crates）
├── crates/                    ★ 内核与宿主
│   ├── dsh-protocol/          L0 线协议（Op / EventMsg / 双轴枚举），零业务依赖
│   ├── dsh-sandbox/           L1 平台（命令包裹：Seatbelt / Landlock+bwrap / ACL）
│   ├── dsh-platform/          L1 平台（进程加固 / fs notify / git）
│   ├── dsh-core/              ★ L2 内核：turn/step 主循环 + 三维闸门 + 4 个 SPI 契据
│   ├── dsh-capability/        L3 能力（Shell-First 工具集：bash / apply_patch / 提问）
│   ├── dsh-orchestration/     L4 目标编排（Goal 引擎）
│   ├── dsh-host-tui/          L5 宿主：终端
│   ├── dsh-host-desktop/      L5 宿主：系统 webview（替代 Electron）
│   ├── dsh-host-web/          L5 宿主：浏览器
│   ├── dsh-exec/              L5 宿主：无头 / CI
│   ├── dsh-cli/               `neo` 入口（multitool）
│   ├── dsh-session/           会话真相源（append-only）
│   ├── dsh-config/            四级配置 + 模式解析
│   └── dsh-mock/              test-support：各 SPI 的 Mock 后端 + 反例后端
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
| `ModelProvider` | 换模型（流式增量，支持工具调用） | mock / scripted | ✅ |
| `SandboxBackend` | 换沙箱实现 | mock ×2 | ✅ |
| `SessionPersistence` | 换会话存储介质 | in-memory / tampering(反例) | ✅ |
| `HostBackend` | 换宿主（TUI/Desktop/Web/Exec） | desktop / mock ×2 | ✅ |
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
