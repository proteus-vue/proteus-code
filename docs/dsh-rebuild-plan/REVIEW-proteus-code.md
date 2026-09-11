# 评审：本计划与现有 proteus-code 实现的关系

> **评审对象**：`docs/dsh-rebuild-plan/`（项目代号 NEO，Rust 重写内核）
> **评审日期**：2026-09-11
> **评审立场**：本仓库已有一个**可运行**的 proteus-code 实现（Electron + DSH bundle）。本计划主张的方向与它**互斥**。此文件不是要否定计划，而是让后来者同时看到两条路，避免只读到一半就动手重写。

---

## 一、先说计划做对的部分

不能因为方向冲突就否认它的价值。以下是我的实测核对，**计划的事实基础基本成立**：

| 计划声称 | 实测 | 结论 |
|---|---|---|
| `vendor/` 下 Cordis 9 个包 | 正好 9 个 | ✅ 准确 |
| 100+ 插件默认加载 | base 一层 84 条插件行，加 web-app 超过 100 | ✅ 准确 |
| 219 个 workspace 包 | `packages/` 下 **274** 个 `package.json` | ✅ 保守了 |
| Rust 原型 + 验证套件 | `cargo check` 通过；`05-验证/verify.sh` **exit 0** | ✅ 不是空谈 |

**它对痛点的诊断，我在本项目里亲历过。** 整个开发过程中最难的部分不是写功能，而是：逆向 DSH 编译后的 `lib/client.js`（类名按构建哈希、无法用 CSS 选择器）、反复对抗 `backdrop-filter` 的包含块副作用与 `isolation` 的层叠上下文、满世界找稳定瞄准点。这些成本**真实存在**，计划第 1 条「摒弃一切皆插件」的理由并不空洞。

---

## 二、决定性问题：它提议「替换成什么」的，DSH 已经有了

计划第二章列出「摒弃 → 替换」，但替换项与 DSH **现有能力高度重合**。逐条核对（证据取自 `packages/bundle/base/cordis.patch.yml`）：

| 计划称需「新建」 | DSH 现状 | 证据 |
|---|---|---|
| 多宿主共享内核（Web + TUI + Exec） | **5 个宿主 profile 已发布** | `bundle/` 下 `web-app` / `headless` / `sdk-app` / `sdk-minimal` / `acp-app` |
| Goal Mode（`/goal`） | **已有** | `dsh-goal` + `dsh-goal-round-driver` 两行 |
| 沙箱 × 审批正交双轴 | **已经是两个独立行** | `dsh-sandbox-policy`（mode）+ `dsh-user-approval`（policy） |
| 3 档沙箱 | **已有，且完全对应** | `permission-presets`: read-only / workspace-write / danger-full-access |
| Shell-First 工具哲学 | **已有** | `dsh-tool-bash` + `bash-local` / `bash-sandbox` provider |
| AGENTS.md 级联 | **已有** | `dsh-agent-instructions`（65 536 字节预算） |
| SQ/EQ 事件队列（可录制回放） | **已有等价物** | append-only SessionEvent 日志 + `deriveMessages()` 投影 |
| 四级配置层叠 | **已有** | bundle 层 → profile patch → home patch → `--patch` |
| ModelProvider 扩展点 | **已有** | `ctx.llm` adapter |

**结论**：计划的「替换」清单，实质是**用 Rust 重新实现 DSH 已有的功能**。而它要摒弃的「一切皆插件」，恰恰是这些能力**免费存在的原因**——多宿主能共享内核，正因为内核是插件树上的服务。

这不是说 Rust 重写没有价值，而是说：**计划低估了 DSH 已交付的东西，把"重写"当成了"建设"。**

---

## 三、三个更严重的问题

### 1. 计划丢掉了产品目标

原始需求是「**proteus-code**：面向 **Proteus 跨端框架** 的桌面 AI 应用」。这份计划**通篇没有出现 Proteus**。它是一份通用 agent 的重写方案。

