# L5 · 宿主层规格

**crates**：`dsh-host-tui`、`dsh-host-web`、`dsh-exec`、`dsh-cli`
**铁律**：**宿主不含任何业务逻辑，只做 渲染 + 输入解析 + 事件消费**

## 共同约束
1. 只通过 `submit(Op)` / `next_event()` 与内核通信
2. 不得持有 Session 状态副本（唯一真相源在 L2 + JSONL）
3. 三个宿主之间**不得互相依赖**（架构守卫校验）

## dsh-host-tui（ratatui + crossterm）

### 快捷键（对齐 Codex）
| 键 | 功能 |
|---|---|
| `Ctrl+L` | 清屏（保留历史） |
| `Ctrl+O` | 复制最新输出到剪贴板 |
| `Ctrl+R` | 搜索提示词历史 |
| `Ctrl+G` | 用 `$EDITOR` 打开外部编辑器 |
| `Tab` | 运行中排队下一轮输入（不打断） |
| `Esc Esc` | 编辑上一条消息 |
| `Shift+Tab` | 循环切换五档执行模式（ZCode） |
| `@` | 模糊文件搜索 |

### Slash 命令
`/goal` `/review` `/model` `/fork` `/permissions` `/compact` `/diff` `/status` `/agent <name>` `/debug-config`

## dsh-host-web（axum + WebSocket/SSE）

- 渲染层用任意前端框架（React/Vue/Svelte 均可，通过 WS 消费事件流）
- **不需要 Electron**：内核是 Rust 单二进制，Web 宿主只是本地服务 + 浏览器
- 五档执行模式做成显式的模式切换器（ZCode UX）
- `@ # / $` 输入解析器（ZCode 交互核心）
- Diff 审批面板：hunk 级接受/拒绝

## dsh-exec（无头 / CI）
```
neo exec "fix failing tests" --json --sandbox workspace-write
```
零交互、可脚本化、可管道化。

## dsh-cli（multitool 入口）
```
neo              → TUI
neo exec "..."   → Exec
neo serve        → Web 宿主
neo mcp-server   → 暴露为 MCP server
neo resume       → 从 Checkpoint 恢复
```

## 验收
- [ ] 三宿主消费同一 EventMsg 流，业务逻辑零重复
- [ ] 宿主间无相互依赖（架构守卫）
- [ ] Web 宿主不依赖 Electron
- [ ] TUI 快捷键与 slash 命令全部可用
