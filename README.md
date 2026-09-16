# NEO —— 用 Rust 重构的编程 Agent 内核

> **一句话**：以 DeepSeek Harness（DSH）为起点，吸收 **ZCode（Z.ai）** 的交互范式与
> **OpenAI Codex CLI** 的运行时架构，用 **Rust** 重写内核，交付
> **TUI / Desktop（系统 webview）/ Web / Exec 四宿主共享同一内核** 的编程 Agent。
>
> **为什么是 Rust**：为了机制层面的两个硬要求 —— **高性能**（无 GC 停顿、零拷贝主循环）
> 与**内存安全**（零 `unsafe`、类型层面无数据竞争）。附带收益是去掉过重的壳
> （Electron ~78 MB → 系统 webview ~5–10 MB）。

---

## 当前状态（诚实标注）

| 层 | 状态 |
|---|---|
| 内核主循环（turn/step、审批挂起恢复、三维闸门） | ✅ **已实现**，16 项 conformance |
| 内存有界性（输出上限、UTF-8 安全截断、上下文上限） | ✅ **已实现**，6 项实测 |
| `ModelProvider` 真实后端（DeepSeek chat-completions） | ✅ **已实现**，HTTP 帧对真实服务器验证通过 |
| **多模型注册表 + 运行时切换**（`/models`、设置页可选） | ✅ **已实现**，真实模型实测切换后下一轮走新 provider |
| **多会话**（`/sessions` 切换、`/new` 新建、历史重建） | ✅ **已实现**，pty 实测切换后历史逐条重建且转录正确 |
| `SandboxBackend` 真实后端（macOS Seatbelt） | ✅ **已实现**，6 项**真机**越权拦截测试 |
| `SessionPersistence` 真实后端（JSONL append-only） | ✅ **已实现**，重开续号已验证 |
| `neo exec` 无头宿主（端到端闭环） | ✅ **已实现**，离线 + 真实 provider 双路径跑通 |
| `neo tui` 终端宿主（NEO 紫主题 + 多行输入 + @// 弹窗 + 鼠标 + 滚动搜索 + 审批 diff + 右侧面板 + Markdown 高亮 + 信任门） | ✅ **已实现**，pty 逐项验证（含宽/窄终端、resize 重绘） |
| `neo serve` Web 宿主（浏览器界面 + SSE 事件流 + 审批 + 目标编排） | ✅ **已实现**，端到端验证（订阅→提交→审批→真落盘）；目标栏浏览器实测（设定→自动逐阶段推进→暂停/恢复/清除） |
| **T6 宿主语义等价**（同一事件流多宿主比对） | ✅ **铁律生效**：headless / TUI / desktop / **web** 四宿主事实完全等价 |
| Shell 工具真实执行（经沙箱） | ✅ **已实现**（`bash` 真跑，输出受限） |
| **真实模型完整推理** | ✅ **已验证**（真实模型 → 真实工具调用 → 真实沙箱 → 真实落盘，端到端跑通） |
| `apply_patch` 落盘 | ✅ **已实现**（经 `ctx.write_file` 走沙箱，唯一匹配校验，真机验证落盘） |
| Web 宿主（零依赖 HTTP + SSE） | ✅ **已实现**（`neo-host-web`；`POST /api/turn` 提交、`GET /api/events` SSE、`/api/approve` 审批、`/api/goal` 编排）。**端点需访问令牌**：启动时生成 256 位随机令牌，除内置页面外一切路径都校验（含不存在的路径，未鉴权者枚举不出路由）；令牌经 URL fragment 下发、页面自动接在每个请求上，程序化客户端可用 `X-Neo-Token` 头。**HTTP 层有 23 项集成测试**（真实监听端口 + 真实路由，只把内核侧换成假内核）：页面/404、引用解析、SSE 线格式与多订阅者扇出、审批 decision 映射与 id 解码、goal 各 action 与 409/400 分支、断连回收、**鉴权 7 项（负向用例 + 路由不可探测 + 页面接线断言）** |
| **Desktop 宿主**（系统 webview 窗口） | ✅ **已实现**：`neo desktop` 打开系统 webview 窗口（macOS WKWebView / Windows WebView2 / Linux WebKitGTK），**复用 Web 宿主全栈**（本地回环端口 + 内置页面，T6 等价天然成立），窗口关闭即退出。窗口内交互验证需真人实机（进程/服务/SSE 连接已机器验证） |
| **桌面原生 GUI 宿主**（egui，**已冻结**） | 🧊 **不再是默认**（2026-09 起默认是 GPUI）：`neo desktop --egui` 才走它。以下记录是它作为默认期间的完整验证历史，保留作为对照：`neo desktop` 曾默认走原生 GUI（egui/eframe，不经 HTTP、不开端口）；`neo desktop --webview` 保留上一条的 webview 路径。已接上 D2–D7 的数据通路（Markdown 正文 / 思考轨迹 / 工具卡片含参数摘要 / **diff 渲染** / 轮摘要 / Goal 面板）与审批（三档 Allow/Always/Reject，未决时**阻塞输入**），复用 `neo-text` 的同一份文本语义与调色板（含防漂移守卫）。**已机器验证**：能编译、真机开出窗口并存活、驱动层有 `driving` 边界守卫（越界 Pump 不触发多余模型请求，有回归测试）、T6 契约已纳入 conformance。**已完成视觉验收**（2026-09-16，授权后真机截图）：中文与 D2–D7 全部正确渲染（思考块 / 工具卡片含参数与输出 / 任务清单 / Markdown 代码块高亮 / 引用 / 轮摘要），代码块缩进正确。**真机全流程已验证**（打字 → 回车提交 → 审批 diff → 批准 → 文件真实落盘 → 输入框恢复）。**过程中抓到并修掉三个机器测不出的缺陷**：egui 默认字体不含 CJK 字形，界面中文全是豆腐块 —— 编译、开窗、驱动测试全绿也照样发生。已加中文字体加载（平台候选表，不打包字体）+ **可机器判定的界面可用性门禁**（`verify_cjk_renderable`，用 `Fonts::has_glyph` 直接查字形，并有反例测试证明它有牙齿）；**审批前的 diff 曾可能与实际改动不是同一个文件**（preview 按进程 CWD 读、execute 按工作区解析，`--workspace` 非进程目录时指向不同文件 —— 用户照着预览批准却改了别的文件），已修为共用同一解析规则并补 4 条回归测试；输入框补自动聚焦（否则必须先点一下才能打字）。**已补 D11**：状态栏执行模式下拉 + `Shift+Tab` 循环（对齐 ZCode），高风险档位在工具栏**常驻**风险提示，模型可运行时切换。**已补 D9**：`Cmd/Ctrl+K` 覆盖式命令面板（搜索过滤、↑↓ 选择、Enter 执行、Esc 关闭，11 条命令）。**已补 D1**：左侧会话栏（列表 + 当前项高亮 + 新建，可切回；当前会话即使尚未落盘也显示）。**已补 D8**：底部命令台（`/terminal` 开关），命令走 `Op::Shell` **不经模型**、经沙箱、与模型工具调用同一条执行路径。**现状**：已冻结（只修 bug，不加功能），保留作回退通道；gpui 稳定后删除 |
| **桌面 GPUI 宿主**（**默认桌面 UI**，2026-09 转正） | ✅ **已转正**：`neo desktop` 默认打开 GPUI 窗口。D1–D11 parity 已覆盖，并在三处领先旧实现（思考轨迹搜索、工具调用分组、只在底部才跟随的自动跟随）。真机验证：中文、D2–D11 全部渲染正确，打字→回车提交→审批→落盘全流程通过，1x/2x 两档 DPI 无错位，IME 有自动化回归用例。它一行 gpui 依赖都不直接声明 —— 全部经 UI 栈取用（单一 pin，门禁强制）。**egui 未删**：`--egui` 保留为回退通道，待稳定后删（阶段 3 剩余项） |
| 上下文压缩（L4 策略） | ✅ **已实现**：`Compactor` seam 在 L2、策略在 L4；`/compact` 端到端，摘要与移除条数可回放 |
| **Goal 目标编排**（`/goal` 系列） | ✅ **三宿主可用**：TUI `/goal <目标>`、exec `--goal`、Web 目标栏（`/api/goal`）。自动逐阶段推进（Plan→Code→Review→Learn），审查失败回退重做 ≤3 次（含**模型显式叫停**），四项停止条件生效；快照事件落日志，**kill 后重启从日志重建续跑**。审查是硬失败信号 + 模型自评（沉默视为通过）；挂钟停止条件未实现（破坏回放确定性） |
| **服务商管理**（设置页可增删改） | ✅ **已实现**：设置页内联表单新增/编辑（密钥字段打码、留空不改），删除需确认。密钥存 `~/.neo/provider_keys.json`（**0600**，与配置分离）；`base_url` 自动规范化为裸主机。缺 key 的条目跳过并提示 |
| **上下文引用解析**（`@file` / `$skill`） | ✅ **已实现**：`@path` 与 `@path#行范围` 经沙箱读入并注入请求；`$skill` 查注册表注入正文、找不到则列出可用项。注入块落 `RefsResolved` 日志，回放一致 |
| **子代理**（Markdown 定义 + 工具白名单） | ✅ **已实现**：`.neo/agents/*.md`（frontmatter: name/tools/model），注册为 `agent_<名>` 工具；子内核审批 Never、沙箱硬边界继承、过程不进主转录。真机验证主模型委派成功 |
| **项目指令级联**（`AGENTS.md`） | ✅ **已实现**：`~/.neo/AGENTS.override.md` → `~/.neo/AGENTS.md` → 仓库根 → 子目录（越具体越靠后），合并上限 32 KiB，超限如实标注截断。并入系统提示词并落 `InstructionsLoaded` 日志，回放还原同一份提示词 |
| **MCP 外部工具 + 资源**（stdio + Streamable HTTP，JSON-RPC） | ✅ **已实现**：`~/.neo/mcp.json` 声明服务器（`command` 本地进程 / `url` 远程端点，恰填其一；用户级，项目级显式拒绝），外部工具入 `ToolRegistry` 复用审批/上限/落盘整条链路；**资源**以每服务器一个 `mcp__<server>__read_resource` 工具暴露（描述内嵌资源目录，模型可控调用）。提示模板（prompts）未接入 |
| Linux / Windows 沙箱 | ❌ **未实现**（**fail-closed**：受限档位拒绝执行，不降级放行） |

