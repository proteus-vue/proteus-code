# neo 官网

`neo.proteus-vue.cn` 的源码。**用 [Proteus](https://github.com/proteus-vue/proteus) 框架构建** ——
本组织自己写的 Vue 跨端框架（一份 Vue SFC → Web 零转换直跑 / 微信小程序 Skyline 原生）。

## 为什么用 Proteus 而不是普通 Vite

三条理由，按重要性排：

1. **dogfooding**：自己的框架先在自己的站点上跑。Proteus 官方官网就是这么做的
   （其 `website/package.json` 描述写着 "dogfooding：用 Proteus 自身构建"）。
   站点是框架最真实的用户，撞到的问题最值得修。
2. **结构约定**：Proteus 的「页面即文件 + `<route>` 块声明 meta + 路由表集中管理」
   让后续加内容页（架构、文档）不需要重新搭架子。
3. **验证框架的 Web 路径**：本站只用 `--target web`，是框架 Web 目标的一个真实用例。

## 代价（诚实记录）

| 代价 | 说明 |
|---|---|
| 全线 beta | Proteus 包版本是 `0.1.0`～`0.3.0-beta.0`，多数只有一个版本。故本站**钉死精确版本**（不用 `^`），升级需人工确认 |
| UI 要自己写 | 官网那套语义组件（`@proteus-vue/components` 的 `p-*`）**未发布 npm**，拿不到 → 本站 UI 全部手写 |
| 无 SSG | 框架的 SSG 未落地，产物是纯 SPA，搜索引擎抓不到正文（已用 `<noscript>` 兜底核心文案）。**已验证 SSR 可行**，将来可升级为 SSG |

## 目录

```
website/
├─ proteus.config.ts     # 唯一配置（框架约定：vite 配置由此组装）
├─ vite.config.ts        # 起手自脚手架，做了 Web-only 收敛（去掉小程序分支）
├─ index.html            # 入口 + SEO meta + <noscript> 兜底
├─ public/               # CNAME / robots.txt / sitemap.xml / favicon.svg
└─ src/
   ├─ main.ts            # 入口（含 .js 标记）
   ├─ App.vue            # 根组件（只放 RouterView）
   ├─ pages/index.vue    # ★ 单页内容（新增页面放这里 + 补 auto-routes.ts）
   ├─ components/        # 按区块拆分：SiteNav / HeroSection / StatsBand /
   │                     #   SectionHosts / SectionFeatures / SectionArch /
   │                     #   SectionWhy / SectionInstall / SectionBoundaries /
   │                     #   SiteFooter / TerminalDemo / CopyCommands / FeatureIcon
   ├─ composables/       # useReveal（滚动揭示）
   ├─ router/            # 路由表 + RouterView（Web 精简版）
   └─ styles/global.css  # 设计系统（色彩 / 类型尺度 / 间距节奏 / 动效令牌）
```

## 设计系统：站点本身就是一屏 TUI

配色不是另调的，**取自 TUI 的真实主题**（`crates/neo-host-tui/src/theme.rs`），
由 `src/composables/useTheme.ts` 写成 CSS 变量。更重要的是**界面语言整套搬过来**：

| 站点元素 | 对应 TUI 的实现 |
|---|---|
| 星场/点阵/斜纹背景 | `/background`（`appearance.rs` 的 `Background`，确定性点阵） |
| `███╗ ██╗` 六行块字词标 | 首屏 logo（`lib.rs` 的 `LOGO_LARGE`，逐行渐变） |
| `▌` 左强调条输入框 | 首屏输入框（两行：提示 + `模式 ⏵ 模型`） |
| 键位提示行 | 底部键位栏（`tab 补全  ctrl+/ 键位提示 …`） |
| 底部常驻状态栏 | TUI 最底行（左路径 / 右状态） |
| `┃` 单边条面板 | TUI 全站的分组惯例（不画四角框） |
| 9 套主题可实时切换 | `ctrl+t` |
| 外观设置面板 | `ctrl+t` / `/background` |

**为什么这么做**：官网是产品的第一屏。与其用文案描述"这是个体面的终端工具"，
不如让站点自己就是一屏 TUI —— 访客点两下主题就能理解「风格包 vs 换色」的区别
（切到 Terminal 连圆角一起变直角）。

四条排版原则写在 `src/styles/global.css` 顶部；两条纪律必须遵守：

- **三级文字也必须过 WCAG AA(4.5:1)**。深色站在这一档最容易偷懒，但 12–13px
  上是真读不清（实测过一次 3.7:1，已修）。
- **窄屏一律换行，绝不静默裁切**：命令与终端输出是给人复制/阅读的。
- **主题色只能用变量**（不要写死 `#rgb`）。曾因写死一个 `--neo-primary-strong`
  然后在重写 `:root` 时把它删掉，导致主按钮渐变失效、几乎看不见。
  改主题时用脚本扫一遍"引用了但未定义的 CSS 变量"能立刻抓出来。

## 本地开发

```bash
cd website
npm install
npm run dev        # 开发服务器
npm run build      # 类型检查（vue-tsc）+ 构建 → dist/web
npm run preview    # 预览构建产物
```

### 视觉验收怎么做

改动视觉后**不要只跑构建**（构建过了不代表好看）。用无头浏览器截图后交给
视觉验收，注意三点：

- **整页截图前必须先滚动一遍**。滚动揭示依赖 `IntersectionObserver`，
  不滚动的话首屏之外全是 `opacity:0`，截出来一片空白 —— 会误判成 bug。
  （`@media print` 里已对这种情况做了兜底。）
- **每个主题都要看**。9 套主题里 `light`（亮底）与 `terminal`（方角 CRT）
  是最容易出问题的两套 —— 亮色主题下任何写死深色的块都会变成"黑砖"。
  可以逐个注入 `localStorage` 的 `neo-site-appearance` 后截图对比。
- **横向溢出**要实测：`document.documentElement.scrollWidth > clientWidth`
  在 390 / 768 / 1440 三个宽度都不该成立。

## 部署

推到 `main`（且改动涉及 `website/**`）即触发 `.github/workflows/website.yml`：
构建 → 上传 Pages 产物 → 部署。PR 只做构建校验，不部署。

**首次需要手动做的两件事**（仓库/域名侧，不在本仓库内）：
1. 仓库 Settings → Pages → Source 选 **GitHub Actions**
2. 域名 DNS 加一条 **CNAME** 记录：`neo.proteus-vue.cn` → `proteus-vue.github.io`

线上是根路径部署，故 `base = '/'`；若改为项目子路径（`/proteus-code/`），
用环境变量 `PROTEUS_BASE` 注入即可，无需改代码。
