<!-- src/components/TuiBackdrop.vue —— 背景纹理（对齐 TUI `/background` 的四种纹理）
     ★ 算法与密度取自 crates/neo-host-tui/src/appearance.rs 的 `Background::cell()`：
       · Stars    : `v = h % 1000`，v≤1 亮、v≤9 暗 → 约 1%（确定性哈希，同一格恒定）
       · Dots     : 规律网格（row%3==1 && col%6==2）—— 规律意味着不闪、更安静
       · Diagonal : 相位 1/17 且每 2 行才考虑一次 —— 作者注释写明目标是 2%~5%
                     "看得见但不抢眼"，1/7（14%）会盖过正文
     参数按网页尺度放大（终端格子小、离眼睛近，网页要更密更亮才看得见），
     但保持同一比例关系与"确定性"（不随机 —— 随机会在重绘时闪成噪点）。 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { type BgKind } from '../theme'

const props = defineProps<{ kind: BgKind }>()

const canvas = ref<HTMLCanvasElement | null>(null)
let raf = 0

/**
 * ★ 实现分工（这是修"背景完全没效果"时定的）：
 *   · stars    → canvas：它是**确定性伪随机**点场（对齐 TUI 的 stars_like 哈希），
 *                 CSS 表达不了"同一格恒定但又不规律"。
 *   · dots     → CSS：规律网格。用 repeating 渐变比 canvas 更锐利，
 *                 且不受 devicePixelRatio 缩放影响、零 JS。
 *   · diagonal → CSS：规律斜纹，理由同上。
 *   · none     → 什么都不画。
 *
 * 早先四种都用 canvas，结果 dots/diagonal 的覆盖率算不准：3px 的点配 32px 间距
 * 只有 0.8% 覆盖，实测"几乎无差异"（用户反馈"完全没效果"）。规律图案交给 CSS，
 * 覆盖率由 background-size 精确决定，不再靠估算。
 */

/** 确定性哈希（同一 (row,col,cols) 恒定）—— 与 TUI 的 stars_like 同族思路 */
function hash(row: number, col: number, cols: number): number {
  let h = Math.imul(row + 1, 0x9e3779b9) ^ Math.imul(col + 1, 0xc2b2ae3d) ^ Math.imul(cols, 0x165667b1)
  h ^= h >>> 29
  h = Math.imul(h, 0xbf58476d)
  h ^= h >>> 32
  return (h >>> 0) % 1000
}

/** 星场：返回亮度档（0 = 不画，0.42 = 暗，1 = 亮），对齐 TUI 的 Dim/Bright 两档 */
function starCell(row: number, col: number, cols: number): number {
  const v = hash(row, col, cols)
  // 对齐 TUI 的 modulo=1000 / hot=9 的**比例**，整体调密（网页上 1% 等于看不见）
  if (v <= 8) return 1
  if (v <= 90) return 0.42
  return 0
}

function draw() {
  const el = canvas.value
  if (!el) return
  // 非星场纹理完全交给 CSS（见模板），canvas 不参与
  if (props.kind !== 'stars') {
    el.width = 0
    el.height = 0
    return
  }
  const dpr = window.devicePixelRatio || 1
  const w = el.clientWidth
  const h = el.clientHeight
  if (!w || !h) return
  el.width = Math.floor(w * dpr)
  el.height = Math.floor(h * dpr)
  const ctx = el.getContext('2d')
  if (!ctx) return
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)

  const cs = getComputedStyle(document.documentElement)
  const primary = cs.getPropertyValue('--neo-primary').trim() || '#a78bfa'
  const alphaMul = Number(cs.getPropertyValue('--neo-star-alpha').trim() || '1') || 1

  const CELL = 16
  const cols = Math.ceil(w / CELL)
  const rows = Math.ceil(h / CELL)

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const b = starCell(r, c, cols)
      if (!b) continue
      // 亮档更实、暗档更虚；再乘主题系数（亮色主题要压暗，否则显得脏）
      ctx.globalAlpha = Math.min(1, alphaMul * (b > 0.9 ? 0.62 : 0.3))
      ctx.fillStyle = primary
      const size = b > 0.9 ? 2.6 : 1.8
      ctx.fillRect(c * CELL + CELL / 2, r * CELL + CELL / 2, size, size)
    }
  }
  ctx.globalAlpha = 1
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
watch(() => props.kind, schedule)
</script>

<template>
  <div class="backdrop" aria-hidden="true">
    <canvas v-show="kind === 'stars'" ref="canvas" class="stars" />
    <!-- 规律纹理走 CSS：锐利、精确、零 JS。
         size 与线宽决定覆盖率（约 3%~4%），是实测过的值。 -->
    <div v-if="kind === 'dots'" class="tex dots" />
    <div v-if="kind === 'diagonal'" class="tex diagonal" />
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

.tex {
  position: absolute;
  inset: 0;
  /* 强度由主题系数控制（亮色主题下压暗，否则显得脏） */
  opacity: calc(var(--neo-star-alpha) * 0.5);
}

/* 点阵：规律网格。覆盖率 = 点面积 / 格面积，直接由下面两个数决定
   （半径 1.8px 的点、18px 间距 → 约 3%）。对齐 TUI 注释里的 2%~5% 区间
   —— "看得见但不抢眼"。TUI 的 dots 还特意比 stars 更安静（规律 → 不闪）。 */
.dots {
  background-image: radial-gradient(
    circle,
    color-mix(in srgb, var(--neo-primary) 80%, transparent) 1.8px,
    transparent 1.8px
  );
  background-size: 18px 18px;
}

/* 斜纹：45° 细线。★线宽与周期决定覆盖率 —— 早先 1px/6px 实测达 15%，
   正是 TUI 注释里"会盖过正文"的量级。现取 1px/14px（约 5~7%），
   且整体再乘 .tex 的 opacity，让它退到"看得见但不抢眼"。 */
.diagonal {
  background-image: repeating-linear-gradient(
    45deg,
    transparent 0,
    transparent 13px,
    color-mix(in srgb, var(--neo-primary) 55%, transparent) 13px,
    color-mix(in srgb, var(--neo-primary) 55%, transparent) 14px
  );
}

/* ★ 这里**刻意没有遮罩层**。
   原先加过一层 `opacity: 0.92` 的 radial-gradient 压暗，渐变末端是与页面同色的
   `rgb(10,10,12)` 不透明 —— 结果把画好的纹理整片擦掉，表现为"背景完全没效果"
   （实测：canvas 上 588 个像素有颜色，屏幕上看不见）。纹理本身就是低对比度装饰，
   需要压暗的是它在亮色主题下的强度，那由 --neo-star-alpha 控制。 */
</style>
