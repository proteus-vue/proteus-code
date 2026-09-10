# proteus-code

> **一个面向 Proteus 跨端框架的桌面 AI 编程应用**，基于 DeepSeek 开源的 **DSH（DeepSeek Harness）** 构建。

proteus-code 把两件事接在一起：

- **DSH** —— 一个「万物皆插件」的 agent harness（Cordis 插件树 + Web UI + Electron 桌面壳）。它提供 agent 循环、会话、工具系统、权限沙箱和浏览器界面。
- **Proteus** —— 一套「统一语义收敛」的跨端框架（一套语义内核，可插拔渲染/编译/宿主后端）。它提供语义原语、编译门禁、conformance 套件和 `proteus` CLI。

proteus-code 在这一层之上做了一件事：**把 Proteus 的 CLI 能力、领域知识和身份标识，做成一个 DSH plugin bundle**，再用一个 Electron 壳把它作为独立桌面应用启动。DSH 的 agent 因此能真正调用 Proteus 的门禁与诊断命令来验证工作，而不是凭记忆断言正确。

---

## 它长什么样

桌面应用启动后是一个完整的 AI 编程界面：左侧会话/工作区栏（已替换为 proteus-code 品牌）、中间对话框、模型与工作区选择，底部设置入口。

界面本身来自 DSH 的 Web Client，proteus-code 通过两个 DSH 官方扩展点改写它：

- `system-prompt.personaPrefix` —— 声明式地注入 proteus-code 的 agent 人格（无需改代码）
- 客户端 slot（`sidebar.brand.*` / `conversation.hero.brand.mark`）—— 用自带的 React 组件替换品牌标识

---

## 文档导航

| 文档 | 内容 |
|---|---|
| [PROJECT_MEMORY.md](PROJECT_MEMORY.md) | **交接文档**：决策依据、踩过的坑、被证伪的方案、此后必须遵守的事实 |
| [AGENTS.md](AGENTS.md) | 在本仓库工作的约定（效率标准 + 项目固有约束） |
| [docs/architecture.md](docs/architecture.md) | 架构与扩展点、UI 定制面、上游契约清单 |
| [docs/getting-started.md](docs/getting-started.md) | 上手、配置、排错 |

---

## 界面：全面换成 proteus-code 自己的

界面能力来自 DSH 的 Web Client，proteus-code 通过**官方扩展点**整体改造它，不 fork、不改上游文件：

| 改造对象 | 使用的扩展点 | 效果 |
|---|---|---|
| **整套配色** | `ctx.theme.overrideTokens()` | 覆盖 **79 个 `--dsw-alias-*` 设计 token**（明暗两套），聊天、侧边栏、设置、弹窗、代码块、状态色、滚动条全部改为 Proteus 色板 |
| **品牌标识** | UI slot `sidebar.brand.*` / `conversation.hero.brand.mark` | 自绘渐变「P」标记与 `proteus CODE` 字标，替换官方 whale 标识 |
| **窗口标题** | 主进程 `page-title-updated` + `document.title` 访问器遮蔽 | 标题固定为 `proteus code` |
| **favicon** | `webServer.tapIndex()` 重写 index.html | 内联品牌 SVG，无额外请求 |
| **启动页文案** | 品牌样式表接管 `[data-dsh-boot]` 字标 | 启动时的 `HARNESS` 换成 `proteus code` |
| **首屏标语** | 文案覆盖（见下） | 「探索未至之境」→「一套语义，任意渲染」 |
| **工作区选择器** | UI slot `conversation.hero.workspace`（优先级 -1 遮蔽原实现） | ZCode 式菜单：搜索 / 打开文件夹 / **不在项目中工作**，含悬停与键盘落点高亮 |
| **选中色 / 焦点环** | 注入全局样式表 | 使用品牌靛蓝，替代浏览器默认 |

### 三个层次的改造，按「正当性」排序

1. **声明式与 token 层（首选）**：配色走 `--dsw-alias-*` / `--dsw-specific-*` token，品牌标识走 slot。这两层完全受支持，上游改版只需跟随 token 名与 slot 名。
2. **注入层**：favicon、标题、全局 CSS、液态玻璃层需要改 `index.html` 本身。DSH 的结构化 `IndexInjection` 没有 `title`/`link` 类型，因此走它文档中明确保留的逃生面 `webServer.tapIndex()`。
3. **文案覆盖层（兜底）**：少数文案属于**无法重新注册**的 locale 命名空间（`LocaleRuntime.register` 对同 ns+locale 重复注册直接抛错，而归属插件先注册）。这类文案只能在渲染后的 DOM 里按**精确文本**替换。

