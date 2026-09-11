# PROJECT_MEMORY.md — proteus-code 项目记忆

> 交接文档。记录**为什么这样做**、**踩过什么坑**、**哪些路走不通**。
> 代码结构看 `README.md`，架构推理看 `docs/architecture.md`。这里只写那两处没有的：决策依据、失败路径、此后必须遵守的事实。

---

## 1. 项目定位

把 Proteus（跨端框架）接入 DSH（DeepSeek Harness），做成 Electron 桌面 AI 编程应用。

**核心决定：不 fork DSH。** DSH 的自我描述是「没有需要打补丁的特权内核，扩展方式是挂插件在旁边」。因此本项目：

- 以 **out-of-tree bundle**（`dsh.bundle.patch`）贡献能力
- 以 **`dsh.client` 双面包**贡献浏览器面
- 以 **`--patch` overlay / profile** 完成组装

**结论**：DSH 升级时只需跟随它的 SPI，无需合并 fork。这是本仓库一切设计的前提，不要为了省事去改 `node_modules` 或复制 DSH 源码。

---

## 2. DSH 扩展点清单（已实证可用）

| 扩展点 | 用途 | 本项目落点 |
|---|---|---|
| `ctx.tools.register(definition)` | 注册模型工具 | `src/tools.ts`（9 个 CLI 工具） |
| `ctx.inject([...], cb)` | 等服务就绪后注册 | 命令 / 技能 / 主题 / 选择器 |
| `ctx.commands.register()` | 斜杠命令（不过模型） | `src/commands.ts` |
| `ctx.skills.register()` | runtime 技能 | `src/skill.ts` |
| `ctx.on('tools/pre-execute', …)` | 工具调度瀑布（allow/ask/deny） | `src/policy-guard.ts` |
| `ctx.shell` | 命令执行（继承沙箱与审批） | `src/proteus-cli.ts` |
| `ctx.theme.overrideTokens(src, {light,dark})` | **整站换肤**（token 层） | `src/client/palette.ts` |
| UI slot `sidebar.brand.*` / `conversation.hero.brand.mark` | 品牌标识 | `src/client/index.tsx` |
| UI slot `conversation.hero.workspace`（**优先级 -1 可遮蔽**） | 工作区选择器 | `src/client/workspace-picker.tsx` |
| `ctx.webServer.tapIndex(fn)` | 改写 `index.html`（title/favicon/全局 CSS） | `src/brand-assets.ts` |
| `system-prompt.personaPrefix` | 声明式注入人格 | `cordis.patch.yml` |

**关键机制**：single 类插槽的争用由**优先级决定，最低者渲染**。框架的报错信息会直接告诉你这条路径（`register at a different priority to shadow it (lowest renders)`）——所以遮蔽上游 UI 不需要禁用它的行。

---

## 3. 三个必须遵守的硬约束

### 3.1 DSH 包只能进 `devDependencies`

**为什么**：发布版 `@deepseek-ai/dsh-tools` 的 peer 图**不自洽**——它引用未发布的 `@deepseek-ai/dsh-type-meta`。任何把 DSH 包声明为 `dependencies`/`peerDependencies` 的包，`pnpm install` 都会因解析该 peer 失败（`ERR_PNPM_FETCH_404 … dsh-type-meta`）。

**做法**：运行时 import 保持 external（由 harness 的模块回退解析），类型走 `src/dsh-host.d.ts` 的本地 `declare module`。

**注意**：**绝不能把 `@deepseek-ai/*` 打进 bundle**——那会产生第二份 Cordis 服务实例，破坏单例语义。

### 3.2 bundle 必须用 `file:` 安装

**为什么**：pnpm 对 `link:` / 裸路径会装成 **symlink**，插件于是从**仓库**解析 import，而仓库是 pnpm 隔离布局、DSH 包不在顶层 → 找不到 `@deepseek-ai/*`。`file:` 会实体复制到 profile 的 `.pnpm`，从而能向上走到 harness 的 `profiles/node_modules` 回退。

真值表：

| 安装方式 | 插件 import 解析自 | 结果 |
|---|---|---|
| `add <dir>`（裸路径） | 仓库 | ✗ |
| `add file:<dir>` | profile → harness 回退 | ✓ |
| `add <tarball>` | profile → harness 回退 | ✓ |
| `--patch` overlay（绝对路径） | 仓库根 node_modules | ✓（早期验证用） |

### 3.3 模板字符串里的 CSS 注释不能含反引号

