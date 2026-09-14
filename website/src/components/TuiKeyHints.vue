<!-- src/components/TuiKeyHints.vue —— 键位提示行
     对齐 TUI 底部的 `↑/↓ 选择 · Enter 确认 · y/n 快捷 · Ctrl+C 退出`：
     键位用主色，说明用次要色，之间用 `·` 分隔。TUI 里这是常驻的"此刻能做什么"，
     放到官网上正好回答"这东西怎么用"。 -->
<script setup lang="ts">
defineProps<{
  hints: { key: string; text: string }[]
  /** 右侧附加说明（对齐 TUI 状态栏右端） */
  tail?: string
}>()
</script>

<template>
  <p class="hints">
    <template v-for="(h, i) in hints" :key="h.key">
      <span v-if="i > 0" class="sep" aria-hidden="true">·</span>
      <kbd>{{ h.key }}</kbd>
      <span class="txt">{{ h.text }}</span>
    </template>
    <span v-if="tail" class="tail">{{ tail }}</span>
  </p>
</template>

<style scoped>
.hints {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.45rem;
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--neo-fg-faint);
}

kbd {
  font-family: var(--neo-mono);
  font-size: 0.94em;
  color: var(--neo-primary);
}

.sep {
  color: var(--neo-border-active);
}

.txt {
  color: var(--neo-fg-dim);
}

/* 右端说明（margin-left:auto 让它在宽屏贴右，窄屏自动换行） */
.tail {
  margin-left: auto;
  padding-left: 1rem;
  color: var(--neo-fg-faint);
}
</style>