第 3 层是唯一有取舍的一层，因此设计上做了三重约束：只按文本内容匹配（不碰 class 与结构，改样式不会破坏它）、完全可逆（disposer 精确还原每个文本节点）、MutationObserver 覆盖后渲染的副本。清单只有一条（首屏标语的中英双语），并在测试中锁定，防止它悄悄膨胀。

### 工作区选择器与「不在项目中工作」

首屏的工作区 chip 上挂的是自己的选择器（接管 `conversation.hero.workspace`，`single` 插槽用**优先级 -1 遮蔽**原实现——框架自己的报错信息就给了这条路径）。三项：搜索过滤、打开文件夹（系统原生选择器）、**不在项目中工作**。

交互上补齐了原实现有、而我们初版缺的**落点反馈**：鼠标悬停与键盘方向键**共用同一个游标**，另有 `Home`/`End`/`Enter`，当前项目单独用选中态标记。菜单打开即聚焦搜索框，可直接输入过滤。

「不在项目中工作」这条值得说明，因为它触及了 DSH 的一个架构事实：**这套 UI 里不存在「无工作区」的可用状态**。输入框要靠 chip 上的标题才能启用，而标题只来自工作区（或会话自身的 cwd）；DSH 原生的「新会话」按钮在没有工作区时也只是清空选择、退回选择器。

所以这条的语义是「**不用你手动挑目录**」，实现上把 harness 的工作目录自动登记为项目再走 `onPick`——和点任意一行完全同一条已验证路径，`create` 幂等因此重复使用只会复用。宿主进程（Node 侧）把 `process.cwd()` 经注入的 `__PROTEUS_CODE_OPTS__` 交给浏览器面，因为只有它知道这个目录。

（诚实记录一次走错的弯路：我先尝试真的建一个「不登记任何项目」的会话，用 `sessions.create({cwd})` + `sessions.open()`。会话确实建成了、也正确归入「未分组」，但界面始终停在首屏——因为整个 UI 的工作区导航只认 `openWorkspace`。那条路被放弃了。）

**有意保留**：导航与设置里的通用概念文案（工作区 / 新会话 / 设置 / 模型 / 插件 / 权限）与多段落的「内测声明」保持原样。前者在任何同类产品里都是同一概念，改名只会让用户困惑；后者是复述「底层 harness 处于预览阶段」的诚实说明。如果你要连这些也换成自己的话术，告诉我即可。

### 液态玻璃（visual centerpiece）

界面主视觉是**液态玻璃**，不是普通毛玻璃。四件事共同构成它，缺一件就会塌回"灰色磨砂"：

| 要素 | 做法 | 为什么必需 |
|---|---|---|
| **可透射的光** | 全窗极光渐变网格（青 → 靛 → 紫 → 粉） | 玻璃叠在纯白/纯灰上是看不见的；背后必须有真实的亮度与色相 |
| **光学折射** | `backdrop-filter: … url(#proteus-glass-lens)`，SVG `feTurbulence` → `feGaussianBlur` → `feDisplacementMap` | 这是液态玻璃与 glassmorphism 的分界：背景被**折射**而不只是模糊 |
| **镜面高光** | 受光边 `inset 1.5px 1.5px 0 white/92%`，背光边压暗，外圈 1px 亮边 | 眼睛靠这道边读出"玻璃"而不是"色块" |
| **浮起** | 大偏移宽扩散的投影 | 让表面脱离背景 |

实现要点（都踩过坑）：

- **必须先清掉不透明底色**。客户端的合成器/侧边栏带着不透明的 token 填充（`--dsw-alias-bg-layer-1` = 纯白），半透明渐变叠在它上面等于没叠——背景完全透不过来。`background-color: transparent !important` 是这里的关键。
- **渐变层的顺序**。CSS 里 `background-image` 的第一层在最上。第一版把白色径向放在最上，结果把整个画面洗成灰色（实测彩度只有 20-30）；把色池放到最上、底色垫底后，四角彩度升到 45-107。
- **色池要够强（75-90%）**。因为它们会被毛玻璃压平；温和的着色模糊之后等于没有。
- **折射只给小浮层**（合成器、菜单、弹窗）。SVG `url()` 背景滤镜每次重绘都要重新过滤背景，所以**不给全高侧边栏**用，且只在 `@supports` 下启用。
- **`prefers-reduced-transparency` 时彻底移除模糊**（不只是去掉着色），`prefers-reduced-motion` 时关掉过渡。

