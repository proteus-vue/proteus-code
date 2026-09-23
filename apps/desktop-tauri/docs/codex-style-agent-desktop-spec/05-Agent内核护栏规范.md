# 05 Agent 内核护栏规范

> **本文件是硬约束，不是建议。**
>
> 每一条都对应一次在本机实测到的真实事故或真实差异。跳过任何一条，都会复现别人已经踩过的坑。
> 对照组：MiMo（缺护栏，出现无限循环与工具调用风暴）vs ZCode（护栏完整，同一模型表现正常）。

---

## 背景：一次真实事故

同一模型 `mimo-v2.6-flash`、同一后端地址，在两个客户端里表现天差地别：ZCode 正常收敛，MiMo 无限循环刷工具调用（数据库里记录到 2247 次调用 / 2133 次错误）。

排查结论：**模型没问题，是 harness 把刹车拆了。** MiMo 里四道闸门同时失效：

1. `doom_loop` 权限闸门被"完全访问权限"静默批准（代码里 `FullAccess → {mode:"auto", reply:"always"}`）
2. `continue_loop_on_deny` 被桌面端硬编码为 `true`（拒绝也不终止）
3. `agent.steps` 默认 `Infinity`（没有步数天花板）
4. 无任何"你在重复调用"的上下文提示

而 ZCode 侧：`detectRepeatedToolCallWarnings`（第 3 次相同调用即提醒）、`detectToolCallBudgetWarning`（单轮预算提醒）、子代理 `maxTurns ?? 4`、`reactLoop.maxRounds 30`、`maxConsecutiveErrors 3`、token/时间预算刹车。

**下面十条，就是把这套护栏固化成规范。**

---

## 第 1 条：重复调用提醒（必做）

连续相同调用必须在**上下文里**告诉模型它在转圈，而不是默默让它继续。

- **反例**：MiMo 全树 `repeatedToolCall` / `toolCallWarning` / `anomalyGuard` **0 命中**。
- **正例**：ZCode 在计算调用签名（工具名 + 参数稳定序列化）后，连续第 3 次相同即注入提醒，且 `maxBudgetWarningsPerTurn: 3` 防刷屏。

**实现要点**：

```ts
interface GuardConfig {
  repeatedToolCallThreshold: 3;      // 连续 N 次相同即提醒
  maxBudgetWarningsPerTurn: 3;       // 单轮最多提醒次数，防止刷屏
  signatureOf(call): string;         // 工具名 + 稳定序列化的参数
}
```

提醒内容应明确给出三条脱身路径（用已有结果推进 / 说明阻塞点 / 向用户求助），而不是只说"不要重复"。

**验收**：手动构造一个重复调用场景（让模型对同一文件连续 grep 同一模式），第 3 次时上下文里必须能看到提醒；第 4 次之后不再重复注入。

---

## 第 2 条：单轮调用预算提醒（必做）

单轮工具调用数超阈值时提醒模型，而不是等熔断。

- **反例**：MiMo 只有 16 次/响应的**熔断**（直接砍掉调用），没有中间提醒。熔断只截单轮，不截循环。
- **正例**：ZCode 在单轮调用数达阈值时注入预算提醒。

**实现要点**：提醒阈值（建议 8–10）要**低于**熔断阈值（建议 16），留出一档缓冲，让模型有机会自我纠正。

**验收**：单轮调用达到提醒阈值时，后续请求里能看到预算提示；达到熔断阈值时该轮被截断且给用户可见的提示。

---

## 第 3 条：禁止"失败级联取消"（必做）

一个工具调用失败，**不能**把同批次其他排队中的调用全部取消。

- **反例**：MiMo 的 `ToolGate` —— `bash` 退出码非 0 即 `gate.fail()`，整批排队调用被取消，上下文中塞满 "Tool call cancelled because an earlier tool call in this response failed."。满屏失败输出对小模型是强误导，直接诱发更多错误调用。
- **正例**：ZCode 是 `maxConcurrency: 10` 且无失败级联（`cascade` 在其代码里只出现在 SQL 的 `on delete cascade`）。

**实现要点**：

```ts
// ❌ 不要这样
if (toolName === 'bash' && exitCode !== 0) cancelAllPendingCalls();

// ✅ 这样
markCallFailed(callId);          // 只标记这一个失败
continueOtherPendingCalls();    // 其他继续跑
```

**验收**：并发发起 5 个调用，其中 1 个故意失败；其余 4 个必须正常完成并返回结果。

---

## 第 4 条：被取消的 tool_call 必须补占位消息（必做，易漏）

在 **chat completions** 方言下，如果模型返回了 `tool_calls` 但你没有为其中某个 call 提供对应的 `tool` 消息，下一轮请求会直接被 API 拒绝（或产生不可预期的上下文）。

- **反例**：MiMo 会话里满屏 `cancelled` 占位，就是这个机制在补洞——但补的是**空内容**，反而污染上下文。

**实现要点**：

```ts
for (const call of assistantMessage.tool_calls) {
  messages.push({
    role: 'tool',
    tool_call_id: call.id,
    content: executed[call.id]?.output
              ?? '[cancelled] 用户中断或前序调用失败',  // 必须有内容，且要写清原因
  });
}
```

占位内容必须**说明原因**（"用户中断" / "超出预算" / "被熔断"），不能是空字符串。空占位会让模型以为工具返回了空结果，从而做出错误判断。

