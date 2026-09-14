// vite.config.ts —— 官网构建配置
//
// 起手自 `npm create @proteus-vue/proteus` 的模板，但做了 **Web-only 收敛**：
//   - 移除小程序分支（mpTransform 管线、mp-entry-stub 入口）——本站没有 MP 目标
//   - 加上 `base`（GitHub Pages + 自定义域名的部署路径）
//
// 保留的部分是框架契约：`<route>` 自定义块的虚拟模块处理、`__PROTEUS_*__` 编译期常量、
// 以及 `@` → `src` 别名。少了它们，"页面即文件 + <route> 声明 meta" 那套约定就跑不起来。
import { defineConfig } from 'vite'
import type { Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

/** 处理 <route> 自定义块虚拟模块（?vue&type=route），保证 Web 构建不报错 */
function routeBlocksPlugin(): Plugin {
  return {
    name: 'proteus-route-blocks',
    enforce: 'pre',
    transform(code, id) {
      if (id.includes('?vue&type=route')) {
        return { code: `export default ${code}`, map: null }
      }
      return null
    },
  }
}

export default defineConfig(() => ({
  // 部署路径。自定义域名（neo.proteus-vue.cn）挂在根路径，故默认 '/'。
  // 若将来改为 GitHub Pages 的项目子路径（/proteus-code/），用 PROTEUS_BASE 注入即可，
  // 无需改代码 —— 与 Proteus 官网自身的做法一致。
  base: process.env.PROTEUS_BASE ?? '/',

  define: {
    __PROTEUS_DEBUG__: process.env.PROTEUS_DEBUG === '1',
    // 本站无 MP 目标，Skyline 恒关
    __PROTEUS_SKYLINE__: false,
  },

  plugins: [vue(), routeBlocksPlugin()],

  resolve: {
    alias: [{ find: '@', replacement: fileURLToPath(new URL('./src', import.meta.url)) }],
  },

  build: {
    target: 'es2018',
    outDir: 'dist/web',
    emptyOutDir: true,
  },
}))