瞄准点只用两类**稳定钩子**，因为客户端类名是按构建哈希的（`pI_x6G_sidebarCol`）：客户端刻意设置的语义属性（`[data-rightbar-collapsed]` 框架、`[data-composer-card]` 合成器、`[data-side]` 下拉、`[role=dialog]` 弹窗），以及 CSS-module 的**局部名后缀**（`[class*="_sidebarCol"]`）。每条规则都写成"钩子消失时表面退回原有 token 配色"，不会有任何表面变得不可用。

可用配置开关：

```yaml
- id: proteus-code
  name: '@proteus-code/dsh-proteus-code'
  config:
    enableLiquidGlass: false   # 关掉液态玻璃层，保留其余品牌改造
```

---

## 快速开始

### 环境要求

- **Node.js ≥ 22.19**（DSH 的引擎要求；Electron 自带的 Node 24 也满足）
- **pnpm ≥ 9**（作为 workspace 与 DSH profile 的包管理器）

> DSH 处于 developer preview，会有破坏性变更。本项目锁定 `@deepseek-ai/dsh@0.1.5-rc.1`。

### 安装

```bash
pnpm install
```

`pnpm install` 会做三件事：构建 workspace 内的包、下载 Electron、把 DSH 装到仓库根。首次安装会提示若干 native 构建脚本（`esbuild`、`electron`、`koffi`、`node-pty`、`protobufjs`、`@deepseek-ai/dsh-subprocess-local`）——这些已在 `pnpm-workspace.yaml` 的 `allowBuilds` 中放行。

### 启动桌面应用

```bash
pnpm start
```

这会构建 bundle 与 Electron 主进程，然后打开原生窗口。应用首次启动时会：

1. 在 `~/.proteus-code` 建立自己的 Harness home（**不会**动你已有的 `dsh` CLI 状态）
2. 写入一个 profile，其 bundle 栈为 `dsh-base` + `dsh-web-app` + `@proteus-code/dsh-proteus-code`
3. 用 `file:` 方式把本仓库构建出的 bundle 装进该 profile
4. 以 OS 随机分配的本地端口启动 harness，并加载进窗口

### 其他入口

```bash
pnpm web      # 无 GUI：起一个本地 Web 版 proteus-code 并打印 URL
pnpm test     # 单元测试（含插件注册、CLI 桥接）
pnpm smoke    # 构建 + 启动 + 截图到 .proteus-code/smoke.png（CI/无人值守自检）
pnpm verify   # 构建 + 测试 + typecheck + 真实 harness 启动一次
pnpm dev      # 开发模式：构建后直接跑 Electron（不重新下载）
```

### 指定 Proteus 工程

bundle 默认从会话工作目录向上寻找 Proteus 工程（识别 `proteus.config.ts` 或 `packages/cli/src/index.ts`）。也可以显式指定：

```yaml
# profile 的 cordis.patch.yml 里覆盖 plugin 行
- id: proteus-code
  name: '@proteus-code/dsh-proteus-code'
  config:
    proteusRoot: /path/to/proteus
    proteusCommand: node /path/to/proteus/packages/cli/src/index.ts
    timeoutMs: 180000
```

---

## 它给 agent 加了什么

### 1. 一组真正调用 `proteus` 的工具

| 工具 | 背后命令 | 用途 |
|---|---|---|
| `proteus_health` | `proteus health` | 工程健康检查，诊断入口 |
| `proteus_check` | `proteus check` | 全套门禁（CSS / 样式 / 路由 / CLI） |
| `proteus_build` | `proteus build --target <web\|skyline\|all>` | 分端构建，可选 Node/Rust 双编译校验 |
| `proteus_explain` | `proteus explain <file\|rule-id>` | 决策 trace / 单条规则 AI 说明书 |
| `proteus_rules` | `proteus rules [phase]` | 规则目录（按编译阶段） |
| `proteus_audit` | `proteus audit <module\|d2\|all\|coverage\|devtools-budget>` | 模块 / 设计系统 / 覆盖率审计 |
| `proteus_conformance` | `proteus conformance [--repo\|--backend\|--only]` | SPI conformance 套件 / 严禁 fork 扫描 |
| `proteus_migrate_mp` | `proteus migrate mp <path> [--dry-run]` | 小程序迁移 codemod |
| `proteus_policy` | 内置策略引擎（不执行命令） | 预测某条命令会被允许/询问/拒绝，以及是哪条规则 |
| `proteus_cli` | `proteus <任意已知子命令>` | 兜底出口（子命令白名单约束） |

