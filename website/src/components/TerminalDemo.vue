<!-- src/components/TerminalDemo.vue —— 终端演示块（静态）
     ★ 这是**示意**，不是录屏：内容取自真实执行输出，但为静态文本。
     理由：TUI 显式要求真终端（stdin 必须是 tty，管道下直接报错），录屏需要额外的
     pty 工具链；而本页要的是"看一眼就知道这是什么"，静态块更好控、不会随版本失效。
     使用的输出由离线桩 provider 实跑得到（无需 API key），命令写在块下方以便复现。 -->
<script setup lang="ts">
interface Line {
  /** 行类型决定配色 */
  kind?: 'cmd' | 'info' | 'tool' | 'ok' | 'dim' | 'plain'
  text: string
}

defineProps<{ title: string; lines: Line[] }>()
</script>

<template>
  <figure class="term">
    <figcaption class="bar">
      <span class="dots" aria-hidden="true"><i /><i /><i /></span>
      <span class="title">{{ title }}</span>
      <span class="badge">静态示意</span>
    </figcaption>
    <pre class="body"><code><span
        v-for="(l, i) in lines"
        :key="i"
        class="line"
        :class="l.kind || 'plain'"
      >{{ l.text }}
</span></code></pre>
    <slot />
  </figure>
</template>

<style scoped>
.term {
  margin: 0;
  background: var(--neo-panel);
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius);
  overflow: hidden;
}

.bar {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.55rem 0.9rem;
  background: var(--neo-element);
  border-bottom: 1px solid var(--neo-border);
}

.dots {
  display: inline-flex;
  gap: 0.35rem;
}
.dots i {
  width: 0.62rem;
  height: 0.62rem;
  border-radius: 50%;
  background: var(--neo-border-active);
}

.title {
  flex: 1 1 auto;
  font-family: var(--neo-mono);
  font-size: 0.78rem;
  color: var(--neo-fg-dim);
}

.badge {
  flex: 0 0 auto;
  padding: 0.1rem 0.5rem;
  font-size: 0.68rem;
  color: var(--neo-warning);
  border: 1px solid var(--neo-warning);
  border-radius: 999px;
  opacity: 0.85;
}

.body {
  margin: 0;
  padding: 0.95rem 1rem;
  overflow-x: auto;
  font-size: 0.8rem;
  line-height: 1.65;
}

.line {
  display: block;
  white-space: pre;
}
.line.cmd {
  color: var(--neo-fg);
  font-weight: 600;
}
.line.info {
  color: var(--neo-info);
}
.line.tool {
  color: var(--neo-primary);
}
.line.ok {
  color: var(--neo-success);
}
.line.dim {
  color: var(--neo-fg-dim);
}
</style>