**一句话现状**：内核 + 真实沙箱 + **真实模型** + 落盘 + **四宿主（exec / TUI / Web / Desktop）**
已全部闭环并跑通；T6 宿主等价铁律在四宿主上生效（桌面复用 Web 栈）。

### 离线自验（不需要 API key）

三个内置桩 provider，各自覆盖不同链路。**没有 key 也能把界面与内核交互全走一遍**：

| provider | 它产出什么 | 能验证到哪些功能 |
|---|---|---|
| `mock` | 一句固定文本 | 信任门 · 首页 · `@` 文件弹窗（过滤/选择）· `/` 命令弹窗 · `/help` `/keys` `/status` · `ctrl+p` 面板 · `ctrl+t` 主题 · `ctrl+b` 侧栏 · `ctrl+r` 历史 · `!shell` 执行 · `@file`/`$skill` 引用注入 · 普通对话 |
| `selftest` | 调一次 `apply_patch` 写文件 | 审批拦截 → **审批前 unified diff 预览** → `y` 批准 → 真实落盘 → 侧栏 Modified Files（+N -N） |
| `demo` | Markdown 正文 + `todowrite` | Markdown 渲染（标题/列表/行内代码/代码块**语法高亮**/引用）· 正文与侧栏的**任务清单**进度 |