**踩了两次**。`liquid-glass.ts` 的 CSS 放在模板字符串里，注释中写 `` `*` `` 或 `` `background-image` `` 会**提前闭合模板字符串**，报错信息指向无关行（`Expected ";" but found "background"`）。写 CSS 注释时不要用反引号包词，用普通引号或直接写。

---

## 4. 两个被证伪的方案（不要再试）

### 4.1 「真正不登记任何项目」的会话 —— 走不通

**尝试**：`sessions.create({ cwd })` + `sessions.open(id)`，不登记任何 workspace。

**结果**：会话**确实建成了**、也正确归入侧边栏的「未分组」，但**界面始终停在首屏、输入框禁用**。

**根因（读源码得出）**：这套 UI 里**不存在可用的「无工作区」状态**。输入框的启用条件依赖 hero 的 workspace chip 有标题，而标题只来自工作区；DSH 原生「新会话」按钮在没有工作区时也只是 `sessions.clear()` + 回到选择器。工作区导航只认 `openWorkspace()`。

**最终方案**：「不在项目中工作」的语义是「**不用你手动挑目录**」——把 harness 工作目录自动登记为项目（`workspaces.create({path})`，幂等），再走与其它行完全相同的 `onPick` 路径。宿主经 `__PROTEUS_CODE_OPTS__.defaultCwd` 把 `process.cwd()` 交给浏览器面，因为只有 Node 侧知道这个目录。

### 4.2 侧边栏打 `backdrop-filter` —— 走不通

**尝试**：给侧边栏本身加毛玻璃。

**结果**：两个连环回归——设置弹窗被压进 280px 侧边栏宽度；改用伪元素 + `isolation` 后弹窗被中间栏盖住，再改又让侧边栏内容消失。

**根因**：
1. **`backdrop-filter` 会为 `position: fixed` 后代创建包含块**，而设置弹窗的 DOM **位于侧边栏子树内** → 弹窗被约束在侧边栏里。
2. **`isolation: isolate` 会创建层叠上下文** → 困住弹窗的 fixed 覆盖层。

**最终方案**：侧边栏**不打滤镜**，改为半透明着色（`color-mix` 混 `--dsw-specific-sidebar-fill`）——这也正是苹果侧边栏的做法（接近不透明的材质，不是透明玻璃）。模糊与 SVG 折射只保留在**不含 fixed 后代**的合成器与浮层上。

**判据**：`backdrop-filter` 只能加在「确定没有 `position: fixed` 后代」的元素上；否则必须移到伪元素（伪元素无后代，永不成为包含块）。

---

## 5. 液态玻璃：实现要点

四要素缺一不可，否则退化成"灰磨砂"：

1. **可透射的光** —— 背景必须有真实亮度与色相，否则玻璃不可见
2. **光学折射** —— `backdrop-filter: … url(#svg)`，`feTurbulence`→`feGaussianBlur`→`feDisplacementMap`（scale 8，克制）
3. **镜面高光** —— `inset 0 1px 0 white`+`inset 0 0 0 1px` 的边（用 inset 而非 border，避免改布局）
4. **浮起** —— 大偏移宽扩散的低透明度投影

**踩过的坑**：
- **必须先清掉不透明底色**。客户端表面带不透明 token 填充（`bg-layer-1` = 白），半透明渐变叠在它上面等于没叠。
- **`background-image` 第一层在最上**。初版把白色径向放最上，整屏被洗成灰（实测彩度仅 20-30）；色池置顶后到 45-107。
- **色池强度要够**（当时 75-90%），因为毛玻璃会压平色彩。**注意**：后期改为苹果式克制配色后已大幅调低——这是有意的风格变更，不是回退。
- **清容器底色不要用通配符**。`*:not(button)` 会误伤浮层里的 inline 元素（每个字出现底色块），还会弄没侧边栏内容。只点名具体的块级容器。

---

## 6. 调试工具（先用这个，别反复看图）

`apps/desktop/src/probe.ts`，三个环境变量：

| 变量 | 作用 |
|---|---|
| `PROTEUS_CODE_PROBE=1` | 读回**计算后**的 token 值与控件状态 |
| `PROTEUS_CODE_DUMP=<path>` | 导出实际绘制表面的结构、稳定属性、CSS 能力 |
| `PROTEUS_CODE_QUERY=<js>` | 执行一段只读脚本并回报结果（最后表达式为返回值） |
| `PROTEUS_CODE_QUERY_SETTLE_MS` | query 后静置，让截图反映 query 后的状态 |
| `PROTEUS_CODE_DARK=1` | 强制暗色（仅够探测/截图，会被 theme 插件改回） |
| `PROTEUS_CODE_SMOKE=<png>` | 加载后截图并退出（自检用） |

