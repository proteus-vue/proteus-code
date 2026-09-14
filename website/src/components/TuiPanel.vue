<!-- src/components/TuiChrome.vue —— TUI 的界面原语（面板 / 键位行 / 状态栏 / 区块标题）
     对齐 TUI 的成对用法：`┃` 单边条面板、`·` 分隔的键位行、底部状态栏。
     四个都只做这一件事，所以全站观感一致 —— 这正是 parity 文档里
     "新界面必须复用原语，不得再手画" 那条纪律在 Web 上的对应物。 -->
<script setup lang="ts">
import { computed } from 'vue'
import type { ThemeName } from '../theme'

// ── 面板：TUI 的分组容器就是「左单边条 + 底色」，不画四角框 ──────────
defineOptions({ inheritAttrs: false })

const props = withDefaults(
  defineProps<{
    /** 面板色号，显示在左上角（对齐 TUI 面板的标题标记） */
    title?: string
    /** 左侧竖条颜色 */
    barColor?: string
    /** 是否方形（由主题的 square 决定，这里只做覆盖） */
    square?: boolean
  }>(),
  { barColor: 'var(--neo-primary)', square: false },
)

const barStyle = computed(() => ({ background: props.barColor }))
</script>

<template>
  <section class="tpanel" :class="{ square: props.square }">
    <span class="tpanel-bar" :style="barStyle" aria-hidden="true" />
    <div class="tpanel-body">
      <p v-if="title" class="tpanel-title">
        <span class="sigil" aria-hidden="true">◆</span>
        <span>{{ title }}</span>
      </p>
      <slot />
    </div>
  </section>
</template>

<style scoped>
.tpanel {
  display: flex;
  background: color-mix(in srgb, var(--neo-panel) 72%, transparent);
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius);
  overflow: hidden;
}
.square {
  border-radius: 0;
}

/* 单边条：TUI 全站的分组标记（审批面板、侧栏都用它，不画四角框） */
.tpanel-bar {
  flex: 0 0 3px;
}

.tpanel-body {
  flex: 1 1 auto;
  min-width: 0;
  padding: 1.1rem 1.25rem;
}

.tpanel-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin: 0 0 0.7rem;
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--neo-fg-dim);
}
.sigil {
  color: var(--neo-primary);
}
</style>
