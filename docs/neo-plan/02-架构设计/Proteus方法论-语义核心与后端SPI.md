# Proteus 方法论应用到 NEO 内核

> **本文是 NEO 的方法论纲领。** 它纠正本计划早期「固定内核 + 4 类扩展点」的表述——那个表述容易被读成「把 DSH 的插件全删掉、写死成一段大代码」，那是**另一种极端，同样错**。
>
> 正确的框架来自 Proteus：**统一语义收敛**。

---

## 一、方法论回顾：统一语义收敛

Proteus 的核心公式：

```
任何跨端问题 = 语义定义（框架做） + 后端实现（平台做）
```

框架只做两件事：

1. **定义「你要什么」** —— 语义接口 / IR
2. **定义「怎么验证做对了」** —— conformance / 铁律 / 编译期约束

平台只做一件事：提供「怎么做」（Backend 实现）。**业务只消费语义接口，对后端零感知。**

### 一个 seam 的三个角色

Proteus 明确：**单个角色不构成 seam**。一个可替换能力必须同时有：

| 角色 | 职责 | 缺了会怎样 |
|---|---|---|
| **Service Definition** | 声明接口与语义 | 没有契约，实现各自为政 |
| **Service Provider** | 实现它（**≥2 个**） | 只有一个实现 = 假 SPI，替换性未经验证 |
| **Consumer** | 通过接口消费 | 没有消费者，接口是死代码 |

---

## 二、为什么这套方法论正好治 DSH 的病

**DSH 的问题不是「插件太多」，而是「seam 没有名字、没有门禁」。**

| | DSH | 纠偏后的 NEO |
|---|---|---|
| seam 数量 | 约 90 个（每个 workspace 包一个） | **5 个 SPI**，每个有名字 |
| seam 有定义吗 | 有（Cordis service） | 有（Rust trait） |
| 有 ≥2 后端吗 | 部分有 | **强制**，CI 校验 |
| 有 conformance 吗 | 部分（G-xx 套件） | **强制**，每个 SPI 一套 |
| 依赖可静态分析吗 | **否**（运行时服务查找） | **是**（crate 依赖图 + 守卫） |

**关键洞察**：把 90 个无名 seam 收敛成 5 个有名 seam，**既得到可替换性，又得到可定位性**——这正是 DSH 缺失的那一半。

> 所以「不要一切皆插件」的正确表述是：**seam 要少、要有名字、要有门禁**；
> 不是「不要 seam」。写死内核反而是把可替换性也一起丢掉。

---

## 三、NEO 的语义核心（IR / 契约）

语义核心是**平台无关的「要什么」**，位于 L0/L2，不依赖任何后端。

| 语义对象 | 定义处 | 它固定了什么（platform-independent） |
|---|---|---|
| **Op / EventMsg** | `dsh-protocol` (L0) | 宿主→内核的意图、内核→宿主的事实。**唯一的跨层契约** |
| **Turn / Step 状态机** | `dsh-core` (L2) | 一轮 = 0..N 步；步 = 一次模型请求 + 其工具调用 |
| **SessionEvent** | `dsh-session` | append-only 真相源；模型可见即已落日志 |
| **Tool 契约** | `dsh-capability` (L3) | 名 / 描述 / 参数 schema / 规范返回值 / 呈现投影 |
| **Approval 决策** | `dsh-protocol` | `allow` / `ask` / `deny` + 理由（**不是**「怎么问」） |
| **SandboxMode** | `dsh-protocol` | 三档语义：read-only / workspace-write / danger-full-access |
| **Context 装配** | `dsh-core` | 哪些内容进请求、以什么顺序、（缓存友好）如何复用前缀 |

**约束挂在 IR 上，不挂在平台上。** 这与 Proteus 一致——也是 AI Agent 能安全介入的原因：它操作 IR，IR 上有铁律。

---

## 四、五个 SPI（每个都有定义 / ≥2 后端 / conformance）

| # | 语义定义（L0/L2） | SPI | 后端（≥2） | Conformance 断言 |
|---|---|---|---|---|
| S1 | `ModelProvider` + 流式事件词汇 | `ModelProvider` | `deepseek` / `openai-compat` / `mock` | 同 Op 序列 → 同 EventMsg 序列（流式块可重放） |
| S2 | `SandboxMode` + `SandboxRequest` | `SandboxBackend` | `seatbelt`(macOS) / `landlock+bwrap`(Linux) / `noop`(测试) | 越权写**必须被拒**；拒绝理由是结构化事实 |
| S3 | `SessionLog`（append-only 语义） | `SessionPersistence` | `jsonl` / `in-memory` | 任意日志可**完整重建状态**；append-only 不被破坏 |
| S4 | `HostSurface`（消费事件 + 提交 Op） | `HostBackend` | `tui` / `desktop` / `web` / `exec` | 同一事件流 → 各宿主语义等价；宿主间零业务逻辑重复 |
| S5 | `Tool` 契约 + `apply_patch` 信封 | `ToolTransport` | `in-process` / `mcp` | 工具契约（schema 校验、规范返回值）跨 transport 一致 |