如果采用它，会同时丢掉：已建的 9 个 Proteus CLI 工具、移植自 Codex 的策略引擎、以及**做这个产品的理由**。

### 2. 用一个已验证可用的系统，换数月的从零重写

本仓库**当前已工作**（129 测试通过，真机验证过）：

- Electron 桌面应用，真实渲染、签名路径就绪
- 9 个 Proteus CLI 工具，经 `ctx.shell` 继承沙箱与审批
- Codex 式执行策略引擎（前缀规则 / 规则自测 / 路径钉死 / 失败关闭）
- 液态玻璃界面、ZCode 式工作区选择器（含「不在项目中工作」）

计划 M0–M6 是**数月**工作量，且在这数月里 **Proteus 能力为零**。

### 3. 想要 Codex 式 Rust agent，Codex 本身就在那里

计划自述「吸收 OpenAI Codex CLI 架构」，然后自己写一套。但：

- **Codex 本身就是 Rust**，Apache-2.0，123k stars，且它**已经实现了**计划里的 SQ/EQ、沙箱×审批、Shell-First、apply_patch、多宿主。
- 计划也承认 **ZCode 主体闭源**，「只借鉴可观测的交互范式」——那正是我们已经在做的（选择落点反馈、`/goal`、引用体系都是可观测行为）。

于是问题回到：**NEO 比 Codex 多什么？** 原始需求里唯一的差异化答案是 **Proteus**，而计划把它删了。若真要 Rust 内核，诚实的路径是 **fork Codex 并接上 Proteus**，而不是重写 DSH。

---

## 四、对 Electron 的判断，计划有一处推理不成立

计划摒弃 Electron，理由是「78MB+ 包体、进程/端口/托盘是胶水、与 Node 内核重复」。

但 Electron 换来的是：原生窗口、菜单与 Dock 集成、**签名/公证/自动更新链路**、离线打包，以及**零额外运行时依赖**（DSH 自带一个，Electron 自带 Node 24）。

而计划提出的替代是「Web 宿主（Axum+WS）+ TUI 宿主」。桌面场景下，Web 宿主**仍然需要一个 webview 外壳**才能成为桌面应用——那正是 Electron 的形态。**包体不是被省下，是被转移了。**

---

## 五、建议

**不整体采纳，但不丢弃。** 计划是一份合格的**设计参考**，其诊断是**有效信号**。

| 做什么 | 理由 |
|---|---|
| **保持 proteus-code 作为交付主线** | 它可用、且是 Proteus 专属的 |
| **把计划里代价低、收益实的点抽进现有代码** | 例如：`/goal` 已在 DSH，先确认并接上；沙箱×审批的正交性只需核对配置行；「落点反馈」类交互已在本项目补齐 |
| **若确要 Rust 内核** | 走 **fork Codex + Proteus 接入**，不要重写 DSH |
| **若要重写，先承认它的真实代价** | 需要重新获得 DSH 已交付但不起眼的东西：会话格式迁移链（v0→v1→v2→v3）、compaction、spill、tool-result pruning、token metering、retry、MCP、subagent、skill、webhook、ACP、SDK、i18n。计划的 M1「JSONL 可重建」只是其中很小一片 |

**决策的分水岭**：proteus-code 是「要押数年的产品」还是「这个季度要能用的工具」？

- 是**长期产品** → 拥有内核有道理，但请 fork Codex（已解决 90% 问题），不要从零重写。
- 是**当前要用的工具** → 现有路径胜出，重写是净损失。

---

## 六、这份计划是怎么进仓库的（存疑，待确认）

本目录（61 文件）创建于 2026-09-10 21:02–21:06，**非本会话产出**。它在 21:08 被我的一次 `git add -A` 连带提交，混进了提交信息为「docs: 保存项目记忆」的 commit `6eb2f21` ——**提交信息未提及本目录**。

若本计划确为有价值的正式方案，建议单独提交并说明来源，以保持记录准确。
