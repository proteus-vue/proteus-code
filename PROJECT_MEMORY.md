# PROJECT_MEMORY.md — NEO 项目记忆

> 交接文档。记录**为什么这样做**、**踩过什么坑**、**哪些路走不通**。
> 结构看 `README.md`，设计看 `docs/neo-plan/`。这里只写那两处没有的。

---

## 1. 项目是什么，为什么改了方向

**NEO**：用 **Rust** 重写内核的编程 Agent，四宿主（TUI / Desktop / Web / Exec）共享同一内核。

### 方向变更记录（重要）

本项目**最初**是「proteus-code」：一个 TypeScript 写的 **Electron 桌面应用**，
以 **DSH（DeepSeek Harness）插件**的形式接入（不 fork DSH，走官方扩展点）。
那版**可运行**（129 测试、真机验证过液态玻璃界面、Proteus CLI 工具、Codex 式策略引擎）。

**后来改为**：用 Rust 重写内核，按 Proteus 方法论（统一语义收敛 + SPI-First），
**摒弃 Electron 这类过重的壳**。

因此：**旧实现整体在 `legacy/`**（不在主线），作为设计参考与经验来源。
旧版的项目记忆在 `legacy/PROJECT_MEMORY-proteus-code.md` —— **重写时值得先读**，
它记录了哪些问题会随 Rust 重写消失、哪些不会（见本文第 6 节摘要）。

---

## 2. 方法论：为什么是「5 个有名 SPI」，不是「一切皆插件」也不是「写死内核」

**DSH 的病不是插件太多，而是约 90 个 seam 没有名字、没有门禁。**

| | DSH | NEO |
|---|---|---|
| seam 数量 | ~90（无名） | **5（有名）** |
| 有契约吗 | 有（Cordis service） | 有（Rust trait） |
| 有 ≥2 后端吗 | 部分，未强制 | **强制**，CI 校验 |
| 依赖可静态分析吗 | **否**（运行时服务查找） | **是**（crate 依赖图 + 守卫） |

把 90 个无名 seam 收敛成 5 个有名 seam，**同时得到可替换性与可定位性** ——
这正是 DSH 缺失的那一半。**写死内核反而把可替换性一起丢了**，那是另一种极端。

**五个 SPI**：`ModelProvider` / `SandboxBackend` / `SessionPersistence` / `HostBackend` / `Tool`。
每个都必须有：契约 + ≥2 真实后端（含 Mock）+ conformance 契约测试。
由 `check_spi_conformance.py` 强制（它区分 landed 与 planned，不谎报通过）。

**耦合审计的意外发现**：旧实现对 DSH 的耦合只有 **8 个文件**，因为
`ctx.tools` / `ctx.shell` / `ctx.llm` / `ctx.commands` / `ctx.skills`
**都是 DSH 自己定义的 SPI** —— 我们是通过接口接入的。

---

## 3. 为什么内核必须是 Rust（两个目标，非语言偏好）

### 目标一：高性能
- 主循环零拷贝（`ModelRequest` 借用 slice + `mem::take`，消除每步 O(N²) 深拷贝）
- 系统提示词与工具 schema 构造时缓存
- 无 GC 停顿（延迟敏感的流式输出路径）
- **实测**：计数分配器测同一步在历史 20 条 vs 400 条下的额外分配差 < 16 KB

### 目标二：内存安全
- **14 crate 零 `unsafe`**（`cargo check` 全绿）
- `Send`/`Sync` 由编译器强制 → 类型层面无数据竞争
- 无悬垂/双重释放（所有权 + 生命周期）

### 但 Rust **不保证内存有界** —— 这是最容易被跳过的一点

Rust 消除 UB，但**一条 `yes` 或 `find /` 能把进程撑爆**，完全安全的 Rust 照样 OOM。
所以有界性是**内核义务**，必须自己实现并测试：

| 措施 | 位置 |
|---|---|
| 单次工具输出上限（256 KB，内核级，工具不可放宽） | `ToolCtx::exec` |
| 截断不切开 UTF-8 码点（直接 `&s[..n]` 会 panic） | `truncate_utf8` |
| 沙箱层已声明的 `truncated` 必须上抛 | `ToolCtx::exec` |
| 上下文消息上限（4096）：超限**报错要求压缩** | `Kernel::check_context_budget` |
| 步数预算 / 单步调用上限 | `drive_steps` |

**上下文超限为什么报错而不是丢最老消息**：静默丢弃会让模型视角与会话日志不一致，
破坏「模型可见即已落盘」的铁律 —— 那比报错危险得多。

---