**验收**：中断一次进行中的多工具调用轮次，检查下一轮请求体中每条 `tool_calls` 都有对应的 `role:"tool"` 消息且 `content` 非空。

---

## 第 5 条：步数硬上限绝不能是 Infinity（必做）

- **反例**：MiMo `const maxSteps = agent.steps ?? Infinity`。
- **正例**：ZCode 子代理 `maxTurns ?? 4`、工作流 `reactLoop.maxRounds 30`。

**实现要点**：

```ts
const maxSteps = agent.steps ?? 50;   // 默认值必须是有限数，永远不要 Infinity
```

并且这个默认值要**可被配置文件覆盖**，但覆盖后仍应有绝对上限（如 200），防止用户自己设成 Infinity 又把刹车拆了。

**验收**：不配置 `steps` 时跑一个会死循环的任务，必须在 50 步内停止并给出说明。

---

## 第 6 条：权限闸门不能被"完全访问"静默批准（必做）

- **反例**：MiMo `function Ek(mode) { return mode === FullAccess ? {mode:"auto", reply:"always"} : … }` —— 选了"完全访问权限"后，doom loop 闸门被自动批准，等于没有闸门。

**实现要点**：

- 权限模式分档（建议 `plan` / `edit` / `full` / `yolo`），由用户**显式选择**且可随时切换。
- 即使是最宽松的档位，**危险操作**（删除文件、force push、执行无 diff 预览的 shell）仍应单独确认。
- 循环检测类闸门**永远不参与自动批准**，必须走独立通道。

**验收**：在最高权限档位下触发重复操作，闸门仍应弹出或记录，不能被静默放行。

---

## 第 7 条：`continue_loop_on_deny` 绝不能硬编码 true（必做）

- **反例**：MiMo 桌面端注入 `experimental.continue_loop_on_deny = true`，导致**用户点"拒绝"之后循环依然继续**——拒绝按钮形同虚设，用户只能点"停止"。

**实现要点**：

```ts
ctx.shouldBreak = cfg.experimental?.continue_loop_on_deny !== true;
// 默认值必须是 false：被拒绝就停
```

这个开关可以作为高级选项暴露给用户，但默认必须关闭。

**验收**：连续拒绝同一操作 3 次，Agent 必须停止该操作路径，而不是继续尝试。

---

## 第 8 条：并发白名单 + 并发上限（必做）

- **实测**：MiMo 的 `ToolGate` 有只读并行白名单 `PARALLEL_READONLY_TOOLS = {read, grep, glob}` `[M]`；ZCode `toolConcurrency.maxConcurrency: 10` `[Z]`。

**实现要点**：

```ts
const PARALLEL_READONLY_TOOLS = new Set(['read', 'grep', 'glob', 'list']);
const MAX_CONCURRENCY = 10;

function canParallel(calls) {
  return calls.every(c => PARALLEL_READONLY_TOOLS.has(c.name)) && calls.length <= MAX_CONCURRENCY;
}
```

- 只读操作可并行；**写操作必须串行**（否则会互相覆盖）。
- 并发上限建议 10，超过则分批。

**验收**：模型一次返回 12 个 read 调用时，应分成 10 + 2 两批；一次返回 read + write 混合时，write 必须串行执行。

---

## 第 9 条：预算刹车（必做）

- **正例**：ZCode 的 `session_target` 带 `token_budget / tokens_used / time_used_seconds`，超限进入 `budget_limited` 状态 `[Z]`。

**实现要点**：给每个会话设置**可选**的 token 预算与时长预算，接近阈值时提示，超限时优雅停止（保留已完成的工作与摘要），而不是硬杀进程。

**验收**：设置 10k token 预算后跑一个大任务，应在接近时提示、超限时停下并给出已完成部分的小结。

---

## 第 10 条：上下文压缩与会话裁剪（必做）

长会话必然撑爆上下文。必须提供：

1. **自动压缩**：达到模型上下文的 N%（建议 70%）时，把早期对话压缩成摘要，保留最近若干轮与所有文件路径引用。
2. **手动压缩**：用户可主动触发。
3. **污染隔离**：单次事故产生的几百条错误不应永久留在上下文里——压缩时优先丢弃失败的工具调用详情。

**验收**：长会话跑到底不报错；压缩后模型仍能引用之前出现过的文件路径。

---

## 附：护栏自检清单

实现完成后，用这一张表逐条打勾。**任何一条为否，都不算完成。**

| # | 检查项 | 通过 |
| --- | --- | --- |
| 1 | 连续 3 次相同调用会在上下文中给出提醒 | ☐ |
| 2 | 单轮调用达阈值有预算提醒，且阈值低于熔断阈值 | ☐ |
| 3 | 单个调用失败不会取消同批其他调用 | ☐ |
| 4 | 被取消/未执行的 tool_call 都有非空占位的 tool 消息 | ☐ |
| 5 | `maxSteps` 默认值是有限数，不是 Infinity | ☐ |
| 6 | 最高权限档位下循环闸门仍不被静默批准 | ☐ |
| 7 | 用户拒绝后循环真正停止（`continue_loop_on_deny` 默认 false） | ☐ |
| 8 | 只读并行、写操作串行，并发有上限 | ☐ |
| 9 | 会话有 token/时长预算，超限优雅停止 | ☐ |
| 10 | 上下文可自动/手动压缩，压缩时优先丢弃失败详情 | ☐ |
