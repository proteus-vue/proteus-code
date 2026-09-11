# WBS 任务分解

## M0 骨架（约 1 周）
| # | 任务 | 产出 | 依赖 |
|---|---|---|---|
| 0.1 | 建 Rust workspace，10 crate 占位 | `Cargo.toml` ×11 | — |
| 0.2 | 定义 `neo-protocol` 全部类型 | `L1` | 0.1 |
| 0.3 | 写架构守卫脚本 | `check_architecture.py` | 0.1 |
| 0.4 | 架构守卫接入 CI | CI 配置 | 0.3 |
| 0.5 | **录基线**：当前 AI 出码一次通过率 | 基线报告 | — |

## M1 协议与会话（约 2 周）
| # | 任务 | 依赖 |
|---|---|---|
| 1.1 | SQ/EQ 队列实现（tokio） | 0.2 |
| 1.2 | submission_loop 与状态线性化 | 1.1 |
| 1.3 | JSONL writer（append-only，seq 递增） | 0.2 |
| 1.4 | JSONL reader（状态重建） | 1.3 |
| 1.5 | golden 用例 ×3 | 1.2, 1.4 |
| 1.6 | `check_protocol.py` / `check_session_schema.py` | 1.5 |

## M2 安全（约 2 周）
| # | 任务 | 依赖 |
|---|---|---|
| 2.1 | SandboxBackend trait + 三平台实现 | 0.1 |
| 2.2 | fail-closed 逻辑 | 2.1 |
| 2.3 | ApprovalGate 双轴判定 | 2.1 |
| 2.4 | 五档执行模式映射表 | 2.3 |
| 2.5 | `check_mode_matrix.py` | 2.4 |
| 2.6 | 三平台实机越权拦截测试 | 2.1 |

## M3 工具（约 2 周）
| # | 任务 | 依赖 |
|---|---|---|
| 3.1 | `bash` 工具（Shell-First） | 2.1 |
| 3.2 | `apply_patch` 工具 + diff 渲染 | 2.1 |
| 3.3 | system prompt：教导 cat/grep/apply_patch 用法 | 3.1, 3.2 |
| 3.4 | Skill 加载器（`.neo/skills/`） | 1.1 |
| 3.5 | Subagent Markdown 加载器 + worktree 隔离 | 3.1 |
| 3.6 | MCP client（显式注册） | 1.1 |
| 3.7 | 100 次任务审计（apply_patch 零例外） | 3.2 |

## M4 Goal 引擎（约 3 周）
| # | 任务 | 依赖 |
|---|---|---|
| 4.1 | 目标解析 → 任务树 | 1.1 |
| 4.2 | 四阶段闭环（Plan/Code/Review/Learn） | 4.1, 3.1 |
| 4.3 | Checkpoint 链 + 断点恢复 | 1.3, 4.2 |
| 4.4 | 停止条件（5 项） | 4.2 |
| 4.5 | ContextManager：固定前缀 + 预采样 compact | 1.1 |
| 4.6 | AGENTS.md 级联（含 32KiB 降级） | 4.5 |
| 4.7 | fresh-by-default（BM25） | 1.4 |
| 4.8 | 四级配置合并 | 4.5 |

## M5 TUI（约 2 周）
| # | 任务 | 依赖 |
|---|---|---|
| 5.1 | ratatui 骨架 + 事件渲染 | 1.1 |
| 5.2 | 快捷键（全量） | 5.1 |
| 5.3 | slash 命令（全量） | 5.1 |
| 5.4 | `@ # / $` 输入解析器 | 5.1 |
| 5.5 | 五档模式切换 UI | 5.1, 2.4 |

## M6 Web（约 2 周）
| # | 任务 | 依赖 |
|---|---|---|
| 6.1 | axum + WS 事件桥 | 1.1 |
| 6.2 | 前端渲染（消费 EventMsg） | 6.1 |
| 6.3 | Diff 审批面板（hunk 级） | 6.2 |
| 6.4 | 五档模式切换器 | 6.2 |
| 6.5 | 与 TUI 一致性验证（同事件流） | 6.1, 5.1 |

**总计约 14 周**（含缓冲，单人节奏；若并行可压缩到 8–9 周）