## 4. 内核实现要点（踩过的坑与设计决定）

### 4.1 契约缺陷：`ModelProvider::complete() -> String` 无法表达工具调用

**原设计**（继承自计划原型）是 `fn complete(&self, prompt: &str) -> String`。
按 SPI-First 这是 **AP-05**（接口表达不了所需行为 = 假 SPI）：
工具调用是 agent 的本质，一个只返回字符串的接口根本描述不了它。

**改为** `fn stream(&self, &ModelRequest) -> ModelStream`（流式增量）。
同时把 `ModelRequest.messages` 从 `Vec` 改为 **借用 slice**（见性能一节）。

### 4.2 沙箱必须是结构保证，不是各工具的自觉

**原设计**的 `Tool::execute(&self, args) -> ToolOutput` 让工具自己去执行进程 ——
安全边界就成了"每个工具作者都要记得"的约定。

**改为** `execute(&self, args, ctx: &ToolCtx)`，`ctx.exec()` 是**唯一**执行入口且已绑定沙箱。
工具**在类型层面就没有绕开沙箱的入口**。

### 4.3 闸门按工具名字符串判断是脆弱的

**原设计**用 `if call == "bash"` 判类别。危害很具体：**新增工具就漏判、改名就失效**。

**改为** `Tool::call_kind(args)` 由工具自己声明语义类别。
`bash` 按命令内容分类，且**写标志优先于只读首词**
（`echo hi > f` 首词是 `echo` 但它在写），识别不出时**保守判 Write**
—— 因为把写误判为读会绕过审批，代价不对称。

### 4.4 审批挂起必须保存「整批调用 + 位置」

只存单个待审批调用的话，模型会看到"半个步骤"、工具结果顺序也会错乱。
所以 `PendingApproval { calls: Vec<ToolInvocation>, index }` 保存整批与位置，
批准后从 `index + 1` 继续执行同一步剩余调用。

### 4.5 同名 trait 会误导（已消除）

`dsh-sandbox`（L1）与 `dsh-core`（L2）原本都定义了 `SandboxBackend`，
但职责不同：前者「把命令包裹成受限形式」，后者「在某档语义下能否执行」。
合并会迫使 L1 依赖 L2 的语义类型，破坏依赖方向。**前者改名 `CommandWrapper`。**

### 4.6 确定性是回放的前提

审批 id 由 `(turn, step, index)` **派生**，不用随机/时钟。
测试断言：同一 Op 序列两次运行得到**完全相同**的事件序列。
系统提示词字节稳定（工具用 `BTreeMap` 有序，不含时间/随机）。

---

## 4.7 真机测试抓到的三个 bug（都是"只在 mock 里对"的典型）

沙箱是真机验证的（真的调 `sandbox-exec` 起真进程），一次就抓出三个只有真跑才暴露的 bug：

| # | Bug | 根因 | 修法 |
|---|---|---|---|
| 1 | 6 个测试跑了 **240 秒**才失败 | **管道死锁**：先 `try_wait` 等进程退出、再读管道。管道缓冲区（macOS 约 64 KB）写满后子进程**阻塞在写**、永不退出 | 必须**并发读取** stdout/stderr（两个读线程）。这是 OS 语义的必需品，不是"加深调试链" |
| 2 | `workspace-write` 能用写工作区外的文件 | Profile 里放开了整个 `/tmp`，于是"工作区外但在 /tmp"也被允许 | **不放开 /tmp**（与 Codex 立场一致），只放开显式声明的可写根 |
| 3 | 工作区内的写也被拒 | macOS 上 `/tmp` 是指向 `/private/tmp` 的符号链接，而 Seatbelt 的 `subpath` 匹配**真实路径** | 注入前 `canonicalize()` |

另外把输出上限从"内核事后截断"改为 **SPI 契约的一部分**（`execute(…, limit_bytes)`）：
实现必须**边读边限**。事后截断意味着内存早已被吃掉，再截断毫无意义。
且达到上限后**立即返回**（不继续 drain）——否则 `yes` 这类永不结束的输出源
会让读线程永不返回。

**教训**：安全边界与资源边界都必须**真机验证**。一个只在 mock 里"生效"的沙箱
等于没有沙箱；一个只在 mock 里"有界"的输出路径等于没有上限。

---

## 4.8 TUI 宿主：真终端才暴露的两个问题

TUI 用 pty 驱动验证（**轮询就绪信号**再送按键，不用固定 sleep —— 否则按键会在
切原始模式前被行规程吃掉，测试反而挂到超时）。

