// proteus.config.ts —— 官网唯一配置（Proteus 约定：vite 配置由此组装）
//
// 这是 **Web-only** 站点（neo 的官网），与脚手架默认的双端配置差异：
//   - platform='web'、skyline 关闭
//   - customRoute 给空 builders：该字段在 wx.router 自定义路由（小程序转场）场景下使用，
//     Web 端没有对应语义 —— 但**类型契约要求它必填**，故显式给空值而非删除
//   - appid 留空：它只用于小程序 project.config.json，Web 构建不消费
//
// ★必填 8 项（platform / skyline / appid / pagesDir / routesOutput / customRoute /
//   setDataBridge / style）—— 依据实际安装的包类型定义：
//   node_modules/@proteus-vue/types/dist/config-schema.d.ts:5
import type { ProteusConfig } from '@proteus-vue/plugin-vite'

const config: ProteusConfig = {
  platform: 'web',
  skyline: false,
  appid: '',
  pagesDir: 'src/pages',
  // 路由表产物路径。**Web-only 下该文件签入仓库**：脚手架原本由 scripts/gen-routes.ts
  // 生成，但那脚本同时写小程序 app.json，故本站移除了它；新增页面时手工补一条即可。
  routesOutput: 'src/router/auto-routes.ts',
  customRoute: {
    // 不注册 wx.router 预设：那是小程序 Skyline 的转场（halfScreen/slideUp/scaleDown），
    // Web 端由前端 CSS 转场承担，无需框架预设。
    registerPresets: false,
    builders: {},
  },
  setDataBridge: { batchWindow: 16, perComponent: false },
  style: { px2rpx: false, rpxRatio: 2 },

  // 刻意**不启用** audit 规则：其 `no-web-platform-api` 默认 error，会拦截 window/document
  // 等裸调用，而官网必须做复制按钮、滚动定位这类 DOM 操作。那套规则是给
  // "一套源码编译到小程序" 的跨端场景准备的，对纯 Web 站点不适用 —— 这不是绕过门禁，
  // 是规则本身不适用于本目标（本站无 MP 目标）。
}

export default config