**注意 S4/S5 的边界**：`Tool` 的**契约**在 L3（语义），**传输**（进程内 / MCP）才是 SPI。这是 Proteus 说的三层角色切分——契约与实现分离。

---

## 四点半、可执行形态：SPI-First

上面「一、方法论回顾」是**原则**；`spi-first` skill 是它的**可执行形态**（LLM 逐阶段执行）：

| 阶段 | 动作 | 本项目落点 |
|---|---|---|
| Phase 0 | 确认目标与红线 | 审计 + 试点；红线=不拆现有可用实现 |
| Phase 1 | **grep 量化耦合点**，产 P0/P1/P2 候选表 | 见 [`05-验证/spi-first-audit.md`](../05-验证/spi-first-audit.md) |
| Phase 2 | 汇报候选、选定试点（小而完整） | 选定 5 个 SPI 的接口 + Mock + conformance |
| Phase 3 | **五步改造**：语义接口 → ≥2 后端（含 Mock）→ conformance → 接线守界 → 诚实边界 | 已执行 |
| Phase 4 | 验证 + **8 反模式自检** | 3 条预警如实保留（AP-04/07/08） |
| Phase 5 | 交付报告（含未做之事） | `spi-first-audit.md` |

**8 条反模式**是这套方法论的牙齿，逐条对照（详见 `anti-patterns.md`）：

| | 反模式 | 本质 |
|---|---|---|
| AP-01 | 只有 1 个实现 | 可替换性从未验证 |
| AP-02 | 接口含技术名词 | 换实现时接口本身要改 |
| AP-03 | 无 conformance | 行为随时间漂移 |
| AP-04 | 业务绕过接口直调底层 | 抽象有漏洞=没抽 |
| AP-05 | 只定义类型不定义行为 | 各实现行为天差地别 |
| AP-06 | 为「可能换」过度设计 | 间接层成本 > 收益 |
| AP-07 | 后端爆炸（6+） | 契约演进成负担 |
| AP-08 | 抽象成本不透明 | 从不做性能实测 |

**本项目首次执行的三个意外收获**（详见审计报告）：

1. **假门禁比没门禁更糟**：门禁首版用字符串搜索判断后端数，在 `HostBackend` 只有
   1 个实现时报了 4 个——**给出假保证**。已改为只认 `impl X for` 语法事实。
2. **负向用例会抓到自己**：坏后端首版只在特定输入下违规，导致负向用例自身失败——
   恰好证明负向用例在起作用。
3. **新增契约指纹漂移检查**：`spi_contract_digests.json`，契约口径静默变化即报错。

---

## 四点半二、为什么内核必须是 Rust（两个目标，不是一种偏好）

选 Rust 的动因是**机制层面的两个硬要求**，不是语言偏好：

### 目标一：高性能

| 隐患（在其他运行时里） | Rust 的对策 | 本项目实测 |
|---|---|---|
| 每步深拷贝整份对话历史 → 一轮 O(N²) | 借用切片 + `mem::take` 移动所有权 | ✅ 计数分配器实测：历史 20 条 vs 400 条，额外分配差 **< 16 KB**（若深拷贝应达数十 KB） |
| 系统提示词每步重新拼接 | 构造时算一次并缓存 | ✅ 提示词字节稳定的同时零重复分配 |
| 工具 schema 每次请求重新分配 | 同上，构造时缓存 | ✅ |
| 每调用一次克隆工作目录 PathBuf | `ToolCtx` 持 `&Path` | ✅ 零克隆 |

**没有 GC 停顿**是附带收益：agent 主循环是延迟敏感路径，一次 stop-the-world
会体现为流式输出的可见卡顿。

### 目标二：内存安全

| 层次 | 保证 | 本项目状态 |
|---|---|---|
| 无 UB | 零 `unsafe`（全 13 crate） | ✅ 已审计，`grep -c unsafe` 为 0 |
| 无数据竞争 | `Send`/`Sync` 由编译器强制；跨线程共享走 `Arc` | ✅ 类型层面保证 |
| 无悬垂/双重释放 | 所有权 + 生命周期 | ✅ |

### 但必须诚实：Rust 不保证「内存**有界**」

这是最容易被跳过的一点。Rust 消除 UB，但**一条 `yes` 或 `find /` 能把进程撑爆** ——
完全安全的 Rust 代码照样 OOM。所以有界性是**内核的义务**，且必须自己测：

