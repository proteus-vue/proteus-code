# 架构

proteus-code 是「DSH 插件 + Electron 壳」，不是 DSH 的 fork。本文说明每个部件为什么这样接，以及它依赖哪些上游契约。

## 全景

```
┌─ Electron 主进程 (apps/desktop/src/main.ts) ─────────────────────────────┐
│  whenReady → ensureProfile() → spawn(dsh --profile proteus-code)         │
│            → 等 harness 打印 URL → BrowserWindow.loadURL(loopback)       │
└──────────────────────────────────────────────────────────────────────────┘
                                  │ spawn
                                  ▼
┌─ DSH profile: proteus-code (位于 ~/.proteus-code) ───────────────────────┐
│  bundle 栈（层序即优先级，后层覆盖前层）:                                  │
│    1. @deepseek-ai/dsh-base        核心：agent loop / session / tools…   │
│    2. @deepseek-ai/dsh-web-app     浏览器面：Web UI / workspace / …      │
│    3. @proteus-code/dsh-proteus-code   ← 本项目                          │
│   + profile 自己的 cordis.patch.yml（用户覆盖层，优先级最高）             │
└──────────────────────────────────────────────────────────────────────────┘
                                  │ 装载
                                  ▼
┌─ @proteus-code/dsh-proteus-code ─────────────────────────────────────────┐
│  cordis.patch.yml（bundle 层）                                            │
│    • system-prompt.personaPrefix = proteus-code 人格                      │
│    • ui-brand-official: disabled                                          │
│    • insert: protect-code 插件行                                          │
│                                                                          │
│  host 面 dist/index.js                                                    │
│    • ctx.tools.register × 9      → proteus_* 工具                         │
│    • ctx.inject(['commands'])    → /proteus-* 斜杠命令                    │
│    • ctx.inject(['skills'])      → proteus-development 技能               │
│                                                                          │
│  client 面 dist/client.js（dsh.client 双面包）                             │
│    • slots.inject('sidebar.brand.mark' / 'sidebar.brand.name')            │
│    • slots.inject('conversation.hero.brand.mark')                         │
└──────────────────────────────────────────────────────────────────────────┘
                                  │ ctx.shell（继承沙箱/审批）
                                  ▼
                          proteus CLI（真实命令）
```

## 为什么是 bundle，不是 fork

DSH 的自我描述是「没有需要打补丁的特权内核：你通过在旁边挂一个插件来扩展它，注册是 effect，插件卸载时自动回卷」。它的文档 `packages/bundle/README.md` 明确了扩展路径：

- **bundle** = 一个 npm 包，声明 `dsh.bundle.patch`，回答「这个包贡献什么」
- **profile** = `$DSH_HOME/profiles/<name>` 下的目录，声明 `dsh.profile.bundles`，回答「按什么顺序组合」

一个 bundle 层是一个 patch 文件（YAML 数组）：按 `id` 覆盖已有行，或 `insert` 新行。层序决定优先级，profile 自己的 `cordis.patch.yml` 永远最后应用——所以用户能覆盖 proteus-code 的任何一行。

proteus-code 的 `cordis.patch.yml` 因此只有三件事：改人格、禁官方品牌、插自己的插件。**没有一个被 fork 的文件。**

## 双面包：host 与 client

DSH 的浏览器插件机制是：宿主 Loader 扫描已装载条目里声明了 `dsh.client` 的包，把它们组成 `window.__DSH_BOOT__`，浏览器再用 `window.__ModuleLoader__.load({ id, factory })` 逐个装载。

于是同一个包提供两个入口：

| 入口 | 目标 | 格式 | 外部依赖 | 用途 |
|---|---|---|---|---|
| `dist/index.js` | Node（宿主） | ESM | `@deepseek-ai/*` | 注册工具 / 命令 / 技能 |
| `dist/client.js` | 浏览器（渲染器） | CJS + 自定义包裹 | `react`、`@deepseek-ai/*` | 注册品牌 slot 组件 |

`package.json` 里 `dsh.client.inject` 声明客户端依赖的服务（`@deepseek-ai/dsh-client-ui-renderer` 提供 slot 注册表）。slot 是 `kind: "single"`，所以**必须**先禁用官方品牌行 `ui-brand-official`，否则同一 slot 有两个注册者。

## 依赖解析：本项目最不显然的一点