```bash
neo tui --provider mock        # 界面与交互全览
neo tui --provider selftest    # 审批 / diff 预览 / 落盘（批准后会写 ./selftest.txt）
neo tui --provider demo        # Markdown 高亮 + 任务清单
```

**离线验不到的**（必须真实模型）：真实推理质量、多轮工具编排、
模型是否真的会调 `todowrite` 维护清单。压缩策略（`/compact`）、引用解析
（`@file` / `$skill`）与项目指令级联（`AGENTS.md`）都已在离线桩上端到端验证：
依次落 `context_compacted` / `RefsResolved` / `InstructionsLoaded` 日志，回放一致。
界面本身与 provider 无关，所以这些以外的交互都能离线确认。

**已知边界**：技能目录与 `AGENTS.md` 都在**启动时**加载一次（装配是冷路径，
引用与提示词是热路径），所以会话中途新增技能或改约定需要重启才可见；
`@` 引用暂不支持带空格的路径（`@"my file.txt"` 语法未实现）；
`AGENTS.md` 超 32 KiB 时**诚实截断**（标注"已截断"），尚未实现规划中的
"让模型生成握手摘要"。

### 安装

四种方式，按「省事 → 可控」排列：

**① 一键脚本（推荐，无需 Rust）** —— 下载预编译二进制：

