# 正交双轴：执行模式 × 沙箱

> **沙箱决定"能做什么"，审批决定"何时必须问"。两轴独立。**
> 吸收 Codex 的正交设计 + ZCode 的五档执行模式 UX。

## 轴二：sandbox_mode（技术边界，OS 强制）

| 档位 | 能力 | 典型场景 |
|---|---|---|
| `read-only` | 只读；任何编辑/命令都需批准 | 陌生仓库首次探查、审计 |
| `workspace-write` | 读写工作区内文件、跑常规命令；**出界或联网需批准**；默认断网 | 日常开发（默认） |
| `danger-full-access` | 无文件系统/网络限制 | 仅隔离容器 |

**fail-closed**（继承 DSH 亮点）：请求受限档位但平台无可用沙箱后端 → 直接抛错拒绝，**绝不裸奔**。

## 轴一：approval_policy（流程边界，何时暂停问人）

| 档位 | 行为 |
|---|---|
| `untrusted` | 自动放行已知安全读操作，其余（含破坏性 git 操作）全部询问 |
| `on-request` | 仅当 agent 想"越界升级"时询问 |
| `on-failure` | 沙箱内跑，只在命令失败时问（已废弃，保留兼容） |
| `never` | 从不询问（仍受沙箱约束） |

## ZCode 五档执行模式 → 双轴映射（UI 档位）

| ZCode 模式 | sandbox_mode | approval_policy | **file_edit** | 语义 |
|---|---|---|---|---|
| **计划模式** Plan | `read-only` | `on-request` | `ask` | 先出计划再动手 |
| **变更前确认** Confirm | `workspace-write` | `untrusted` | `ask` | 每次改文件/跑命令前问 |
| **默认** Default | `workspace-write` | `on-request` | `ask` | 平衡（推荐默认） |
| **自动编辑** Auto Edit | `workspace-write` | `on-request` | **`auto`** | 文件改动自动，命令需批 |
| **完全访问** Full Access | `danger-full-access` | `never` | `auto` | 最高自主，风险自担 |

> ⚠️ **第三维 `file_edit` 不是可有可无的。**
> Default 与 AutoEdit 在"沙箱 × 审批"双轴上**完全相同**，若只留两维，这两个模式在底层无法区分。
> 真实差异在**工具类别粒度**：文件编辑是否自动放行、命令是否仍需批准。
> 这一点是 `check_config_layers.py` 的 C3 校验**实际跑出来的**，不是纸上推演。

> **核心设计逻辑**：自主程度与信任度必须正相关。UI 上按"计划 → 变更前确认 → 默认 → 自动编辑 → 完全访问"排序，越往右信任要求越高。

## 自动推荐（吸收 Codex 的贴心设计）

启动时检测当前目录是否被版本控制：
- **是 git 仓库** → 推荐「默认」（workspace-write + on-request）
- **非版本控制** → 推荐「计划模式」（read-only + on-request）

## 常见误区（必须在 UI/文档里讲清楚）

> ❌ 把 `approval_policy = never` 当成"放开权限" → 沙箱仍是 read-only，agent 照样写不了文件
> ❌ 把 `sandbox = danger-full-access` 配上 `untrusted` → 还是会不停被问

**两个旋钮必须分别配。** 这一条由 `05-验证/checks/check_mode_matrix.py` 校验矩阵自洽性。

## 网络默认策略

`workspace-write` 下**默认断网**。
需联网时显式开启 `[sandbox_workspace_write] network_access = true` 或单次批准。
理由：阻止被投毒的依赖包 install 脚本外联。
