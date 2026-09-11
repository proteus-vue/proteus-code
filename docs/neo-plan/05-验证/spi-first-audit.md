# SPI-First 耦合点审计与试点报告

> 按 `spi-first` skill 五阶段执行。审计对象：**proteus-code 现有实现**（Electron + DSH bundle）。
> 审计日期：2026-09-11 · 审计范围：`packages/` + `apps/`（排除 tests/dist/node_modules）

---

## Phase 0 — 目标与范围

| 项 | 值 |
|---|---|
| 目标 | 审计 + 试点改造（NEO Rust 重写前，先量化「换掉什么要改多少」） |
| 范围 | 全部业务源码；**不动** 已完成的 Rust 原型分层与 ADR |
| 改造边界红线 | 现有 Electron 实现**保持可用**（它是当前唯一可交付物），不在此次改造中拆除 |

---

## Phase 1 — 耦合点审计（量化）

计数对象 = 业务源码文件（排除 `tests/`、`dist/`、`node_modules/`）。

| # | 耦合点 | 具体实现名 | 业务文件数 | 优先级 | 未来动因 | 备注 |
|---|---|---|---|---|---|---|
| 1 | **DSH 内核** | `@deepseek-ai/*` | **8** | **P0** | 用户已决定用 Rust 重写内核 | 文件数 8 < 20，但「确定要替换」为真 → 按动因升为 P0 |
| 2 | **Electron 壳** | `electron` | **2** | **P1** | 用户明确要求摒弃重壳 | 文件数 2 < 5，但含 **25 处 API 调用**（main 21 / probe 4）→ 按动因升为 P1 |
| 3 | Proteus CLI | `proteus` 可执行 | 1（`proteus-cli.ts`） | **P2** | 稳定，不换 | 已是单点适配器，登记观察 |

### 关键发现：DSH 耦合只有 8 个文件，因为**它本身已经是 SPI**

明细（8 个文件）：

```
src/tools.ts        src/commands.ts     src/skill.ts        src/index.ts
src/policy-tool.ts  src/proteus-cli.ts  src/dsh-host.d.ts   (apps) desktop/src/profile.ts
```

`ctx.tools` / `ctx.shell` / `ctx.llm` / `ctx.commands` / `ctx.skills` **都是 DSH 自己定义的 SPI**。我们是通过这些接口接入的，所以耦合面很小。

> **这个发现修正了对 DSH 的判断**：DSH 的问题不是「没有 SPI」，而是 **SPI 太多且无名（~90 个）**。
> 我们实际只用到 5 个。所以 NEO 的正确定位不是「从零造 SPI」，而是
> **把 90 个无名 seam 收敛为 5 个有名的**——这与 `Proteus方法论` 文档的结论一致。

### Electron 耦合明细

| 文件 | API 调用数 |
|---|---|
| `apps/desktop/src/main.ts` | 21（`BrowserWindow`×10、`shell.`×4、`app.whenReady`、`dialog.`…） |
| `apps/desktop/src/probe.ts` | 4 |

> **好消息**：Electron 只渗进 2 个文件，说明壳层**本来就已经隔离好了**。
> `HostBackend` 的工作是把这条**已存在的好边界形式化**，而不是新建边界——改造风险因此很低。

---

## Phase 2 — 试点选择

**选定试点：5 个 Rust SPI 的语义接口 + Mock 后端 + conformance。**

选择理由（符合 skill 的「小而完整」标准）：

- 影响面真实（NEO 的全部扩展性都建立在这 5 个 SPI 上）；
- 文件数不爆炸（4 个 crate 内）；
- 做完**立刻能被 conformance 证明**（这是 skill 要求的「可证明」）。

明确边界：
- **新建**：`crates/dsh-mock`（Mock 后端 + conformance）、`crates/dsh-host-desktop`
- **修改**：`dsh-core`（补 3 个语义接口）、`Cargo.toml`、`verify.sh`、`check_architecture.py`、`check_spi_conformance.py`
- **不动**：现有 Electron 实现、DSH bundle、ADR 既有结论

---

## Phase 3 — 五步改造

### Step 1｜语义接口（禁厂商/技术名词）

新增 3 个接口到 `dsh-core`（其余 2 个已存在）：

| 接口 | 命名纪律自检 |
|---|---|
| `SessionPersistence` | `append` / `load` —— 领域动词，未用 "JSONL"/"SQLite"/"S3" |
| `SandboxBackend` | `supports(mode)` / `execute(mode, cmd)`，参数用 `SandboxMode` 领域枚举，未用 OS 专有结构体 |
| `HostBackend` | `run` / `capabilities` —— 未用 "Electron"/"TUI"/"wry" |

**错误用统一领域类型**：`PersistenceError { Tampered, Unavailable }`、`SandboxOutcome { Ran, Denied }`——上层不 catch 底层 IO 异常。

### Step 2｜≥2 后端（含 Mock，强制）

新建 `crates/dsh-mock`。**Mock 不是可选项**（AP-01）：

| SPI | 实现数 | 分布 |
|---|---|---|
| `ModelProvider` | **2** | `MockModelProvider`、`ScriptedModelProvider`（行为不同） |
| `SandboxBackend` | **5** | `NoopSandbox`、`LeakySandbox`(坏) + dsh-sandbox 内 3 个 |
| `SessionPersistence` | **2** | `InMemoryPersistence`、`TamperingPersistence`(坏) |
| `HostBackend` | **3** | `DesktopHost` + `MockHost`、`BrittleHost`(坏) |
| `Tool` | **5** | dsh-capability 3 个 + `MockTool`、`NamelessTool`(坏) |

**每个 SPI 都配了一个"坏后端"** —— 这是 conformance 负向用例的被试。

