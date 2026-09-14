<!-- src/App.vue —— 站点根组件（SPA 壳：RouterView 按当前路由渲染页面）
     壳本身只放 RouterView —— 头部/页脚都在页面里，因为本站是单页，不需要跨页共享的 chrome。 -->
<script setup lang="ts">
import RouterView from './router/RouterView.vue'
</script>

<template>
  <RouterView />
</template>