工具优先通过 DSH 的 `ctx.shell` 服务执行，从而**继承会话的沙箱与审批策略**；当某 profile 没有 shell 服务时，回退到本地 `child_process`，工具依然可用。

### 1.5 执行策略（吸收自 Codex execpolicy）

`tools/pre-execute` 上挂了一个**前缀规则策略引擎**，设计直接取自 OpenAI Codex 的 `codex-execpolicy`：

```json
{
  "rules": [
    {
      "id": "no-recursive-root-delete",
      "pattern": ["rm", ["-rf", "-fr", "-r"]],
      "decision": "forbidden",
      "justification": "递归删除在本工作区被禁止；请删除具体路径。",
      "match": ["rm -rf build", ["rm", "-fr", "node_modules"]],
      "notMatch": ["rm file.txt"]
    },
    { "pattern": ["git", "push"], "decision": "prompt", "justification": "推送会发布内容，请确认远端与分支。" }
  ]
}
```

放到 `.proteus-code/policy.json`（向上自动发现），或写进插件 config 的 `policy` 字段。四条 Codex 语义都被保留：

1. **前缀规则 + 备选项** —— `["rm", ["-rf","-fr"]]` 用一条规则覆盖多个变体。
2. **规则自测** —— `match` / `notMatch` 是规则自带的单元测试，加载时校验；写错了在启动时**响亮失败**，而不是静默放行。上面那条错误策略会让 DSH 启动直接报错并指出是哪条规则。
3. **最严格者胜出** —— 多条规则命中时取 `allow < prompt < forbidden` 的最大值。
4. **host executable 钉死** —— `hostExecutables` 限定 basename 规则只能匹配声明的绝对路径，避免 `/tmp/evil/git` 继承 `git` 的规则。

两处对 Codex 的有意改动：规则用 JSON 而非 Starlark（宿主已经解析 YAML，再引入一个语言运行时是纯成本）；决策映射到 DSH 的 `allow/ask/deny`，且 **`allow` 是委托而非短路**——策略只能收紧，不能放宽，宿主的沙箱与审批策略永远仍然生效。

策略文件解析失败时**失败关闭**（deny 并把解析错误作为理由），不会静默无策略运行。

### 2. 斜杠命令（不经过模型）

`/proteus-health`、`/proteus-check`、`/proteus-build`、`/proteus-rules`、`/proteus-explain` —— 直接对工程执行 CLI，结果渲染在会话之外，不产生模型消息（与 DSH 自带 `/compact` 同一套机制）。

### 3. Proteus 开发技能

一个 runtime skill（`proteus-development`），在被选中时才注入模型上下文，内容涵盖：语义层优先、检查先于断言、规则优先于猜测、柔性布局原语、Capability Hook、迁移约定，以及**诚实边界**（哪些规划中的能力还不能声称可用）。

### 4. 人格与身份

`cordis.patch.yml` 里声明式设置 agent 人格；客户端 bundle 替换品牌标识。二者都不需要 fork DSH。

---

## 仓库结构

```
proteus-code/
├── packages/proteus-code-bundle/     # @proteus-code/dsh-proteus-code —— DSH bundle
│   ├── src/index.ts                  #   host 插件入口（tools / commands / skill / policy）
│   ├── src/tools.ts                  #   9 个 proteus_* CLI 工具
│   ├── src/policy.ts                 #   策略引擎（前缀规则 / 自测 / host-executable 钉死）
│   ├── src/policy-runtime.ts         #   策略解析与缓存（守卫与工具共用，保证一致）
│   ├── src/policy-guard.ts           #   tools/pre-execute 守卫（只收紧不放宽）
│   ├── src/policy-tool.ts            #   proteus_policy 自省工具
│   ├── src/commands.ts               #   5 个斜杠命令
│   ├── src/skill.ts                  #   proteus-development 技能
│   ├── src/proteus-cli.ts            #   CLI 定位 / 沙箱执行 / 回退
│   ├── src/client/index.tsx          #   浏览器面：品牌 slot 组件
│   ├── src/dsh-host.d.ts             #   DSH 宿主 API 的环境声明
│   └── cordis.patch.yml              #   bundle 层：人格 + 禁用官方品牌 + 插入插件
├── apps/desktop/                     # @proteus-code/desktop —— Electron 壳
│   ├── src/main.ts                   #   主进程：起 harness、开窗口、smoke 模式
│   ├── src/cli.ts                    #   无 GUI 入口（pnpm web）
│   └── src/profile.ts                #   Harness home、profile、Node ≥22 解析
├── scripts/build-bundle.mjs          # host + client 双面构建
├── scripts/verify.mjs                # 端到端验证
└── docs/
    ├── architecture.md               # 架构与扩展点
    └── getting-started.md            # 面向使用者的上手与排错
```

