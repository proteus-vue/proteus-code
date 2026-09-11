# AGENTS.md — NEO（Rust 内核）工作约定

## 效率优先（最高优先级）

> **强制技能**：本项目所有工作都必须按 `docs/ai-efficiency-rules/`（技能名
> `ai-efficiency-rules`）的规范执行。它是**可机器判定**的硬约束，不是风格建议：
> 六类低效行为（盲等 / 重复拉取 / 重复读 / 无归因重试 / 该并行却串行 / 输出爆炸），
> 三条总则（先判断再动手、等待有条件且获取有缓存、能并行就并行），
> 冲突时按 **正确性 > 安全性 > 效率** 裁决。
>
> 已接入门禁：`scripts/verify.sh` 第 4 段跑 `audit_efficiency.py`，
> **error 级违规会卡门禁**（固定 sleep、重复拉取、无退出条件轮询）。
> 需要等就绪时用 `docs/ai-efficiency-rules/scripts/wait_for.sh`（条件探测 + 总超时），
> 需要拉远程时用 `cache_fetch.sh`（缓存 + 用完清理），不要裸写 `sleep` / `git clone`。

**交付效率是第一标准。** 以下是硬性禁止项，都是实际踩过的低效行为：

- **禁止固定 `sleep` 等待。** 需要等一个进程就绪时，轮询它的就绪信号（日志出现特定行 / 端口可连 / 文件出现），或直接把命令跑完再退出。`sleep 120` 这类等待纯属浪费——上一轮需求因此跑了 40 多分钟。
- **禁止长时间反复测试。** 一次改动跑一轮验证即可。不要为了"再确认一下"重启应用、重复截图、反复读同一份日志。**验证一次，拿到结论就走。**
- **不要用脚本解码图片来"测量"视觉问题。** 需要看 UI 就直接截图看；像素解码只在确实需要数值（对比度）时用一次。用户一张截图一秒能说清的事，不要跑几轮脚本。
- **不要在定位问题时反复 dump 本地 log。** 先读源码确认契约（`node_modules/.pnpm/@deepseek-ai/*/lib/client.js` 是编译后的真实契约，带注释），再从源码推断行为。多数"跑起来看"的疑问读代码就能回答。

**启动 Electron 做一次性验证的正确方式**：把「启动 + 交互 + 采集 + 退出」写成**一个**脚本，用 `PROTEUS_CODE_SMOKE` / `PROTEUS_CODE_QUERY` 让它自己跑完并退出，然后读那一次的输出。不要 `sleep` 分段等待。

## 项目固有约束（改代码前必读）

- **不 fork DSH**，而是按 `docs/neo-plan/` 的方案用 Rust 重写内核。
  设计依据与取舍见 `docs/neo-plan/02-架构设计/`（方法论纲领先读）。
- **不 fork Codex**，但可对照它的架构（Apache-2.0）。若确需 Rust agent，
  诚实路径是 fork Codex 并接 Proteus，而非重写 DSH —— 当前选择是后者，
  理由见 `PROJECT_MEMORY.md`。
- **沙箱是内核的结构保证**：`Tool::execute` 必须接收内核注入的 `ToolCtx`，
  工具**不得**有任何绕开沙箱执行进程的入口。新增工具必须遵守。
- **模型可见即已落日志**：凡进入模型请求的内容都要能从会话日志重建。
  新增模型可见输入必须同时落盘，否则回放（T2）与审计失效。
- **依赖只能向下**：`L0 protocol → L1 platform → L2 core → L3 capability → L4 → L5 host`。
  由 `docs/neo-plan/05-验证/checks/check_architecture.py` 强制。
- **零 `unsafe`**（当前 20 crate 全零）。确需引入必须在提交信息里说明理由与安全论证。
- **零 warning**：`scripts/verify.sh` 会把 warning 判为失败。
- **每个 SPI 必须 ≥2 真实后端 + conformance**，否则是假 SPI（AP-01/AP-03）。
  门禁 `check_spi_conformance.py` 会拒。
- **内存有界性是内核义务**：Rust 只消除 UB，不保证有界。新增可能产生大输出的
  路径必须受上限约束并如实上报 `truncated`。

## 旧实现的位置

早期 TypeScript 实现（DSH 插件 + Electron 壳）已整体移到 `legacy/`，**不再主线**。
它是可运行的（129 测试、真机验证过），作为设计参考与经验来源保留：
- `legacy/PROJECT_MEMORY-proteus-code.md` 记录了 Electron/DSH 路径踩过的全部坑
  （哪些会随 Rust 重写消失、哪些不会）。重写时值得先读。

## 提交约定

一个逻辑改动一个 commit，消息写清「为什么」（约束、踩过的坑），而不只是「改了什么」。
