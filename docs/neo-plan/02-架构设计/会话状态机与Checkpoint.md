# 会话状态机与 Checkpoint（断点恢复）

> 吸收 ZCode Goal Mode 的"可暂停 / 可重定向 / 可断点恢复"，叠加 Codex 的 Session/Turn 模型。

## 状态机

```
        ┌──────────┐
        │   Idle   │◄──────────────┐
        └────┬─────┘               │
             │ Op::UserTurn        │ TurnComplete
             ▼                     │
        ┌──────────┐          ┌────┴─────┐
        │ Planning │─────────►│ Executing│
        └──────────┘          └────┬─────┘
                                   │ 需要审批
                                   ▼
                            ┌─────────────┐
                            │AwaitingAppro│
                            └──────┬──────┘
                                   │ Op::Approve
                                   ▼
                              (回到 Executing)
```

任一状态收到 `Op::Interrupt` → 在**安全点**中断 → 回到 `Idle`。
**安全点**：工具调用边界。绝不在写文件中途中断（避免半写状态）。

## run_turn 主循环（L2 core）

```
1. 预采样：估算本轮 token 占用
2. 若超限 → ContextManager::compact()（压缩而非截断）
3. 组装请求：固定前缀（system + 工具描述，字节稳定）+ 增量历史
4. 流式请求模型，逐块发 AgentMessageDelta
5. 解析工具调用 → ToolRouter
6. 每个工具调用过 ApprovalGate（沙箱 × 审批 双轴判定）
7. 沙箱内执行，产出 ToolCallEnd
8. 回到 3，直到模型不再请求工具 或 触达停止条件
9. 保存 Checkpoint，发 TurnComplete
```

**关键**：步骤 3 的**固定前缀必须字节稳定**。
工具描述按固定顺序注册、system prompt 不掺时间戳/随机数 —— 这直接决定 prompt cache 命中率。

## Checkpoint 设计（ZCode 断点恢复）

```
checkpoint:
  id:            cp_01HX...
  goal_id:       goal_...          # 若属某个 Goal
  turn_index:    17
  parent:        cp_01HW...        # 链式，支持回溯
  context_digest: sha256(...)      # 压缩后上下文摘要，用于校验
  pending_subtasks: [...]          # 未完成的子任务队列
  artifacts:     [...]             # 已产出物（文件/diff/测试结果）
  created_at:    RFC3339
```

**恢复流程**：进程重启 → 读最新 checkpoint → 重建 pending_subtasks → 继续执行。
**不重建模型上下文的历史细节**（太贵），而是：
- 载入 `artifacts` 摘要
- 载入 `pending_subtasks`
- 以"续接提示"方式让模型知道已完成什么

## 停止条件（防止长程任务失控）

对齐 ZCode "goal is met or a stopping condition triggers"：

| 条件 | 默认阈值 | 可配 |
|---|---|---|
| 最大迭代轮数 | 50 | ✅ |
| 最大 wall-clock | 4h | ✅ |
| 最大 token 预算 | 500K | ✅ |
| 连续失败次数 | 3 | ✅ |
| Review 未通过重试上限 | 3 | ✅ |
