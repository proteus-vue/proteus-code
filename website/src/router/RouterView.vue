<!-- src/router/RouterView.vue —— Web 渲染容器（应用壳）
     与脚手架自带版本的差异：脚手架那份是为**小程序多页转场**写的（378 行 CSS 转场 +
     遮罩层 + .page 白底/暗色覆盖），单页营销站用它会引入无关的层叠与白底样式。
     这里保留框架的核心约定（routeMap + import.meta.glob 懒加载 + 按 route 渲染），
     去掉转场机制 —— 营销站是单页锚点导航，不需要页间转场。 -->
<script setup lang="ts">
import { computed, defineAsyncComponent } from 'vue'
import type { Component } from 'vue'
import { routeMap } from './auto-routes'

// 懒加载全部页面：Web 端按页面自动 code-split。
// glob 相对本文件（src/router/）→ src/pages/**
const modules = import.meta.glob('../**/pages/**/*.vue')

const currentRoute = 'pages/index'

const view = computed<Component | null>(() => {
  const rec =
    routeMap[currentRoute] || Object.values(routeMap).find((r) => r.path === currentRoute)
  if (!rec) return null
  const load = (modules as Record<string, () => Promise<unknown>>)[rec.component]
  return load ? defineAsyncComponent(load as () => Promise<Component>) : null
})
</script>

<template>
  <div class="router-view">
    <Transition name="fade" mode="out-in">
      <component :is="view" v-if="view" :key="currentRoute" />
      <div v-else class="page-missing">页面未找到</div>
    </Transition>
  </div>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.page-missing {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 60vh;
  color: var(--neo-fg-dim, #8b8ba7);
}
</style>
