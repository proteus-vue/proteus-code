<!-- src/components/TuiBackdrop.vue —— 背景纹理（对齐 TUI `/background` 的四种纹理）
     ★ 为什么用 canvas 而不是 CSS 渐变：TUI 的星场是**确定性点阵**
     （`Background::cell(row, col, cols)` 同一格永远同一结果 —— 随机会让每帧重画时闪成噪点）。
     复刻同一算法就能得到同样的观感，而不是"随便撒点星星"。 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { type BgKind } from '../theme'

const props = defineProps<{ kind: BgKind }>()

const canvas = ref<HTMLCanvasElement | null>(null)
let raf = 0

// ── 与 TUI 一致的确定性伪随机（同一 (row,col) 恒等）──────────────────
function hash(row: number, col: number): number {
  let h = (row * 73856093) ^ (col * 19349663)
  h = (h ^ (h >>> 13)) * 1274126177
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295
}

/** 取值域：TUI 的 cell() 返回 (char, Brightness)。这里只取"是否画 + 多亮"。 */
function cell(kind: BgKind, row: number, col: number): number {
  const h = hash(row, col)
  switch (kind) {
    case 'stars':
      // 稀疏：约 1/14 的格子有点，且亮度分两档（对齐 star_dim / star_bright）
      if (h < 0.93) return 0
      return h > 0.985 ? 1 : 0.45
    case 'dots':
      // 点阵：规律排列（每 4 格一个点），亮度均匀
      return row % 4 === 0 && col % 4 === 0 ? 0.55 : 0
    case 'diagonal':
      // 斜纹：错开排列的斜线
      return (row + col) % 6 === 0 ? 0.5 : 0
    default:
      return 0
  }
}

function draw() {
  const el = canvas.value
  if (!el) return
  const dpr = window.devicePixelRatio || 1
  const w = el.clientWidth
  const h = el.clientHeight
  el.width = Math.floor(w * dpr)
  el.height = Math.floor(h * dpr)
  const ctx = el.getContext('2d')
  if (!ctx) return
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)

  const cs = getComputedStyle(document.documentElement)
  const starColor = cs.getPropertyValue('--neo-primary').trim() || '#a78bfa'
  const textColor = cs.getPropertyValue('--neo-fg').trim() || '#ededed'
  const starAlpha = Number(cs.getPropertyValue('--neo-star-alpha').trim() || '0.5')

  const CELL = 22 // 格边长（px）：够密才有"星场"感，够疏才不吵
  const cols = Math.ceil(w / CELL)
  const rows = Math.ceil(h / CELL)

  if (props.kind !== 'none') {
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const b = cell(props.kind, r, c)
        if (!b) continue
        ctx.globalAlpha = starAlpha * b
        // 星场/点阵偏主色，斜纹偏正文色（斜纹用主色会像划痕）
        ctx.fillStyle = props.kind === 'diagonal' ? textColor : starColor
        const size = props.kind === 'dots' ? 1.6 : b > 0.9 ? 1.9 : 1.2
        ctx.fillRect(c * CELL + CELL / 2, r * CELL + CELL / 2, size, size)
      }
    }
    ctx.globalAlpha = 1
  }
}

function schedule() {
  cancelAnimationFrame(raf)
  raf = requestAnimationFrame(draw)
}

onMounted(() => {
  schedule()
  window.addEventListener('resize', schedule)
  // 主题切换要重画：canvas 的颜色读的是 CSS 变量，改变量不会自己重绘。
  window.addEventListener('neo-theme-changed', schedule)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  window.removeEventListener('resize', schedule)
  window.removeEventListener('neo-theme-changed', schedule)
})
// 纹理变化也要重画（算法换了）
watch(() => props.kind, schedule)
</script>

<template>
  <div class="backdrop" aria-hidden="true">
    <canvas ref="canvas" class="stars" />
    <div class="veil" />
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  overflow: hidden;
}
.stars {
  width: 100%;
  height: 100%;
  display: block;
}
/* 顶部压暗：让首屏文字压在更暗的底上，星点不会干扰阅读 */
.veil {
  position: absolute;
  inset: 0;
  background: radial-gradient(
    ellipse 90% 55% at 50% 0%,
    color-mix(in srgb, var(--neo-backdrop) 30%, transparent),
    var(--neo-backdrop) 78%
  );
  opacity: 0.92;
}
</style>