| # | 问题 | 根因 | 修法 |
|---|---|---|---|
| 1 | 在 pty 里跑仍报"标准输入不是终端" | **Rust 的 `Command::output()` 会关闭子进程 stdin**，`stty` 因此认为 fd 0 不是终端而失败；而父进程的 `is_terminal()` 明明是 true | 所有 `stty` 调用必须 `stdin(Stdio::inherit())` |
| 2 | 错误信息误导 | 原实现把"stty 调用失败"和"输入不是终端"混成同一句 `None` | `RawMode::enter()` 返回**具体原因**（三种失败补救方式不同） |

**教训**：错误信息必须区分原因。把两种不同的失败合并成一个 `None`，
会让排查方向完全错（我一开始真去怀疑 pty 测试方式，而不是自己的代码）。

另外 TUI 的 `interactive_prompt: true` 能力声明原本是**假的** ——
它能显示"需要审批"却不会问用户，内核会一直挂起。补齐交互后能力声明才成立：
同一任务、同一档位，应答 `y` 则文件写入、应答 `n` 则不写（真终端验证）。

---

## 5. 假通过：门禁与测试各抓到过一次自己

这两次都值得记，因为它们说明"看起来有保护"有多危险：

1. **SPI 门禁首版用字符串搜索判断后端数**，在 `HostBackend` 只有 1 个实现时报了 4 个
   （注释里的 tui/web/exec 被算数）。**假门禁给假保证，比没有门禁更糟。**
   已改为只认 `impl <Trait> for` 语法事实。

2. **上下文上限的测试先写、实现忘加**：测试跑出 FAILED。
   如果没写这条测试，那会是一个"文档声称有保护、实际没有"的洞。

3. **负向用例自己失败过一次**：`TamperingPersistence` 首版只在输入含 `"user"` 时违规，
   而契约用例的输入是 `{"a":1}`，导致负向用例自身失败。
   这恰好**证明负向用例在起作用**（它真的能抓住违规）。已改为无条件违规。

---

## 6. 旧实现（legacy/）里哪些经验仍然有效

重写时**别重复这些坑**，也**别以为重写能消掉它们**：

| 经验 | 重写后 |
|---|---|
| `file:`/symlink 解析陷阱、profile/pnpm/bundle 层叠、Node ≥22 要求、bundle 改动须重启 | ✅ **消失**（编译期静态链接、单二进制） |
| Electron ~78 MB 壳 | ✅ 消失（系统 webview ~5–10 MB） |
| **DOM/CSS 复杂度**（哈希类名不可选、`backdrop-filter` 会为 fixed 后代创建包含块、`isolation` 会创建层叠上下文） | ❌ **仍在**（webview 就是浏览器） |
| **OS 沙箱三平台差异**（Seatbelt / Landlock+bwrap / ACL 不同构） | ❌ **仍在**，且是 NEO 最重的部分 |

---

## 7. 调试与验证

```bash
cargo test --workspace      # 32 测试：内核 16 + 内存 6 + SPI conformance 10
bash scripts/verify.sh      # 全套门禁（Rust 测试 + 零 warning + 6 个 Python 守卫）
```

**验证套件在 `docs/neo-plan/05-验证/`**（Python + golden 用例），
`scripts/verify.sh` 会自动带上它并设 `NEO_ROOT`。

**写测试时的两条纪律**：
1. **每个 SPI 都配一个「坏后端」**作为负向用例被试 —— 抓不住反例的套件没有牙齿。
2. **性能与有界性要实测**，不能写形容词。`crates/dsh-core/tests/memory.rs`
   用计数 `GlobalAlloc` 量化分配；上限类断言必须给出具体字节数。

---

## 8. 当前缺口（诚实清单）

| 缺口 | 影响 |
|---|---|
| **真实模型 provider 未接** | 只有 mock，内核**尚无真机验证**。这是最大风险 |
| **工具未真实执行**（bash 只走沙箱接口、apply_patch 未落盘） | 无法完成真实任务 |
| JSONL 持久化未接线（`dsh-session` 有格式与校验，内核用 in-memory） | 会话不可跨进程恢复 |
| 宿主后端未实现（只有 desktop 空壳） | 无可用界面 |
| L4 Goal 编排未实现（`Compact` / `GoalSet` 等 Op 未实现） | 长程任务不可用 |
| `prefers-reduced-transparency` 等可访问性路径未真机验证 | — |

**下一步优先级建议**：接真实 provider + 真实 bash 执行。
现在有 32 个测试证明机制对，但还没在真实模型上跑过一轮 ——
那比继续扩展协议更能暴露设计问题。