### Step 3｜conformance 契约测试

`crates/dsh-mock/tests/conformance.rs`，**10 个测试，同一份用例跑所有后端**：

| 契约 | 正向用例 | 负向用例（证明套件有牙齿） |
|---|---|---|
| HostBackend | 2 后端各跑一通 + 双后端同流等价 | `BrittleHost` 必须被抓住 |
| SessionPersistence | append-only 保真 | `TamperingPersistence` 必须被抓住 |
| SandboxBackend | 能力声明与执行一致 | `LeakySandbox` 必须被抓住 |
| ModelProvider | 同输入同输出（T2 前提） | — |
| Tool | 名/描述非空 + 畸形参数不 panic | `NamelessTool` 必须被抓住 |

**只测语义契约，不测实现细节**：顺序、保真、能力边界、确定性。

### Step 4｜接线与守界

- `verify.sh` 从 `cargo check` 升级为 **`cargo test --workspace`** —— 否则"有 conformance"只是文件存在（AP-03）。
- `check_spi_conformance.py` 接入 `verify.sh`，成为 CI 门禁。

### Step 5｜诚实边界（三条，不可省）

#### 边界 1：性能开销 —— **measured: false**
抽象间接层（trait 动态派发）的开销**未实测**。禁止宣称数字。
> 待办：M1 阶段补 `criterion` 基准，测 `dyn Tool` vs 单态调用差异。

#### 边界 2：能力不齐时的降级
已落地机制：`SandboxBackend::supports()` + `HostCapabilities`。
**禁止静默失败**——`NoopSandbox` 对不支持的档位显式返回 `Denied`，而非放行。
> 未完成：`Capabilities` 降级表尚未覆盖全部后端组合。

#### 边界 3：何时**不值得**用这套 SPI
- **一次性脚本 / MVP**：不要引入（AP-06）。NEO 是产品，故适用。
- **性能热点路径**：trait 动态派发若实测超标，应改静态分派而非加分支。
- **后端涨到 6+ 且差异大**：应**拆 SPI**，不是加 `if backend == X`（AP-07）。当前最大 5（`Tool`），已达上限。
> **预警**：`Tool` 有 5 个实现，接近 AP-07 阈值。若继续增长，应把 `Tool` 的**传输**（in-process/MCP）与**契约**彻底分离，而不是让单 trait 承载两组语义。

---

## Phase 4 — 验证与反模式自检

### 执行结果（如实报告）

```
verify.sh                                   ✅ 全部通过
  check_architecture.py                     ✅
  check_protocol.py                         ✅
  check_session_schema.py                   ✅
  check_config_layers.py                    ✅
  check_mode_matrix.py                      ✅
  check_spi_conformance.py                  ✅（修正后）
  cargo test --workspace                    ✅ 10 passed / 0 failed
```

### 反模式自检

| 反模式 | 命中 | 说明 |
|---|---|---|
| AP-01 单后端 SPI | ❌ **已消除** | 5 个 SPI 全部 ≥2 实现（修正前 `HostBackend` 只有 1 个且门禁谎报 4 个） |
| AP-02 接口含技术名词 | ❌ | 三个新接口命名自检通过 |
| AP-03 无 conformance | ❌ **已消除** | 首次运行本检查时，5 个 SPI 都无 conformance |
| AP-04 业务绕过接口 | ⚠️ | **现存**：Electron 直接出现在 `main.ts`（21 处）。这是下一试点（见 Phase 5 遗留） |
| AP-05 只定义类型不定义行为 | ❌ | conformance 已用例化行为（顺序/保真/能力边界/确定性） |
| AP-06 过度设计 | ❌ | 三动因均真（要替换 / 多实现 / 平台适配），SPI 数量守在上限 8 内 |
| AP-07 后端爆炸 | ⚠️ | `Tool` 达 5 个实现，接近阈值；已在边界 3 预警 |
| AP-08 成本不透明 | ⚠️ | 性能 `measured: false`，已显式标注（未伪造） |

> **⚠️ 三条预警如实保留**，未粉饰。

---

## Phase 5 — 遗留与下一步

### 本次未做（明确边界）

- 未拆除 Electron 实现（它是当前唯一可交付物，拆了就没有可用产品）。
- 未实测抽象性能开销。
- 未实现 `Capabilities` 全组合降级表。
- 未给 `Tool` 做「契约 vs 传输」的彻底分离（只是预警）。

### 剩余候选（下一步试点）

| 优先级 | 候选 | 计划 |
|---|---|---|
| **P1** | **Electron → `HostBackend` 后端** | 把 `main.ts` 里 21 处 Electron API 收进 `dsh-host-desktop` 的 Rust 实现；前端资源改由 webview 自定义协议注入。**这是 AP-04 的现存漏洞** |
| P1 | DSH → Rust 内核 | 按 `04-落地计划/` M0–M6 推进 |
| P2 | Proteus CLI 调用 | 已单点适配（`proteus-cli.ts`），登记观察，暂不动 |

### 本次交付的意外收获（方法论层面）

1. **假门禁比没门禁更糟**：门禁首版「用字符串搜索判断后端数」，在 `HostBackend` 只有 1 个实现时报了 4 个——**给出了假保证**。若不是按 SPI-First 做审计，这个洞会一直存在。已改为只认 `impl X for` 语法事实。
2. **负向用例真的会抓到自己**：`TamperingPersistence` 首版只在输入含 `"user"` 时违规，而契约用例的输入是 `{"a":1}`，导致负向用例自身失败——**恰好证明负向用例在起作用**。已改为无条件违规。
3. **SPI 版本漂移检查**：新增 `spi_contract_digests.json` 指纹基线，契约口径静默变化会在门禁报错。契约漂移是 SPI 最常见的隐性腐化。
