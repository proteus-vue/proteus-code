# NEO Desktop（Tauri）

Codex / ZCode / MiMo 风格的桌面壳，**只通过 stdio JSON-RPC 对接自家 `neo app-server`**
（与编辑器客户端同一路径，不依赖任何 `neo-*` crate）。

本地三家解包规格：`/Volumes/data1/work/WorkBuddy/2026-09-22-10-34-07/codex-style-agent-desktop-spec/`
（数值令牌已抄入 `src/styles/tokens.css`，无第三方源码）。

## 架构

```
React WebView  ──invoke──▶  Tauri Rust  ──stdio JSON-RPC──▶  neo app-server
     ▲                         │                                    │
     └──── event neo-event ────┘                                    ▼
                                                              neo 内核 (mock/真实 provider)
```

## 开发

```bash
# 仓库根（保证 target/debug/neo 存在）
cargo build -p neo-code-cli

cd apps/desktop-tauri
npm install
npm run tauri dev
```

- 默认 provider：`mock`（无需 API key）
- 指定二进制：环境变量 `NEO_BIN=/path/to/neo`
- 指定工作区：`VITE_NEO_WORKSPACE=/path/to/ws` 后 `npm run tauri dev`

## P0（已做）

- 四区骨架 + 设计令牌（本地三家规格数值）
- `initialize` 握手、事件流（turn / 流式 / 工具卡 / 审批 / 错误）
- `thread/*`、`models/list`、`git/info`、`approval/respond`
- Enter 发送 · Shift+Enter 换行 · Esc 中断

> **产品形态真源**：先读 [`docs/PRODUCT-IA.md`](docs/PRODUCT-IA.md)，再改 UI。
> 禁止单靠截图打补丁。

## 复刻标准进度（对照 `docs/desktop-parity.md` D1–D12）

| # | 项 | 状态 |
|---|---|---|
| D1 | 左侧会话栏 | ✅ 列表/切换/新建 + 状态点 + `+/-` + **标题搜索**；分组/视图 ⬜ |
| D2 | Markdown 正文 | ✅ |
| D3 | 思考轨迹（折叠 + 搜索） | ✅ |
| D4 | 工具卡片分组 + 参数摘要 | ✅ |
| D5 | diff 渲染 | ✅ 行着色 · 并排/统一 · 全屏 |
| D6 | 轮摘要（token + 耗时） | ✅ |
| D7 | 右侧 Goal 面板 | ✅ |
| D8 | 命令台（不经模型） | ✅ 一次性 `command/exec`；`session:true` 持活 PTY + write/resize/terminate（内核侧） |
| D9 | 命令中心 ⌘K | ✅ |
| D10 | 审批阻塞 composer + 三档 + 风险常驻 | ✅ |
| D11 | 执行模式切换 | ✅ 下拉 + ⇧Tab + 面板 |
| D12 | 文件树 / Wiki | ✅ 树+预览+引用 · 改动列表 · **Repo Wiki**（敏感整篇排除）；真 PTY 外的浏览器 ⬜ |

**右栏 Tabs**：Goal · 文件 · 改动 · Diff · **Wiki**  

快捷键：`⌘K` `⌘N` `⌘B` `⌘J` `⌘L` `⇧Tab` · Diff `v` · `Esc`

## 独立 workspace

`src-tauri/Cargo.toml` 自带 `[workspace]`，根 `Cargo.toml` 已 `exclude`，
**不进入内核 L0–L5 架构守卫**。