```bash
curl -fsSL https://raw.githubusercontent.com/proteus-vue/proteus-code/main/scripts/install.sh | sh
```

装到 `~/.local/bin/neo`（可用 `NEO_INSTALL_DIR` 改）。预编译产物覆盖
macOS（Apple Silicon / Intel）与 Linux x86_64；脚本会校验 SHA-256。

**Linux 预编译产物是精简版**（不含任何桌面窗口），原因有两条且**都关于
系统库**，不是关于代码：
- webview 桌面需要 `libwebkit2gtk`（构建期 pkg-config + 运行期动态库）；
- GPUI 桌面需要 `libxkbcommon` / `libxkbcommon-x11` / `libwayland` / `libX11`。
  其中 `libxkbcommon` / `libwayland` / `libX11` 走 **dlopen（运行时加载）**
  —— 编译不需要它们，但**运行时缺了会直接 panic**
  （`Library libxkbcommon.so could not be loaded.`）。
  而 **`libxkbcommon-x11` 是链接期要求**（Ubuntu 包名
  `libxkbcommon-x11-dev`）：缺它会在**链接**时报
  `unable to find library -lxkbcommon-x11`。
  注意这个差异很坑：`cargo check` 不做链接，所以"编译检查通过、测试/打包失败"。
  完整清单（Debian/Ubuntu）：
  ```bash
  sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev \
       libwayland-dev libx11-dev libfontconfig1-dev
  ```
  也就是说 Linux 上自行构建含桌面的版本时，编译能过、启动才炸，
  所以这里必须写清楚要装什么。

Linux 上要桌面窗口：用方式 ③ 自行构建，并先装好上述系统库；或继续用 TUI
（命令行体验完整，不受影响）。**Linux 桌面产物尚未验证**（未在 CI 或真机上
跑过含 gpui 的 Linux 构建），这一点如实记在项目缺口清单里。

**② npm（有 Node 环境时最省事）**：

```bash
npm install -g @proteus-vue/neo-code
```

安装期**不执行脚本、不联网**：二进制放在按平台拆分的可选依赖里
（`@proteus-vue/neo-code-darwin-arm64` / `-darwin-x64` / `-linux-x64`），npm 只装匹配的
那一个。平台边界与方式 ① 相同（Linux 包不含桌面宿主）。

**③ cargo install（需 Rust）** —— 覆盖所有平台与 feature 组合：

```bash
# 从源码仓库（含桌面窗口，默认 feature）
cargo install --git https://github.com/proteus-vue/proteus-code -p neo-code-cli --locked

# 精简版：不要桌面窗口，Linux 上免装 libwebkit2gtk
cargo install --git https://github.com/proteus-vue/proteus-code -p neo-code-cli --locked --no-default-features
```

> 包名是 `neo-code-cli`，但**命令名始终是 `neo`**。

**④ 从源码构建**：

```bash
cargo build --release -p neo-code-cli   # 产物在 target/release/neo
```

装好后若 `neo` 提示 command not found，是因为安装目录不在 `PATH` 里
（`cargo install` 装到 `~/.cargo/bin`，脚本装到 `~/.local/bin`，
npm 用 `npm prefix -g` 下的 `bin`）。

### 亲测可用

