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
   ├─ main.ts            # 入口
   ├─ App.vue            # 根组件（只放 RouterView）
   ├─ pages/index.vue    # ★ 单页内容（新增页面放这里 + 补 auto-routes.ts）
   ├─ components/        # CopyCommands / TerminalDemo
   ├─ router/            # 路由表 + RouterView（Web 精简版）
   └─ styles/global.css  # 配色取自 TUI 真实主题
```

与脚手架默认产物的差异（都写在对应文件的注释里）：
- 删除了小程序侧文件（`main.mp.ts`、`shims/mp.d.ts`、`scripts/gen-routes.ts`）
- `proteus.config.ts` 设 `platform: 'web'`；`customRoute` 留空值（类型必填但 Web 无意义）
- **不开 `audit` 规则**：其 `no-web-platform-api` 会拦 `window`/`document` 裸调用，
  而本站要做复制按钮等 DOM 操作。该规则面向跨端场景，不适用于纯 Web 站点
- `router/RouterView.vue` 换成 Web 精简版（脚手架的 378 行转场是给小程序多页写的）

## 本地开发

```bash
cd website
npm install
npm run dev        # 开发服务器
npm run build      # 类型检查（vue-tsc）+ 构建 → dist/web
npm run preview    # 预览构建产物
```

## 部署

推到 `main`（且改动涉及 `website/**`）即触发 `.github/workflows/website.yml`：
构建 → 上传 Pages 产物 → 部署。PR 只做构建校验，不部署。

**首次需要手动做的两件事**（仓库/域名侧，不在本仓库内）：
1. 仓库 Settings → Pages → Source 选 **GitHub Actions**
2. 域名 DNS 加一条 **CNAME** 记录：`neo.proteus-vue.cn` → `proteus-vue.github.io`

线上是根路径部署，故 `base = '/'`；若改为项目子路径（`/proteus-code/`），
用环境变量 `PROTEUS_BASE` 注入即可，无需改代码。
