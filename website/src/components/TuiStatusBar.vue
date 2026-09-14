<!-- src/components/TuiStatusBar.vue —— 底部状态栏
     对齐 TUI 最底行的「左：工作区路径 ······ 右：就绪」。
     固定的状态栏是终端应用最显著的特征之一，放到官网既有辨识度
     也实用：它挂载在这里显示主题名，与 TUI 显示模式/模型是同一种信息角色。 -->
<script setup lang="ts">
import { useTheme } from '../composables/useTheme'

const { current } = useTheme()

// 右侧状态：跟随主题变化，与 TUI 状态栏右端（就绪 / 运行中）同角色
const status = () => current.value.label
</script>

<template>
  <div class="statusbar">
    <span class="left">
      <span class="dot" aria-hidden="true" />
      <span>neo.proteus-vue.cn</span>
    </span>
    <span class="right">
      <span class="k">主题</span>
      <span class="v">{{ status() }}</span>
    </span>
  </div>
</template>

<style scoped>
.statusbar {
  position: fixed;
  inset: auto 0 0;
  z-index: 40;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  height: 1.7rem;
  padding: 0 0.9rem;
  font-family: var(--neo-mono);
  font-size: 0.7rem;
  color: var(--neo-fg-dim);
  background: color-mix(in srgb, var(--neo-panel) 92%, transparent);
  border-top: 1px solid var(--neo-border);
  backdrop-filter: blur(10px);
}

.left {
  display: inline-flex;
  align-items: center;
  gap: 0.45rem;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}

/* 状态点：TUI 用色点表示状态，这里保持同语义 */
.dot {
  width: 0.42rem;
  height: 0.42rem;
  border-radius: 50%;
  background: var(--neo-success);
  box-shadow: 0 0 6px var(--neo-success);
}

.right {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  white-space: nowrap;
}
.k {
  color: var(--neo-fg-faint);
}
.v {
  color: var(--neo-primary);
}
</style>