```bash
# 交互式（TUI）：首次进入某目录会先问"是否信任"（落盘 ~/.neo/trusted.json）
# 首页：星场 + 居中 logo + 带边框输入框（内含 模式 ⏵ 模型）
#   @          文件弹窗（↑↓ 选择 / 实时过滤 / Enter 确认）
#   /          命令弹窗（help / keys / status / theme / new / compact / exit）
#   审批时会先显示 unified diff（不再盲批）
#   右侧面板：Context（token 占用）/ Todo（进度）/ Modified Files（+N -N）
#   ctrl+b 开关侧栏（<96 列自动隐藏）
#   !cmd       直接执行 shell（走同一沙箱），输出进会话
#   ctrl+p     命令面板（分四组 + 右侧键位提示 + 模糊搜索）
#   /settings  设置界面（系统 / 模型 / 会话 / 显示）
#   ctrl+t     切换主题（7 套，默认 NEO 紫）
#   @file#12-40 引用文件的行范围
#   助手回复按 Markdown 渲染（代码块语法高亮、行内代码、标题、列表）
#   多行输入：最多显示 6 行（超出滚到末尾），带光标移动与整行编辑
#     alt+enter 换行 · ctrl+k 删到行尾 · ctrl+u 删到行首 · ctrl+w 删词
#     ctrl+z 撤销 · ctrl+y 重做 · home/end · alt+b/f 按词移动
#   转录阅读：pageup/pagedown 半页滚动 · ctrl+e 回最新 · ctrl+f 搜索
#   鼠标：滚轮滚动转录 · 点击选弹窗项 · 点侧栏切换（收起后点右下角 ‹ 展开）
#   全屏 diff 查看器：/diff 或审批时按 d（hunk/文件跳转、双列视图、文件树）
#   ctrl+/ 键位提示（which-key，按上下文显示此刻能用的键）
#   /undo 回退对话一轮（**不还原文件**，界面会明确标注）
#   /details 展开工具输出（默认折叠；失败时总是展示）
#   /thinking 显示推理过程（默认隐藏）
#   /copy 复制最近一条回复到系统剪贴板
#   ctrl+z 挂起回 shell（需终端支持作业控制；fg 恢复）
#   提醒（提示音/桌面通知）：默认关，/notify 或 NEO_TUI_NOTIFY=1 开启
#     完成 / 出错 / 需审批时提醒（macOS osascript+afplay / Linux notify-send）
#   外观：/background 切背景纹理（星场/点阵/斜纹/纯色）
#         /logo 切 Logo 样式（大 6 行 / 小 3 行 / 极简 1 行 / 隐藏）
#         NEO_TUI_BG_FILE=<字符画文件> 可用自定义背景（零依赖，不做图像解码）
#   Tab 补全 / ctrl+r 历史 / ctrl+g 编辑器 / ctrl+l 清屏 / esc 关弹窗
neo tui --provider selftest --mode default
# 等价写法：neo --provider selftest --mode default（裸选项默认进 TUI）

# Web 宿主：启动后用它打印的完整 URL 打开（形如 http://127.0.0.1:8787/#token=…）
#   端点需要访问令牌，裸地址打开会 401 —— 用打印出来的那条
neo serve --provider selftest --mode default

# 离线跑通（不需要 API key）—— 验证装配 → 内核 → 沙箱 → 落盘
cd /tmp && neo exec "列出文件" --provider mock --mode auto-edit

# 接真实模型（需要一个有效的 DEEPSEEK_API_KEY）
export DEEPSEEK_API_KEY=sk-...
neo exec "用一句话回答 1+1" --mode plan
```

## 快速开始（开发仓库）

```bash
# 需要 Rust —— 版本由 rust-toolchain.toml 钉定（1.95.0），rustup 会自动选用
cargo run -p neo-code-cli -- tui --provider mock   # 不装 PATH，直接用 cargo 跑 TUI

cargo test --workspace     # 806 个测试
cargo check --workspace    # 25 个 crate，零 unsafe、零 warning

bash scripts/verify.sh     # 全套门禁：架构 / 协议 / 会话 / 配置 / 模式矩阵 / SPI / 执行效率 / 测试
```

发布（维护者）：打 tag 即触发 `.github/workflows/release.yml`，出多平台二进制并
（配置了 npm 凭证时）发 npm 包。

```bash
git tag v0.1.0 && git push origin v0.1.0
```

完整步骤（含首次发布、npm 凭证、撤销版本、排错）见
**[docs/RELEASE.md](docs/RELEASE.md)**。

## 官网

**<https://neo.proteus-vue.cn>** —— 用本组织自己的 [Proteus](https://github.com/proteus-vue/proteus)
跨端框架构建（dogfooding），源码在 [`website/`](website/README.md)。

```bash
cd website && npm install && npm run dev    # 或 npm run build → dist/web
```

推到 `main` 且改动涉及 `website/**` 时，`.github/workflows/website.yml` 自动构建并部署到 GitHub Pages。