**为什么需要它**：整套 UI 靠 token 换肤，**截图看不出"某个 token 没生效"或"滤镜被不透明底盖住"**——必须读计算值。

**暗色真实验证**：往 Harness home 的 `settings.yaml` 写 `ui-theme: {preference: dark}`。注意命名空间是 **`ui-theme`** 而非 `settings.theme`（后者会被应用改写）。

---

## 7. UI 定制的三层正当性

改动按此排序，**能走上一层就不要用下一层**：

1. **声明式 / token 层**（最优）：`--dsw-alias-*` + `--dsw-specific-*` token、UI slot。完全受支持。
2. **注入层**：改 `index.html`（favicon/标题/全局 CSS）。DSH 的结构化 `IndexInjection` 无 `title`/`link` 类型，故用它文档明确保留的 `webServer.tapIndex`。
3. **文案覆盖层**（兜底）：locale 命名空间**不可重新注册**（`LocaleRuntime.register` 对同 ns+locale 重复注册直接抛错，归属插件先注册），只能在渲染后按**精确文本**替换。清单锁定为 1 条，有测试守着。

**稳定瞄准点**：客户端类名按构建哈希（`pI_x6G_sidebarCol`）**不可用**。只能用语义属性（`[data-rightbar-collapsed]`、`[data-composer-card]`、`[data-side]`、`[role=dialog]`）与 CSS-module 的**局部名后缀**（`[class*="_sidebarCol"]`）。每条规则都要保证「钩子消失时退回原有 token 配色」。

---

## 8. 环境与启动链

- **Node ≥22.19 必需**（DSH 引擎要求）。解析顺序：`PROTEUS_CODE_NODE` → nvm v≥22 → **Electron 自带 Node 24**（所以没装 nvm 的机器也能跑）。
- **Harness home 默认 `~/.proteus-code`**，与用户的 `dsh` CLI 状态完全隔离。
- 桌面应用用**随机端口**起 harness，只加载 loopback origin。
- **Electron 44 的 main 必须打成 ESM**（包是 `type: module`；打成 CJS 会报 `require is not defined`）。

排错入口：`docs/getting-started.md` 的「排错」一节。

---

## 9. 测试策略

129 passed。分层：

| 层 | 覆盖 |
|---|---|
| `plugin.test.ts` | 工具/命令/技能注册、开关项、守卫与品牌注入已挂载 |
| `tools.test.ts` | 每个工具→精确 argv（含引号转义、开关取反） |
| `policy.test.ts` | 策略引擎全语义（46 项） |
| `brand.test.ts` | token 明暗完整性、index 重写、玻璃层、**4 条回归守卫** |
| `workspace-picker.test.ts` | 游标规则（纯函数）、落点反馈、失败不静默 |
| `copy-overrides.test.ts` | 替换/可逆/后渲染副本 |
| `integration.test.ts` | 对真实 Proteus 工程跑 CLI（`PROTEUS_CODE_TEST_CHECKOUT` 门控） |

**4 条回归守卫是刻意留的**，专锁第 4 节那两个坑，改玻璃层前先看它们（都在 `brand.test.ts`）：

| 守卫 | 锁住的事实 |
|---|---|
| `never puts backdrop-filter, isolation, or stacking changes on the sidebar` | 侧边栏是弹窗的祖先，不得打滤镜/隔离/改 z-index |
| `never creates a stacking context on a glazed host` | 不得用 `isolation: isolate`（会困住 fixed 弹窗） |
| `leaves overlay host rules free of position overrides` | 浮层定位归客户端，宿主规则不得改 `position` |
| `clears the opaque token fill, or the backdrop could never show through` | 不清不透明底色，背景透不过来 |

**注意**：「清底色不得用通配符」这条教训**只有代码注释、没有测试**——它靠人工遵守。若日后有人改回 `*:not(button)`，测试不会失败。

---

## 10. 待办

- **未验证**：`workspacePicker: false` 开关（代码在，未实跑）；`prefers-reduced-transparency` 降级路径（未在真机验证）。
- **未做**：Electron 签名 / 打包 / 自动更新。当前是开发态启动。
- **上游契约漂移风险**：DSH 处于 developer preview。`docs/architecture.md` 末尾有「上游契约清单」，DSH 改版时对照它。
