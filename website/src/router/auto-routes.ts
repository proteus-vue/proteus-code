// src/router/auto-routes.ts —— 应用侧路由表（路由表由应用注入 createRouter）
//
// 脚手架原本用 scripts/gen-routes.ts 生成此文件，但那个脚本同时写小程序 app.json，
// 本站是 Web-only 且已移除该脚本 —— **故本文件签入仓库、手工维护**。
// 新增页面时：在 src/pages/ 加 .vue，然后在这里补一条记录。
import type { RouteRecord } from '@proteus-vue/router/types'

export const routes: RouteRecord[] = [
  {
    name: 'index',
    path: 'pages/index',
    component: '../pages/index.vue',
    meta: { title: 'neo —— 用 Rust 重写的编程 Agent 内核', isTab: true },
  },
]

export const tabRoutes = routes.filter((r) => r.meta?.isTab)

export const routeMap: Record<string, RouteRecord> = routes.reduce((m, r) => {
  m[r.name] = r
  return m
}, {} as Record<string, RouteRecord>)

// 类型提示：按路由名索引的参数类型表（来源：<route> 块 params 声明）
declare module '@proteus-vue/router/types' {
  interface RouteParamsByName {
    index: Record<string, never>
  }
}