## 仓库结构

```
proteus-code/                  ← 项目本体是 Rust 内核
├── Cargo.toml                 workspace（31 crates）
├── crates/                    ★ 内核与宿主
│   ├── neo-protocol/          L0 线协议（Op / EventMsg / 双轴枚举），零业务依赖
│   ├── neo-text/              L1 基础：宿主中立的文本语义（色调 / 宽度 / Markdown / 高亮）
│   ├── neo-ui-kit/            L1 UI 门面（唯一 pin GPUI 的地方）
│   ├── neo-ui-render/         L2 UI 渲染缝（中立颜色/几何 + RenderBackend）
│   ├── neo-ui-behavior/       L3 UI 行为层（焦点仲裁 / 按键路由，与后端解耦）
│   ├── neo-ui/                L4 UI 设计系统（品牌主题 + 组件）
│   ├── neo-driver/            L3 GUI 共享内核驱动（driving 边界守卫所在）
│   ├── neo-sandbox/           L1 平台（命令包裹：Seatbelt / Landlock+bwrap / ACL）
│   ├── neo-platform/          L1 平台（进程加固 / fs notify / git）
│   ├── neo-core/              ★ L2 内核：turn/step 主循环 + 三维闸门 + 4 个 SPI 契据
│   ├── neo-capability/        L3 能力（Shell-First 工具集：bash / apply_patch / 提问）
│   ├── neo-orchestration/     L4 目标编排（Goal 引擎）
│   ├── neo-host-tui/          L5 宿主：终端
│   ├── neo-host-desktop/      L5 宿主：系统 webview（替代 Electron）
│   ├── neo-host-egui/         L5 宿主：桌面原生 GUI（egui；neo desktop 默认走它）
│   ├── neo-host-gpui/         L5 宿主：桌面 GPUI（**默认桌面 UI**）
│   ├── neo-host-web/          L5 宿主：浏览器（零依赖 HTTP + SSE，含内置页面）
│   ├── neo-exec/              L5 宿主：无头 / CI
│   ├── neo-code-cli/          `neo` 入口（multitool）
│   ├── neo-session/           会话真相源（append-only）
│   ├── neo-session-store/     多会话库（列举 / 新建 / 删除 / 标题）
│   ├── neo-skill-loader/      技能目录发现与加载（SKILL.md → SkillRegistry）
│   ├── neo-instructions/      项目指令级联加载（AGENTS.md → 系统提示词）
│   ├── neo-providers/         服务商注册表（用户级 providers.json）
│   ├── neo-config/            四级配置 + 模式解析
│   └── neo-mock/              test-support：各 SPI 的 Mock 后端 + 反例后端
├── docs/
│   ├── neo-plan/              ★ 设计方案：架构 / 模块规格 / 里程碑 / 验证套件
│   │   ├── 02-架构设计/       ★ 方法论与 SPI 纲领（先读这个）
│   │   ├── 03-模块规格/       逐层可落地规格
│   │   ├── 04-落地计划/       M0–M6 里程碑与验收
│   │   └── 05-验证/           可执行验证套件（Python + golden）
│   └── spi-first-methodology/ 方法论原文（跨 16 次生产泛化）
├── npm/neo-code/              npm 发布包装层（bin/neo.js 只按平台转发到真二进制）
├── website/                   官网（用 Proteus 框架构建 → neo.proteus-vue.cn）
├── scripts/verify.sh          全套门禁入口
├── scripts/install.sh         一键安装（下载预编译二进制 + 校验和）
├── scripts/publish-npm.sh     发布 npm 包（平台包 + 主包，支持 --dry-run）
└── legacy/                    旧实现（Electron + DSH 插件），仅作参考
```

> **`legacy/` 是什么**：本项目早期用 TypeScript 写的「DSH 插件 + Electron 壳」实现。
> 它**可运行**（129 个测试、真机验证过），但方向已改为 Rust 内核，故整体退到
> `legacy/` 作为**设计参考与经验来源**（哪些坑会随重写消失、哪些不会）。

---

## 设计立场（三句话）

1. **不是「一切皆插件」，但也不是「写死内核」。**
   DSH 的病不是插件太多，而是**约 90 个 seam 没有名字、没有门禁** ——
   无法静态分析、无法验证可替换性。NEO 把它们**收敛为 5 个有名 SPI**，
   每个都强制「契约 + ≥2 后端 + conformance」。这样**同时得到可替换性与可定位性**。

