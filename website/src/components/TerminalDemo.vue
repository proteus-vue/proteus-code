<!-- src/components/TerminalDemo.vue —— 终端演示块（静态）
     ★ 这是**示意**，不是录屏：内容取自真实执行输出，但为静态文本。
     理由：TUI 显式要求真终端（stdin 必须是 tty，管道下直接报错），录屏需要额外的
     pty 工具链；而本页要的是"看一眼就知道这是什么"，静态块更好控、不会随版本失效。
     使用的输出由离线桩 provider 实跑得到（无需 API key），命令写在块下方以便复现。

     外观对标同行的产品截图处理：外发光 + 顶部高光边 + 等宽字体行高放松。 -->
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
  position: relative;
  margin: 0;
  background: linear-gradient(180deg, var(--neo-panel), #100f16);
  border: 1px solid var(--neo-border-active);
  border-radius: var(--radius);
  overflow: hidden;
  /* 外发光：深色底上不用阴影（看不出来），用同色系光晕 */
  box-shadow: 0 0 0 1px rgba(167, 139, 250, 0.06), 0 24px 70px -20px rgba(139, 92, 246, 0.35);
}

/* 顶部一道高光边（同行截图常见，让"窗口"有玻璃感） */
.term::before {
  content: "";
  position: absolute;
  inset: 0 0 auto;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(167, 139, 250, 0.6), transparent);
}

.bar {
  display: flex;
  align-items: center;
  gap: 0.7rem;
  padding: 0.66rem 0.95rem;
  background: color-mix(in srgb, var(--neo-element) 70%, transparent);
  border-bottom: 1px solid var(--neo-border);
}

.dots {
  display: inline-flex;
  gap: 0.4rem;
}
.dots i {
  width: 0.66rem;
  height: 0.66rem;
  border-radius: 50%;
  background: var(--neo-border-active);
}
.dots i:first-child {
  background: #4a3a52;
}

.title {
  flex: 1 1 auto;
  min-width: 0;
  font-family: var(--neo-mono);
  font-size: var(--fs-xs);
  color: var(--neo-fg-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.badge {
  flex: 0 0 auto;
  padding: 0.14rem 0.55rem;
  font-size: 0.68rem;
  color: var(--neo-warning);
  border: 1px solid color-mix(in srgb, var(--neo-warning) 55%, transparent);
  border-radius: var(--radius-pill);
  opacity: 0.9;
}

.body {
  margin: 0;
  padding: 1.15rem 1.2rem;
  overflow-x: auto;
  font-size: 0.82rem;
  line-height: 1.75;
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
  color: var(--neo-fg-faint);
}

@media (max-width: 640px) {
  /* 标题在窄屏允许折行而非省略号：省略号会切掉"真实落盘"这个关键信息
     （它正是这一块想证明的事）。标题栏高度略变，但信息完整更重要。 */
  .title {
    white-space: normal;
    overflow: visible;
    line-height: 1.35;
    min-width: 0;
  }
  .bar {
    align-items: flex-start;
  }
  .body {
    font-size: 0.7rem;
    line-height: 1.75;
  }
  /* ★ 输出行在窄屏折行而非裁切：这是产品演示，被切掉一半反而像坏了。
     pre + white-space:pre 会保留缩进与空行，故这里改成 pre-wrap 让长行折行。 */
  .line {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
}
</style>
