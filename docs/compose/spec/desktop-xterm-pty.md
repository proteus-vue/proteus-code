---
feature: desktop-xterm-pty
status: delivered
updated: 2026-09-24
branch: main
commits: 9913eda8..5fef1848
---

# Desktop xterm + PTY Terminal

## Report

**What was built** — 桌面右栏「终端」改为 xterm 交互窗，经 app-server `command/exec?session` 接内核 portable-pty。输入走 `execWrite`，空闲用 `execWrite("")` 拉增量输出（≥180ms），`FitAddon` 同步 resize，关标签/unmount `terminate`。写占用时按键入队不丢；写失败先 terminate 再清会话 id。旧 term-log 主路径删除，palette 仍可用一次性 `command/exec`。

**Verification** — `npx tsc --noEmit` PASS · `npm run build` PASS（xterm 使 chunk>500kB，预期）· 效率审计 0/0/0 · 审查 2 critical 已修并复审 PASS。

**Journey log**
- SessionManager `write` 是 `mem::take` 增量 + 最多约 80ms 等待 → 适合拉模型，但并发 write 必须串行/排队。
- 排水错误若只清 `sessionIdRef` 会孤儿 PTY → 先 capture id 再 terminate。
- `onData` 在 draining 时直接 return 会丢快速击键 → pendingInputRef 队列。

## [S1] Problem
桌面右栏「终端」目前是简易命令日志：一次性 `command/exec` 或手动 `pty` 文本会话，没有 xterm 交互终端（光标、ANSI、resize、持活输入）。PRODUCT-IA IA-9 / P2 明确桌面 xterm 接线 ⬜；内核 SessionManager（portable-pty）与 `command/exec?session` + write/resize/terminate 已就绪。

## [S2] Design
- **依赖**：`@xterm/xterm` + `@xterm/addon-fit`（仅桌面前端，不进内核）。
- **组件**：新 `components/TerminalPane.tsx`；打开时 `commandExecSession(shell)` 取 `session_id`，xterm 挂载后写入初始 `output`。
- **输入**：xterm `onData` → `execWrite(sessionId, data)`；响应里的 `output` 写入 `term.write`。
- **拉输出**：会话空闲时用 `execWrite(sessionId, "")` 轮询排水（`SessionManager::write` 已 `mem::take` 增量；间隔 ≥150ms，仅 `running` 时）。
- **resize**：`FitAddon` + 窗口/容器 resize → `execResize(sessionId, cols, rows)`。
- **结束**：组件 unmount / 关标签 / 切会话 → `execTerminate`；`running:false` 停轮询并在 xterm 提示退出码。
- **UI**：替换 `panelTab === "terminal"` 现有 `term-log`+`term-input`；保留简短 note（沙箱会话、不经模型）。旧一次性 shell 入口可收进 palette，不删 `commandExec` RPC。
- **错误**：start/write 失败显示在 xterm 底行或 note，不吞。

## [S3] Out of Scope
- 不改内核 PTY / SessionManager 语义。
- 不引入 TCP app-server、不绕过沙箱。
- 不做 tmux/多会话复用 UI（单会话即可）。
- 图片附件、⌘F 等其它优先级项不在本 feature。

## Tasks
- [x] T1: 安装 @xterm/xterm + @xterm/addon-fit 并确保 `npm run build` 通过 — acceptance: package.json 含依赖且 build 绿 (covers: S2)
- [x] T2: 实现 TerminalPane（start / onData write / 空闲排水 / fit resize / terminate） — acceptance: 打开终端标签得到活 shell，按键有回显，窗口变宽后 `stty size` 跟随 (covers: S2; depends: T1)
- [x] T3: 接入 App 终端 tab，替换旧 term-log UI — acceptance: panelTab terminal 渲染 xterm；旧输入框不再作为主路径 (covers: S2; depends: T2)
- [x] T4: 样式与 PRODUCT-IA 回填 — acceptance: 终端面板铺满 wb-body；IA-9/ P2 状态更新为桌面 xterm 已接 (covers: S2; depends: T3)
