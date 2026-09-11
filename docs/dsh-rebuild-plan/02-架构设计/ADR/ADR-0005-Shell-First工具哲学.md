# ADR-0005：Shell-First 工具哲学，文件变更只走 apply_patch 信封

- **状态**：已接受
- **关联**：Codex Shell-First；DSH 冗余项 R3（100+ 插件默认全量）

## 背景
DSH 默认加载 100+ 官方插件，工具描述占用大量上下文，且干扰模型工具选择。
Codex 的做法相反：system prompt 教模型一套以 shell 为核心的工具包（cat 读、grep/find 搜、测试运行器验证），
**文件变更则保留给专用的 `apply_patch` 信封**。

## 决策
1. **默认只注册最小工具集**：`bash`（Shell-First 核心）+ `apply_patch` + 少量必需内置。
2. 其余能力通过 **Skill / MCP / Subagent** 显式声明后才加载。
3. **所有文件修改必须走 `apply_patch`**，禁止用 shell 重定向/ heredoc 写文件。

## 理由
1. 工具越多，上下文占用越大、模型选择越易错 —— 与"固定前缀、字节稳定"的缓存策略一致。
2. `apply_patch` 提供**可 diff、可审查、可回滚**的变更路径，这是安全审计的基础。
3. Shell 能覆盖绝大多数"读/搜/跑"需求，无需专用工具。

## 代价
- 模型需要被明确教导"读用 cat、搜用 grep、写用 apply_patch"（system prompt 职责）
- 某些高频操作（如批量重命名）不如专用工具顺手 → 用 Skill 封装

## 验证方式
M3 验收：审计 100 次任务的工具调用日志，文件变更 100% 经 `apply_patch`，**零例外**。