---

## 为什么这样接（而不是 fork DSH）

DSH 的设计是「没有需要打补丁的特权内核」。扩展它的正规方式是**挂一个插件在其它插件旁边**，而不是修改它。因此 proteus-code：

- **不做 DSH fork**，不复制它的源码
- 以一个 **out-of-tree bundle**（`dsh.bundle.patch`）的形式贡献：人格行 + 插件行
- 以 `dsh.client` **双面包**的形式贡献浏览器面（host + client 同一包两入口）

这条路线的好处是：DSH 升级时 proteus-code 只需要跟随它的 SPI，而不是合并 fork。

## 吸收的 Codex 设计

执行策略模块（`src/policy*.ts`）是 OpenAI Codex（`codex-execpolicy`）设计的移植，同样是**兼容层而非 fork**：保留语义，适配本宿主的扩展点。

| Codex 设计 | 在 proteus-code 的落点 | 为什么值得吸收 |
|---|---|---|
| **前缀规则 + 备选 token** —— `pattern: ["rm", ["-rf","-fr"]]` | `src/policy.ts` 的 `matchesPrefix` | 一条规则覆盖命令变体；策略可读、可审 |
| **规则自测** —— `match` / `notMatch` 在加载时校验 | `parsePolicy` 加载即校验 | 写错的规则在启动时响亮失败，绝不静默放行 |
| **最严格者胜出** —— 命中多条时取 `max(allow, prompt, forbidden)` | `evaluatePolicy` 的 `RESTRICTIVENESS` 排序 | 规则可以放心叠加，安全性单调 |
| **host executable 钉死** —— basename 规则限定绝对路径 | `hostExecutables` + `resolveHostExecutables` | `/tmp/evil/git` 无法继承 `git` 的规则，防同名劫持 |
| **`execpolicy check`** —— 不执行就能问「会怎样」 | `proteus_policy` 工具 | 模型能自查，人能审计拦截原因 |

有意不同的两点，以及原因：

1. **规则是数据不是 Starlark。** 宿主已经解析 YAML 配置，为策略再引入一个语言运行时是纯成本，而上面四条语义才是价值所在。
2. **`allow` 是委托而不是短路。** Codex 独占执行路径，可以自己决定 allow；这里策略挂在 DSH 的瀑布上，`allow` 必须 `next()` 让宿主的沙箱与审批继续跑。结果是这个策略**只能收紧、不能放宽**——插件不该有权绕过宿主的安全策略。

策略解析失败时**失败关闭**：deny 并把解析错误作为理由，而不是静默无策略运行。

> 未吸收（有意）：Codex 的 Rust 沙箱机制（Seatbelt / Landlock / bwrap）。DSH 已有自己的 `dsh-sandbox` / `dsh-bash-sandbox` provider 承担这一层；重复实现既无必要，也会和宿主的沙箱语义冲突。

## 已知限制（诚实边界）

- DSH 处于 **developer preview**，会破坏性变更；本项目锁定 `0.1.5-rc.1`。
- 发布版 DSH 的 peer 依赖图目前**不自洽**（部分 peer 目标如 `@deepseek-ai/dsh-type-meta` 未发布），所以 bundle 不能把 DSH 包声明为 `dependencies`/`peerDependencies`。本项目因此把 DSH 包放进 `devDependencies`，运行时 import 由 harness 自身的模块回退解析，类型检查走 `src/dsh-host.d.ts` 的本地声明。等上游发布自洽依赖图后，可换回真实 devDependencies。
- 桌面应用目前是**开发态启动**（Electron 直接跑构建产物），尚未做签名、打包、自动更新。DSH 官方桌面应用是签名 Electron 壳，本项目对齐它的启动模型但未做发布工程。
- 品牌替换依赖 DSH 客户端的 slot 名（`sidebar.brand.*`）与官方品牌行 id（`ui-brand-official`）；DSH 改名时需要同步。
- Proteus 侧同样是诚实边界：Web + 微信小程序（Skyline）可用；NativeBackend、宿主运行时、执行载体（AOT）、全终端为规划中，agent 技能里已明确要求不得声称其可用。

## 协议

Apache-2.0。DSH 为 MIT，Proteus 为 Apache-2.0。