这是踩过的坑，也是最需要被后人理解的约束。

DSH 的 profile 用 pnpm 的 `hoisted` linker。DSH 启动时会把安装依赖闭包里的包**heal 一个 symlink** 到 `$DSH_HOME/profiles/node_modules`，作为裸插件名解析的兜底。

于是三方 bundle 的安装方式决定了它的 import 能否解析：

| 安装方式 | 落点 | bundle 的 import 解析自 | 结果 |
|---|---|---|---|
| `dsh plugin add <dir>` | profile 内 symlink → 仓库 | 仓库（pnpm 隔离布局，DSH 包不提升到顶层） | ✗ 找不到 `@deepseek-ai/*` |
| `dsh plugin add file:<dir>` | profile 的 `.pnpm` 实体副本 | profile → 向上走到 healed `profiles/node_modules` | ✓ |
| `dsh plugin add <tarball>` | profile 的 `.pnpm` 实体副本 | 同上 | ✓ |
| `--patch` overlay（绝对路径） | 直接 import 该文件 | 仓库根 `node_modules` | ✓（本项目早期验证用） |

`apps/desktop/src/profile.ts` 因此使用 `file:` 前缀安装，并带一个基于 mtime 的刷新判断：bundle 重新构建后，先 `remove` 再 `add`，保证 profile 拿到的是最新副本。

### 为什么 DSH 包只能进 devDependencies

发布到 npm 的 `@deepseek-ai/dsh-tools` 的 peer 依赖里包含 `@deepseek-ai/dsh-type-meta`，而这个包**没有发布**。任何把 DSH 包声明为 `dependencies` 或 `peerDependencies` 的 bundle，在 `pnpm install` 时都会因解析该 peer 而失败（实测：`ERR_PNPM_FETCH_404 … dsh-type-meta`）。

所以本项目的做法是：

- **运行时**：`import` 由 harness 的模块回退解析（bundle 里 `@deepseek-ai/*` 保持 external，绝不打包——否则会产生第二份 Cordis 服务实例，破坏单例语义）
- **类型**：`src/dsh-host.d.ts` 用 `declare module` 声明本项目实际用到的宿主 API
- **构建期**：`devDependencies` 里不放 DSH 包

等上游发布自洽依赖图后，`dsh-host.d.ts` 可以删除，改回真实 devDependencies。

## 执行策略

`proteus-cli.ts` 是本项目唯一与「执行」相关的模块，它区分两件事：

1. **定位**：`findProteusRoot()` 从会话 cwd 向上找 `proteus.config.ts` 或 `packages/cli/src/index.ts`；`resolveProteusArgv()` 依次尝试 `node_modules/.bin/proteus` → `tsx packages/cli/src/index.ts` → 构建产物。
2. **执行**：优先 `ctx.shell`（provider 是 `dsh-bash-sandbox`，因此命令受会话沙箱与审批策略约束）；无 shell 服务时回退 `child_process.execFile`。

参数一律经 `shellQuote()` 转义后拼成命令串交给 shell；`proteus_cli` 兜底工具用 `ALLOWED_SUBCOMMANDS` 白名单限制子命令，避免把任意命令透传给沙箱。

## 启动时序（桌面）

```
app.whenReady
  └─ startHarness
       ├─ resolvePaths()        # repoRoot / dshHome / bundleDir / dshEntry
       ├─ ensureProfile()       # 建 home + profile + pnpm-workspace + file: 安装
       ├─ harnessLaunch()       # 解析 Node ≥22（env 覆盖 → nvm → Electron 自带 Node）
       └─ spawn + waitForUrl()  # 正则匹配 "dsh web: http://…"
  └─ createWindow(url)          # loopback origin 白名单，外链走系统浏览器
  └─ （可选）runSmoke()         # PROTEUS_CODE_SMOKE=<png> 时截图并退出
```

Node ≥22 的解析是必要的：DSH 要求 `^22.19.0 || >=24`，而系统 `node` 可能是 18。解析顺序为 `PROTEUS_CODE_NODE` → `~/.nvm/versions/node/v≥22` → Electron 自带 Node（`ELECTRON_RUN_AS_NODE=1`，v24.20）。最后一条让应用在没有 nvm 的机器上也能自洽运行。

## 界面定制面（DSH 客户端）