| 有界性措施 | 实测断言 |
|---|---|
| 单次工具输出上限（默认 256 KB，**内核级**，工具不可放宽） | 4 MB 输出 → 模型看到 ≤ 上限且 `truncated=true` |
| 截断不切开 UTF-8 码点 | 上限取 1/2/4/5/10/100/2999 全部通过（直接 `&s[..n]` 会 panic） |
| 沙箱层截断必须上抛 | 沙箱已声明截断时，内核不得抹成 false |
| 上下文消息上限（默认 4096） | 超限**报错要求压缩**，绝不静默丢消息 |
| 步数预算 / 单步调用上限 | 模型失控时截断，不死循环 |

> **为什么上下文超限选择报错而非丢弃最老消息**：静默丢弃会让模型的视角与
> 会话日志不一致，破坏"模型可见即已落盘"的铁律 —— 那比报错危险得多。
> 压缩是 L4 orchestration 的职责（`Compact` Op），当前**未实现**。

### 顺带的风险消除（去掉 Electron 与 Node 之后）

`file:`/symlink 解析陷阱、profile/pnpm/bundle 层叠、Node ≥22 运行时要求、
bundle 改动须重启 —— 全部消失（编译期静态链接、单二进制）。
但 **DOM/CSS 复杂度仍在**（webview 就是浏览器），**OS 沙箱三平台差异仍在**。

---

## 五、编译期优先（Proteus 的「先知」原则）

Proteus 要求「能编译期发现的问题绝不留到运行时」。Rust 在这里是**方法论优势，不只是语言选择**：

| 层次 | 手段 | 拦下什么 |
|---|---|---|
| 类型层 | newtype / enum / `BrandedId` | `SessionId` 误当 `ToolCallId`；非法 `SandboxMode` |
| 状态机层 | typestate（`Turn<Ide>` → `Turn<Streaming>`） | 未开的 turn 被引用；重复 commit |
| crate 层 | 依赖方向守卫 | 向上依赖、循环依赖 |
| 契约层 | conformance 套件（跑起来） | 后端语义漂移 |

**铁律清单**（每条都要有机器校验，无校验的不算铁律）：

- **T1** 依赖只向下、无环 —— `check_architecture`
- **T2** 协议确定性 —— `check_protocol`（golden 回放）
- **T3** 会话可重建 + append-only —— `check_session_schema`
- **T4** 沙箱 × 审批矩阵自洽（5 档执行模式 × 3 档沙箱）—— `check_mode_matrix`
- **T5** 配置四层级联优先级正确 —— `check_config_layers`
- **T6** 宿主语义等价（新增）—— 同事件流喂给 ≥2 宿主后端，断言可消费且无业务分叉
- **T7** 文件变更**只能**走 `apply_patch` 信封（Codex 的铁律，防止绕过审计）

---

## 六、诚实边界（必须写清，Proteus 的硬要求）

Proteus 明令声明「不适合的场景」与「规划未落地」的部分。NEO 同样：

- **不追求**成为通用 agent 框架。NEO 是**面向 Proteus 的编程 Agent**，差异化在 Proteus（见第七节）。
- **OS 沙箱的三平台差异是真实成本**：macOS Seatbelt 与 Linux Landlock+bwrap 能力不同构；Windows restricted token+ACL 需要单独验证。**不得声称三平台等价**，只能声称「同一 `SandboxMode` 语义下各自尽最大努力，且越权必被拒」。
- **MCP 是 SPI 后端，不是内核职责**。MCP 不可用不影响内核可用。
- **`dsh-host-desktop` 的 webview 依赖 OS**：WKWebView/WebView2/WebKitGTK 行为有差异（尤其 `backdrop-filter`）。**不得声称三平台视觉一致**。
- 早期 Rust 原型**未跑过 `cargo test`**，只有 `cargo check` 与 Python 验证套件（`verify.sh`）。

---

## 七、和 DSH / 纯 Codex 的区别（三方对比）

| 维度 | DSH | Codex CLI | **NEO** |
|---|---|---|---|
| 内核 | 插件化（可换 loop） | 固定（Rust） | **固定语义核心 + 5 个受控 SPI** |
| 扩展 | 任意层插拔 | 配置 + MCP | **仅 5 个 SPI**，每个有 conformance |
| 依赖可静态分析 | 否（运行时查找） | 是 | **是** + 守卫强制 |
| 宿主 | Web（+ Electron 壳） | CLI / TUI / Exec / AppServer / MCP | **TUI / Desktop(系统 webview) / Web / Exec**，同为 `HostBackend` |
| 换 UI 动内核吗 | 动（UI 是插件） | 不动 | **不动**（HostBackend SPI） |
| 与 Proteus 的关系 | 无 | 无 | **内核即 Proteus 语义层的消费者；差异化的唯一来源** |

**NEO 的定位一句话**：用 Proteus 的方法论，做 Codex 已验证的内核架构，服务 Proteus 跨端框架。
——**没有第三句**。删掉 Proteus 就不是 NEO，是又一个通用 agent（这一点是本计划早期版本最大的缺陷，已修正）。
