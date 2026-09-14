<!-- src/components/FeatureIcon.vue —— 特性图标（内联 SVG，零依赖）
     为什么自绘而不是引图标库：本站的全部依赖都钉着 beta 版本，多一个图标库就多一个
     会漂移的依赖；而这 6 个图标各十几行，画一次就够。线宽统一 1.6，风格与终端观感一致。 -->
<script setup lang="ts">
defineProps<{ name: string }>()

// 每个图标只有 path 集合，统一在模板里套 <svg> 外壳（尺寸/描边/圆角端点一处定义）
const PATHS: Record<string, string[]> = {
  // 盾牌：真实 OS 级沙箱
  shield: ['M12 3l7 3v6c0 4.5-3 7.6-7 9-4-1.4-7-4.5-7-9V6l7-3z', 'M9 12l2 2 4-4'],
  // 双轴：沙箱 × 审批
  axes: ['M4 20V4', 'M4 20h16', 'M8 16l4-4 3 2 5-6'],
  // 插头/SPI：可插拔后端
  plug: ['M9 3v5', 'M15 3v5', 'M6 8h12v3a6 6 0 01-12 0V8z', 'M12 17v4'],
  // 终端提示符：离线可试
  terminal: ['M4 6h16v12H4z', 'M8 11l2 2-2 2', 'M13 15h4'],
  // 量尺：内存有界
  ruler: ['M3 9h18v6H3z', 'M7 9v3', 'M11 9v2', 'M15 9v3', 'M19 9v2'],
  // 环形箭头：可回放
  replay: ['M20 12a8 8 0 11-2.3-5.6', 'M20 4v5h-5'],
}
</script>

<template>
  <span class="fi" aria-hidden="true">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
      stroke-linecap="round" stroke-linejoin="round">
      <path v-for="(d, i) in PATHS[name] || []" :key="i" :d="d" />
    </svg>
  </span>
</template>

<style scoped>
.fi {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2.3rem;
  height: 2.3rem;
  margin-bottom: 0.95rem;
  color: var(--neo-primary);
  background: var(--neo-selected);
  border: 1px solid var(--neo-border);
  border-radius: 10px;
}
.fi svg {
  width: 1.25rem;
  height: 1.25rem;
}
</style>