改界面有且只有四个受支持的入口，本项目全部使用，没有触碰上游文件：

```
┌─ index.html（宿主侧，服务时注入）──────────────────────────────────────┐
│  webServer.tapIndex()  ← 唯一能改 <title> / favicon / 全局 CSS 的面     │
│    src/brand-assets.ts: applyBrandToIndex()                            │
│    src/client/liquid-glass.ts: LIQUID_GLASS_CSS + LENS_SVG             │
└────────────────────────────────────────────────────────────────────────┘

┌─ 客户端（浏览器，插件装载后）──────────────────────────────────────────┐
│  ctx.theme.overrideTokens(source, {token: {light, dark}})               │
│    → 79 个 --dsw-alias-* token，明暗两套；整站配色                      │
│    src/client/palette.ts                                               │
│                                                                        │
│  ctx.slots.register('sidebar.brand.mark' | 'sidebar.brand.name'         │
│                    | 'conversation.hero.brand.mark')                    │
│    → 品牌图形与字标；需先 disabled 官方行 ui-brand-official             │
│    src/client/index.tsx                                                │
│                                                                        │
│  document.title 访问器遮蔽 + MutationObserver 兜底                       │
│    → 标题；layout 每次会话切换都会重写它                                 │
│                                                                        │
│  按精确文本替换（仅当 locale 命名空间不可重新注册时）                     │
│    → src/client/copy-overrides.ts                                      │
└────────────────────────────────────────────────────────────────────────┘
```

### 为什么文案覆盖是最后手段

`LocaleRuntime.register(ns, locale, dict)` 对**同一 ns + locale 重复注册直接抛错**，而归属插件在自己的 `apply` 里先注册。因此 `conversation`、`settings.models` 等命名空间的文案无法通过受支持的方式覆盖。

两类文案因此被区别对待：

| 情形 | 处理 |
|---|---|
| 品牌性质的短语（首屏标语） | 精确文本替换，可逆 + 观察者覆盖后渲染副本 |
| 通用概念词（工作区/新会话/设置/模型/权限） | **不改**：任何同类产品都是同一概念，改名降低可理解性 |
| 多段落说明（内测声明） | **不改**：替换 250 字长段落需精确匹配，脆弱且收益低 |

覆盖清单在 `tests/copy-overrides.test.ts` 中被锁定为恰好一条，防止这一层无声扩大。

### 液态玻璃的实现与实测约束

玻璃层是一张注入的样式表加一段隐藏 SVG。它为什么长这样，全部来自实测而非审美偏好：

| 实测发现 | 后果 |
|---|---|
| 客户端类名按构建哈希（`pI_x6G_sidebarCol`） | 只能瞄准语义属性与 CSS-module 局部名后缀，不能写字面类名 |
| 表面带不透明 token 填充（`bg-layer-1` = 白） | 必须先 `background-color: transparent !important`，否则背景透不过来，玻璃失效 |
| `background-image` 第一层在最上 | 第一版把白色径向放最上，整屏被洗成灰（彩度 20-30）；色池置顶后到 45-107 |
| 毛玻璃会压平色彩 | 色池强度必须到 75-90%，温和着色模糊后等于没有 |
| `backdrop-filter: url(#svg)` 可用 | 真实折射可实现，但每次重绘都要重新过滤背景 |
| 全高元素上跑 SVG 滤镜代价高 | 折射只给合成器/菜单/弹窗，侧边栏只用 blur |

因此：折射在 `@supports` 内、只作用于小浮层；`prefers-reduced-transparency` 时移除 blur 本身；每条规则都保证钩子消失时退回原有 token 配色。

`apps/desktop/src/probe.ts` 是配套的诊断面：`PROTEUS_CODE_PROBE` 读回计算后的 token 值，`PROTEUS_CODE_DUMP` 导出实际绘制表面的结构与稳定属性，`PROTEUS_CODE_QUERY` 执行一段只读脚本回答临时问题。改 token 或玻璃的调试都应该先看这些读数——截图看不出"某个 token 没生效"或"滤镜被不透明底盖住"这类问题。

### 配色如何做到一致

