<!-- src/components/StatsBand.vue —— 数据带
     对标同行：把关键数字从正文里拎出来做成一条视觉带，扫一眼就能记住产品"多大"。
     ★ 数字全部可从仓库门禁复现（cargo test / verify.sh），不是宣传口径。 -->
<script setup lang="ts">
const stats = [
  { value: '5', label: '宿主共享一核', note: 'TUI / Desktop / Web / Exec / app-server' },
  { value: '32', label: 'crate 分层', note: 'L0–L5，依赖只能向下' },
  { value: '1104', label: '测试全绿', note: '内核 / 内存 / SPI / 宿主' },
  { value: '0', label: 'unsafe / warning', note: '生产代码，门禁强制' },
]
</script>

<template>
  <section class="band">
    <div class="wrap grid">
      <div v-for="s in stats" :key="s.label" class="stat reveal">
        <div class="value">{{ s.value }}</div>
        <div class="label">{{ s.label }}</div>
        <div class="note">{{ s.note }}</div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.band {
  position: relative;
  padding: 3.2rem 0;
  border-top: 1px solid var(--neo-border);
  border-bottom: 1px solid var(--neo-border);
  background: linear-gradient(180deg, var(--neo-panel), transparent);
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  gap: 1.5rem;
}

.stat {
  padding-left: 1.2rem;
  /* 左侧一道主色短条：与 TUI 面板的"单边条"语言一致 */
  border-left: 2px solid color-mix(in srgb, var(--neo-primary) 45%, transparent);
}

.value {
  font-size: var(--fs-stat);
  font-weight: 700;
  line-height: 1.05;
  letter-spacing: -0.03em;
  background: linear-gradient(120deg, var(--neo-fg), var(--neo-primary));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.label {
  margin-top: 0.4rem;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--neo-fg);
}

.note {
  margin-top: 0.15rem;
  font-size: var(--fs-xs);
  color: var(--neo-fg-faint);
}
</style>
