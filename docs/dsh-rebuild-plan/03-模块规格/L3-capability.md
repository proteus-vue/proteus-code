# L3 · 能力层规格

**crate**：`dsh-capability`
**依赖**：L2、L1、L0
**职责**：3 类扩展点的实现与注册（Tool / Skill / Subagent）+ MCP client

## 1. Tool（Shell-First，ADR-0005）

### 默认最小工具集（仅此三项默认注册）
| 工具 | 用途 |
|---|---|
| `bash` | Shell-First 核心：cat 读、grep/find 搜、跑测试、跑 linter |
| `apply_patch` | **所有文件变更的唯一通道** |
| `request_user_input` | agent 主动向用户提问 |

### 硬规则
> **文件修改 100% 走 `apply_patch`。禁止 shell 重定向 / heredoc 写文件。**
> 由 system prompt 约束 + ToolRouter 层审计（M3 验收：零例外）。

### 扩展工具
通过显式声明注册（配置文件或 MCP），**不默认加载**。
这与 DSH "100+ 插件默认全量" 形成对照 —— 目的是**压缩工具描述、提高选择准确率、提高缓存命中**。

## 2. Skill
可复用工作流，Markdown + 可选脚本定义。
对齐 DSH 的 skill 概念，但不走插件框架，走**文件系统约定 + 显式注册**：
```
.neo/skills/<name>/SKILL.md
```

## 3. Subagent（对齐 ZCode）
**Markdown 定义**，与 ZCode 的 `~/.zcode/agents/*.md` 同构：
```markdown
---
name: test-runner
model: deepseek-v4-flash
tools: [bash, apply_patch]
---
你负责跑测试。给定文件或目录，找出相关测试并执行，
返回失败摘要与行号。
```
- 全局：`~/.neo/agents/`
- 项目级：`.neo/agents/`
- 调用：`/agent test-runner`

**隔离**：每个 subagent 独享 git worktree（对齐 Codex 2026 的做法），避免并发写冲突。
冲突时生成 diff review session 交人工裁决。

## 4. MCP client
- 连接外部 MCP server
- 健康检查和重连
- MCP 工具**不默认注册**，需显式声明

## 验收
- [ ] 默认工具集 ≤ 3 个
- [ ] 100 次任务审计：文件变更 100% 经 apply_patch
- [ ] Subagent 可从 Markdown 正确加载，工具白名单生效
- [ ] 未声明的 MCP 工具不出现在工具描述中