配色不改任何组件，只改 token。客户端组件只读 `--dsw-alias-*`，从不写字面色值，所以一次 token 覆盖即整站生效。`palette.ts` 覆盖的族包括：表面（base/layer-1..3/module-platform/overlay/mask）、文字（primary..dimmed/inverted）、边界（l1..l4）、按钮（primary/contrast/elevated/floating/toolbar/ghost）、交互态（hover/active/danger）、Markdown 与代码块、滚动条、状态色（success/warn/error/business）、浮层（tooltip/toast）。明暗两套同时给出，跟随系统的偏好也能正确着色。

### 启动页（`[data-dsh-boot]`）

启动加载器由前端入口 JS 动态创建，其文案 `HARNESS` / `Loading plugins…` 是入口里的字面量，且节点带哈希类名。品牌样式表以 `[data-dsh-boot]` 数据属性为锚点、取其第一个文本子元素做 `::after` 内容替换，并给 spinner 上品牌色。**若上游改变该结构，最坏结果是启动页维持原样**，不会影响应用本体。

## 执行策略（Codex execpolicy 的适配）

策略模块是 OpenAI Codex `codex-execpolicy` 的移植。它不是 fork——保留语义，换宿主扩展点。

```
                    ┌─ tools/pre-execute (waterfall) ─────────────────┐
bash / pwsh ───────▶│  policy-guard: 命中 forbidden → deny            │
proteus_cli ───────▶│                命中 prompt    → ask             │
其他工具 ──────────▶│                其余(含 allow) → next() 交给宿主 │
                    └────────────────────────────────────────────────┘
                                     │ 共用同一个 PolicyRuntime
                                     ▼
                     policy-runtime（解析 + 按 mtime 缓存）
                        ├─ 插件 config 的 policy（加载时校验）
                        └─ .proteus-code/policy.json（向上发现）
                                     │
                                     ▼
                             policy.ts（纯函数引擎）
```

### 为什么 runtime 是独立一层

守卫和 `proteus_policy` 工具**必须给出同一个答案**。如果各自解析，工具可能报「允许」而守卫实际「拒绝」——那比没有工具更糟。所以解析、缓存、合并都收在 `PolicyRuntime` 一处，两边都从它读。

### 语义保持（对照 Codex）

| 语义 | Codex 实现 | 本项目实现 |
|---|---|---|
| 前缀匹配 + 备选 token | `PrefixPattern::matches_prefix` | `matchesPrefix` |
| 首 token 必须精确、优先精确匹配 | `match_exact_rules` 先于 basename 回退 | `evaluatePolicy` 先精确，后回退 |
| 最严格者胜出 | `Evaluation::from_matches` 取 `max` | 按 `RESTRICTIVENESS` 排序取首个 |
| host executable 钉死 | `host_executable(name, paths)` | `hostExecutables` + `resolveHostExecutables` |
| 规则自测 | 加载期校验 `match`/`not_match` | `parsePolicy` 加载期校验 |

### 与 Codex 的两处有意偏离

1. **数据而非 Starlark。** Codex 用 Starlark 表达规则。这里用 JSON/YAML 数据：宿主已经在解析配置，引入一个语言运行时是纯成本，而上面那些语义才是价值。
2. **`allow` 委托而非短路。** 这是最关键的一处。Codex 独占执行路径，可以自行决定放行；本项目挂在 DSH 的瀑布上，`allow` 必须 `next()`，让 `dsh-sandbox-policy` / `dsh-user-approval` 等监听器继续跑。结果是策略**只能收紧**。这是刻意的权限边界：插件不该能放宽宿主的安全策略。

### 失败模式

| 情况 | 行为 | 原因 |
|---|---|---|
| config 里的 policy 格式错误 | 插件加载失败 | 遵循宿主「配置错误应响亮失败」的规范 |
| 发现到但无法解析的策略文件 | **失败关闭**：deny 并附带解析错误 | 宁可拒绝也不静默无策略运行 |
| 策略文件被编辑 | 按 mtime 重新解析 | 改规则不需要重启 |
| 无规则 | 完全委托给宿主 | 未配置策略时不引入行为 |

## 测试策略

