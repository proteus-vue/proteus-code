// src/router/index.ts —— 应用侧路由单例（框架约定：路由表由应用注入 createRouter）
// 路由表来自 ./auto-routes（本站 Web-only，该文件签入仓库 —— 脚手架原本由 scripts/gen-routes.ts
// 生成，但那个脚本同时写小程序 app.json，故本站移除了它）
import { createRouter } from '@proteus-vue/router'
import { routes } from './auto-routes'

// 单页站暂时只用锚点导航，这个实例是**给后续内容页留的导航入口**
// （页面里 `import { router } from '../router'` 即可跳转），不是死代码。
export const router = createRouter(routes)

// 类型契约透传（应用页面可直接 import type { PageOnLoad } from './router'，无需深路径）
export type {
  RouteRecord,
  RouteMeta,
  RouteParams,
  RouteParamsByName,
  PageOnLoad,
  NavigateOptions,
  RouterInstance,
} from '@proteus-vue/router'
