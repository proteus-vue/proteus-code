# DSH 现有能力 → NEO 模块映射

> DSH v0.1.0-rc.6 的能力如何落到 NEO 六层里。**不是一一照搬**，冗余项直接裁掉。

| DSH 能力（插件形态） | NEO 落点 | 层 | 处置 |
|---|---|---|---|
| 模型适配器（DeepSeek/OpenAI/Claude/…） | `ModelProvider` trait 实现 | L2（扩展点） | ✅ 保留 |
| 工具注册表 | `ToolRegistry`（BTreeMap，顺序稳定） | L2 | ✅ 保留，**改为固定顺序以保证缓存命中** |
| 文件编辑 / shell / 搜索 | `bash` + `apply_patch` | L3 | 🔶 **收敛为 Shell-First 最小集** |
| 其余 90+ 官方工具 | Skill / MCP 显式注册 | L3 | ❌ **不再默认加载** |
| 会话日志（append-only） | `dsh-session` JSONL | 旁挂 | ✅ 保留并强化为真相源 |
| 沙箱（bwrap/Landlock、seatbelt、token） | `dsh-sandbox` | L1 | ✅ 保留，Rust 重写 |
| fail-closed 拒绝执行 | `SandboxError::Unavailable` | L1 | ✅ 保留 |
| 存储层 | 固定实现，格式版本化 | 旁挂 | ❌ **不再可替换**（ADR-0001） |
| **Agent loop** | `run_turn`（固定） | L2 | ❌ **不再插件化**（ADR-0001） |
| **UI** | L5 三宿主 | L5 | ❌ **不再插件化**（ADR-0001） |
| 调度器 / 子 Agent | `dsh-orchestration` Goal 引擎 | L4 | 🔶 收敛为 Goal + 四阶段 |
| 4 种运行模式（Standard/PTC/Minimal/Creator） | — | — | ❌ **取消**，改为沙箱×审批×file_edit 正交组合 |
| PTC（生成 TS 脚本编排） | — | — | ❌ **取消独立运行时**；多步编排改由 Subagent / shell 脚本承担 |
| 插件框架 Cordis（vendor 9 包 + 18 patch） | — | — | ❌ **完全移除**，不引入任何插件元框架 |
| 无 API Key 回放测试 | golden 回放 | 05-验证 | ✅ **保留并升级**为核心验证手段 |
| 219 个 workspace 包 | 12 个 crate | — | 🔶 按层收敛 |
| Electron 桌面壳 | — | — | ❌ 移除，改 Web + TUI 宿主 |

## 迁移建议顺序

1. **先冻结**：把 DSH `0.1.0-rc.6` 存为只读基线快照，作为行为对照
2. **不要逐包迁移**：NEO 是重写，逐包搬会同时带入冗余设计
3. **按 NEO 的层从底往上建**：L0 协议 → L1 平台 → L2 内核 → …
4. **行为对照**：用同一批任务在 DSH 基线与 NEO 上跑，对比会话日志差异
5. **能力按需搬**：只有当某个 DSH 插件被真实使用时，才在 NEO 里以 Tool/Skill 形式重建