| 层 | 文件 | 覆盖 |
|---|---|---|
| 插件注册 | `tests/plugin.test.ts` | 工具/命令/技能全部注册；开关项生效；子命令白名单；策略守卫已挂载；品牌注入已挂载 |
| 工具→argv | `tests/tools.test.ts` | 每个工具的精确命令行（含引号、开关取反） |
| CLI 桥接 | `tests/proteus-cli.test.ts` | 引号转义、工程发现、argv 解析、runner 选择 |
| 策略引擎 | `tests/policy.test.ts` | 分词、前缀匹配、自测规则、最严格者胜出、host-executable 钉死、文件发现与热更新、失败关闭、守卫 deny/ask/allow、工具与守卫一致性 |
| 品牌 | `tests/brand.test.ts` | 79+11 token 明暗完整性、favicon 自包含、index 重写（标题/图标/样式注入/幂等/缺 head 容错）、玻璃层（滤镜定义先于引用、位移量克制、环境光存在、镜面高光、只瞄准稳定钩子、@supports 与无障碍降级、可独立关闭） |
| 文案覆盖 | `tests/copy-overrides.test.ts` | 中英双语替换、无关文案不动、可逆还原、后渲染副本覆盖、dispose 后不再改写、清单锁定为 1 条 |
| 真实集成 | `tests/integration.test.ts` | 对真实 Proteus 工程跑 `rules` / `health`（`PROTEUS_CODE_TEST_CHECKOUT` 门控） |
| 端到端 | `scripts/verify.mjs` + `pnpm smoke` | 构建 + 测试 + 真实 harness 启动 / 截图 |

测试把 `@deepseek-ai/*` alias 到 `tests/stubs/`，使单测不需要 harness 在场。

## 上游契约清单（DSH 改版时需同步）

| 契约 | 本项目依赖点 |
|---|---|
| plugin 形态：导出 `name` / `inject` / `apply(ctx, config)` | `src/index.ts` |
| `ctx.tools.register(definition)` 接受纯定义对象 | `src/tools.ts` |
| `ctx.inject([...], cb)` 门控服务 | `src/index.ts` |
| `ctx.commands.register({ name, description, input, handler })` | `src/commands.ts` |
| `ctx.skills.register({ name, description, whenToUse, source, content })` | `src/skill.ts` |
| `ctx.shell.resolve()/run()` 的请求与结果形状 | `src/proteus-cli.ts` |
| `system-prompt.personaPrefix` 配置字段 | `cordis.patch.yml` |
| UI slot 名 `sidebar.brand.mark` / `sidebar.brand.name` / `conversation.hero.brand.mark` | `src/client/index.tsx` |
| 官方品牌行 id `ui-brand-official` | `cordis.patch.yml` |
| `dsh.client` 声明与 `window.__ModuleLoader__.load` 包裹格式 | `package.json` / `build-bundle.mjs` |
| profile 的 `pnpm-workspace.yaml` 内容 | `src/profile.ts` |
| `ctx.theme.overrideTokens(source, {light,dark})` | `src/client/palette.ts` |
| `ctx.webServer.tapIndex(fn)` | `src/brand-assets.ts` |
| 主题暗色属性 `data-ds-dark-theme`、启动属性 `data-dsh-boot` | `src/client/palette.ts` |
| 稳定 DOM 钩子：`[data-rightbar-collapsed]` / `[data-composer-card]` / `[data-side]` / `[data-phase]` / `[class*="_sidebarCol"]` | `src/client/liquid-glass.ts` |
| `backdrop-filter: url(#svg)` 支持与代价 | `src/client/liquid-glass.ts` |
| locale 不可重复注册（文案覆盖的前提） | `src/client/copy-overrides.ts` |

## 品牌的两个层面

品牌替换要处理两处，因为它们的来源不同：

1. **侧边栏 / 欢迎页的图形与文字** —— 来自 UI slot。官方品牌插件（`@deepseek-ai/dsh-client-ui-brand-official`）注册了 `sidebar.brand.mark` / `sidebar.brand.name`，而这两个 slot 是 `kind: "single"`，所以 patch 层先 `disabled: true` 禁用它，再由本项目的客户端面填充。
2. **窗口标题 / `document.title`** —— layout 插件里 `productTitle` 是**编译期硬编码**的 `"DeepSeek Harness"`，并在每次会话/面板变化时写 `document.title`。因此：

   - Electron 主进程拦截 `page-title-updated` 并把窗口标题钉在 `proteus-code`（用户可见的那个标题）
   - 客户端插件用 `Object.defineProperty` 遮蔽 `document.title` 访问器，忽略上游写入；无法重定义时回退到 `MutationObserver`

这两点都不需要 fork 上游 UI。
 |