2. **宿主只是内核的一个消费者。**
   TUI / Desktop / Web / Exec 不持有业务状态，只消费同一条事件流（Op / EventMsg）。
   这是 Codex 多宿主设计的精髓，也是「改 UI 不动内核」的前提 ——
   并用 **T6 铁律**机器验证（同一事件流喂多个宿主后端，断言语义等价）。

3. **安全是正交双轴，且沙箱是内核的结构保证。**
   沙箱决定「能做什么」，审批决定「何时必须问」。二者必须独立配置。
   更关键的是：`Tool::execute` 接收由内核注入的 `ToolCtx`，**工具在类型层面就没有
   绕开沙箱的入口** —— 安全边界不能靠约定。

---

## 五个 SPI（可替换性的全部出口）

| SPI | 语义定义 | 已实现后端 | conformance |
|---|---|---|---|
| `ModelProvider` | 换模型（流式增量，支持工具调用） | deepseek / scripted / mock ×2 | ✅ |
| `SandboxBackend` | 换沙箱实现 | local(Seatbelt 真机) / mock ×2 | ✅ |
| `SessionPersistence` | 换会话存储介质 | jsonl(真落盘) / in-memory / tampering(反例) | ✅ |
| `HostBackend` | 换宿主（TUI/Desktop/Web/Exec） | desktop / tui / web / mock ×2 | ✅ 4 宿主 T6 等价 |
| `Tool` | 加能力（Shell-First） | 3 内置 + mock ×4 | ✅ |
| `Clipboard` | 交给系统剪贴板 | system(pbcopy/wl-copy/xclip) + noop | ✅ |
| `Notify` | 让用户注意到某件事 | system(osascript/notify-send) + noop | ✅ |

**每个 SPI 都配了「坏后端」作为负向用例的被试** —— 一个不能被 conformance 抓住的
反例，等于套件没有牙齿。

---

## 文档导航

| 文档 | 内容 |
|---|---|
| [docs/neo-plan/README.md](docs/neo-plan/README.md) | 设计方案总览（先读执行摘要） |
| [docs/neo-plan/02-架构设计/Proteus方法论-语义核心与后端SPI.md](docs/neo-plan/02-架构设计/Proteus方法论-语义核心与后端SPI.md) | **方法论纲领**：为什么 5 个 SPI、为什么 Rust |
| [docs/neo-plan/03-模块规格/](docs/neo-plan/03-模块规格/) | 逐层规格（L0–L5） |
| [docs/neo-plan/04-落地计划/](docs/neo-plan/04-落地计划/) | M0–M6 里程碑与验收标准 |
| [docs/spi-first-methodology/](docs/spi-first-methodology/) | SPI-First 方法论原文与自检清单 |
| [PROJECT_MEMORY.md](PROJECT_MEMORY.md) | 项目记忆：决策依据、踩过的坑、被证伪的方案 |
| [docs/RELEASE.md](docs/RELEASE.md) | **发布检查清单**：首次发布 / npm 凭证 / 撤销与排错 |
| [website/README.md](website/README.md) | **官网**源码与选型说明（用 Proteus 框架构建） |
| [docs/desktop-plan.md](docs/desktop-plan.md) | **桌面版实现计划**（Rust 原生 GUI，阶段 A–E） |
| [docs/desktop-parity.md](docs/desktop-parity.md) | **桌面版对标规格**：Codex / ZCode 桌面版逐条对照 |
| [docs/opencode-parity.md](docs/opencode-parity.md) | TUI 对标规格（opencode / mimo，P1–P14） |
| [AGENTS.md](AGENTS.md) | 在本仓库工作的约定（效率标准 + Rust 约束） |
| [docs/ai-efficiency-rules/](docs/ai-efficiency-rules/) | **强制技能**：AI 执行效率规范（六类低效行为 / 三条总则 / 可跑脚本 / CI 审计） |

---

## 协议

MIT，见 [LICENSE](LICENSE)。项目借鉴的 DSH 亦为 MIT —— 两者兼容，
且本项目是对 DSH 的独立重写而非 fork（见 `PROJECT_MEMORY.md` 的方向变更记录）。
