<!-- src/components/TuiBackdrop.vue —— 背景纹理（对齐 TUI `/background` 的四种纹理）
     ★ 实现分工：
       · stars    → canvas + rAF：**星芒**点场（十字光芒 + 光晕 + 闪烁）+ 流星滑落
       · dots     → CSS 渐变：规律网格（覆盖率由 background-size 精确决定）
       · diagonal → CSS 渐变：45° 细纹（线宽/周期决定覆盖率）
       · none     → 不画
     规律纹理用 CSS 更锐利且零 JS；星场要点光叠加与动画，只能 canvas。

     ★ 与 TUI 的关系（诚实说明取舍）：
       TUI 的星场是**静止**的 —— `appearance.rs` 的 `cell()` 是确定性哈希，同一格
       每帧恒定。原因是终端每帧全量重画，随机纹理会让画面**闪成噪点**。
       官网这里要做"星芒 + 流星"，就必须有动画。为不丢掉 TUI 那条经验，做法是：
         · 星点**位置与大小**用固定种子 PRNG → 每次加载完全一致，不会噪
         · 只有**亮度**随时间缓慢呼吸（闪烁），这是观感需求且不产生噪点
         · 流星是独立的短生命周期粒子，与星点无关

     ★ 性能（星场是纯装饰，不该拖累页面）：
       · 星芒**预渲染**成离屏精灵，每帧只 drawImage（不每帧建渐变）
       · 粒子数按面积算并设上限；DPR 参与换算但封顶 2
       · 标签页不可见时暂停 rAF；组件卸载时清理
       · 尊重 prefers-reduced-motion：只画一帧静态星场，不跑循环、不出流星 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from 'vue'
import { type BgKind } from '../theme'

const props = defineProps<{ kind: BgKind }>()

const canvas = ref<HTMLCanvasElement | null>(null)
let raf = 0

// ── 确定性 PRNG（mulberry32）：星点位置/大小每次加载一致 ────────────────
function makeRng(seed: number) {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

// ── 颜色工具：把 CSS 变量里的 hex 转成可调 alpha 的 rgba ───────────────
function toRgb(css: string): [number, number, number] {
  const m = css.trim()
  const h = m.replace('#', '')
  if (/^[0-9a-f]{6}$/i.test(h)) {
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]
  }
  if (/^[0-9a-f]{3}$/i.test(h)) {
    return [parseInt(h[0] + h[0], 16), parseInt(h[1] + h[1], 16), parseInt(h[2] + h[2], 16)]
  }
  const rgb = m.match(/(\d+)[,\s]+(\d+)[,\s]+(\d+)/)
  return rgb ? [+rgb[1], +rgb[2], +rgb[3]] : [167, 139, 250]
}
const rgba = ([r, g, b]: [number, number, number], a: number) => `rgba(${r},${g},${b},${a})`

// ── 星芒精灵：预渲染一次，之后每帧 drawImage ─────────────────────────────
// 结构（自内向外）：亮核 → 柔和光晕 → 十字光芒。
// 十字光芒用"把径向渐变压成极扁的椭圆"实现：中心粗、两端细，正是星芒的形状。
let spriteCache: { key: string; big: HTMLCanvasElement; small: HTMLCanvasElement } | null = null

function makeSprite(px: number, color: [number, number, number], withBeams: boolean): HTMLCanvasElement {
  const c = document.createElement('canvas')
  c.width = c.height = px
  const g = c.getContext('2d')
  if (!g) return c
  const cx = px / 2
  g.translate(cx, cx)

  // 光晕：中心白 → 主色 → 透明（柔和的"星气"）
  const halo = g.createRadialGradient(0, 0, 0, 0, 0, cx)
  halo.addColorStop(0, 'rgba(255,255,255,0.98)')
  halo.addColorStop(0.12, rgba(color, 0.9))
  halo.addColorStop(0.38, rgba(color, 0.22))
  halo.addColorStop(1, rgba(color, 0))
  g.beginPath()
  g.arc(0, 0, cx, 0, Math.PI * 2)
  g.fillStyle = halo
  g.fill()

  if (withBeams) {
    // 十字光芒：水平 + 垂直各一道极扁的径向渐变；lighter 让交叠处更亮
    g.globalCompositeOperation = 'lighter'
    for (const rot of [0, Math.PI / 2]) {
      g.save()
      g.rotate(rot)
      g.scale(1, 0.05)
      const beam = g.createRadialGradient(0, 0, 0, 0, 0, cx)
      beam.addColorStop(0, 'rgba(255,255,255,0.95)')
      beam.addColorStop(0.3, rgba(color, 0.5))
      beam.addColorStop(0.7, rgba(color, 0.12))
      beam.addColorStop(1, rgba(color, 0))
      g.beginPath()
      g.arc(0, 0, cx, 0, Math.PI * 2)
      g.fillStyle = beam
      g.fill()
      g.restore()
    }
    g.globalCompositeOperation = 'source-over'
  }
  return c
}

/** 取（或建）当前主色对应的精灵。主色变了才重建。 */
function getSprites(color: [number, number, number]) {
  const key = color.join(',')
  if (spriteCache?.key === key) return spriteCache
  const big = makeSprite(96, color, true) // 大星：带十字星芒
  const small = makeSprite(48, color, false) // 小星：只要光晕（星芒在小尺寸上会糊成一团）
  spriteCache = { key, big, small }
  return spriteCache
}

// ── 星点 ────────────────────────────────────────────────────────────────
interface Star {
  x: number
  y: number
  r: number // 半径（px）
  base: number // 基础亮度 0..1
  twinkle: number // 闪烁速率
  phase: number // 闪烁相位（避免同步呼吸）
  beam: boolean // 是否用带星芒的精灵
}

let stars: Star[] = []

function buildStars(w: number, h: number) {
  const rng = makeRng(0x5eed) // 固定种子 → 每次加载星图一致
  // 密度：约每 11000px² 一颗，并设上限（纯装饰，不能拖累页面）
  const count = Math.min(240, Math.round((w * h) / 11000))
  stars = []
  for (let i = 0; i < count; i++) {
    const t = rng()
    // 大小分档：多数是小星，约 14% 是有星芒的亮星（层次感来自这个比例）
    const beam = t > 0.86
    const r = beam ? 9 + rng() * 9 : 1.1 + rng() * 2.2
    stars.push({
      x: rng() * w,
      y: rng() * h,
      r,
      base: beam ? 0.75 + rng() * 0.25 : 0.22 + rng() * 0.5,
      twinkle: 0.4 + rng() * 1.1,
      phase: rng() * Math.PI * 2,
      beam,
    })
  }
}

// ── 流星 ────────────────────────────────────────────────────────────────
interface Meteor {
  x: number
  y: number
  vx: number
  vy: number
  life: number
  maxLife: number
  len: number
  width: number
  bright: number
}

let meteors: Meteor[] = []
let nextMeteorAt = 0

// 频次：从 2.6~7s 提到 1.1~3.0s（用户要求"频繁点儿"）。
// ★但不能只调间隔：一颗流星存活约 1.4~2s，间隔缩短后必然出现重叠。
//   故同时给**并发数**设上限（见 spawnMeteor 里的 MAX_ALIVE），
//   否则会变成"流星雨"，丢掉"偶尔划过头顶"的氛围。
const METEOR_MIN_GAP = 1100
const METEOR_MAX_GAP = 3000
/** 同屏最多几颗 —— 频次提高后靠这个守住"克制" */
const METEOR_MAX_ALIVE = 2

function spawnMeteor(w: number, h: number, now: number) {
  // 同屏上限：满了就跳过本次（把下一次排到稍后），避免重叠成"流星雨"
  if (meteors.length >= METEOR_MAX_ALIVE) {
    nextMeteorAt = now + METEOR_MIN_GAP * 0.6
    return
  }
  // 固定方向：约 28° 斜向右下（比 45° 更"掠过头顶"的观感）
  const ang = Math.PI * 0.155
  // 速度随拖尾一起提：长尾巴配慢速会显得"飘"，扫快一点才像流星
  const speed = 0.75 + Math.random() * 0.45 // px/ms
  meteors.push({
    // 起点偏左上，让轨迹从画面外进入
    x: -w * 0.1 + Math.random() * w * 0.85,
    y: -h * 0.05 + Math.random() * h * 0.32,
    vx: Math.cos(ang) * speed,
    vy: Math.sin(ang) * speed,
    life: 0,
    // 存活时间同步拉长，让长尾有足够时间划过（否则尾巴刚显形就回收了）
    maxLife: 1200 + Math.random() * 700,
    // 拖尾长度：从 120~250px 提到 260~470px（用户要求"再长点儿"）
    len: 260 + Math.random() * 210,
    width: 1.5 + Math.random() * 1.3,
    bright: 0.8 + Math.random() * 0.2,
  })
  nextMeteorAt = now + METEOR_MIN_GAP + Math.random() * (METEOR_MAX_GAP - METEOR_MIN_GAP)
}

function drawMeteor(g: CanvasRenderingContext2D, m: Meteor, color: [number, number, number]) {
  // ★ 拖尾长度必须是**像素**：早先把 len 当成速度比例去除，结果尾巴只有 1~4px
  //   （等于只是一个点，屏幕上看不见流星）。这里按方向单位向量 × 像素长度算。
  const mag = Math.hypot(m.vx, m.vy) || 1
  const tailX = m.x - (m.vx / mag) * m.len
  const tailY = m.y - (m.vy / mag) * m.len
  const grad = g.createLinearGradient(m.x, m.y, tailX, tailY)
  // 头亮 → 尾透明；中段给主色，让轨迹带品牌色而不是纯白噪声
  grad.addColorStop(0, rgba([255, 255, 255], 1))
  grad.addColorStop(0.12, rgba([255, 255, 255], 0.75 * m.bright))
  grad.addColorStop(0.42, rgba(color, 0.45 * m.bright))
  grad.addColorStop(1, rgba(color, 0))
  g.strokeStyle = grad
  g.lineWidth = m.width
  g.lineCap = 'round'
  g.beginPath()
  g.moveTo(m.x, m.y)
  g.lineTo(tailX, tailY)
  g.stroke()
  // 头部亮点（略放大，让"流星头"醒目）
  g.fillStyle = rgba([255, 255, 255], 0.95 * m.bright)
  g.beginPath()
  g.arc(m.x, m.y, m.width * 1.4, 0, Math.PI * 2)
  g.fill()
}

// ── 主循环 ──────────────────────────────────────────────────────────────
let lastT = 0
let curSize = { w: 0, h: 0 }

function renderFrame(now: number) {
  const el = canvas.value
  if (!el) return
  const g = el.getContext('2d')
  if (!g) return

  const { w, h } = curSize
  if (!w || !h) return

  const cs = getComputedStyle(document.documentElement)
  const primary = cs.getPropertyValue('--neo-primary').trim() || '#a78bfa'
  const color = toRgb(primary)
  const strength = Number(cs.getPropertyValue('--neo-star-alpha').trim() || '1') || 1
  const { big, small } = getSprites(color)

  g.clearRect(0, 0, w, h)
  // 光叠加：星芒与光晕叠在一起更自然（亮色主题下强度已被压低，不会过曝）
  g.globalCompositeOperation = 'lighter'

  const t = now / 1000
  for (const s of stars) {
    // 闪烁：亮度呼吸 ±28%。相位各异 → 不会整屏一起明灭（那会很假）。
    const k = 0.72 + 0.28 * Math.sin(t * s.twinkle + s.phase)
    const a = Math.min(1, s.base * k * strength)
    if (a <= 0.01) continue
    g.globalAlpha = a
    const sprite = s.beam ? big : small
    const d = s.r * (s.beam ? 2 : 2)
    g.drawImage(sprite, s.x - d / 2, s.y - d / 2, d, d)
  }

  g.globalAlpha = 1
  for (const m of meteors) drawMeteor(g, m, color)
  g.globalCompositeOperation = 'source-over'
}

function loop(now: number) {
  const dt = lastT ? Math.min(64, now - lastT) : 16 // 封顶：切回标签页时不要"跳帧"
  lastT = now

  const { w, h } = curSize
  if (w && h) {
    if (now >= nextMeteorAt) spawnMeteor(w, h, now)
    // 推进流星；出画或超龄即回收
    meteors = meteors.filter((m) => {
      m.life += dt
      m.x += m.vx * dt
      m.y += m.vy * dt
      return m.life < m.maxLife && m.y < h + 80 && m.x < w + 120
    })
    renderFrame(now)
  }
  raf = requestAnimationFrame(loop)
}

// ── 尺寸 / 生命周期 ─────────────────────────────────────────────────────
const reduceMotion = () =>
  typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches

function resize() {
  const el = canvas.value
  if (!el) return
  const dpr = Math.min(2, window.devicePixelRatio || 1) // 封顶 2：3x 屏上没必要
  const w = el.clientWidth
  const h = el.clientHeight
  if (!w || !h) return
  curSize = { w, h }
  el.width = Math.floor(w * dpr)
  el.height = Math.floor(h * dpr)
  const g = el.getContext('2d')
  if (g) g.setTransform(dpr, 0, 0, dpr, 0, 0)
  buildStars(w, h)
  // 精灵按主色缓存（getSprites 内部判断），尺寸/DPR 变化只影响绘制缩放，
  // 不需要重建 —— 所以这里不手动清缓存。
  renderFrame(performance.now())
}

function start() {
  stop()
  if (props.kind !== 'stars') return
  resize()
  if (reduceMotion()) {
    // 降级：只留一帧静态星场（仍好看，但不闪、不飞流星）
    renderFrame(performance.now())
    return
  }
  lastT = 0
  nextMeteorAt = performance.now() + 600 // 首次很快出现，让访客立刻知道有流星
  raf = requestAnimationFrame(loop)
}
function stop() {
  if (raf) cancelAnimationFrame(raf)
  raf = 0
  meteors = []
}

function onVisibility() {
  if (props.kind !== 'stars') return
  if (document.hidden) stop()
  else start()
}

onMounted(() => {
  start()
  window.addEventListener('resize', resize)
  window.addEventListener('neo-theme-changed', resize)
  document.addEventListener('visibilitychange', onVisibility)
})
onBeforeUnmount(() => {
  stop()
  window.removeEventListener('resize', resize)
  window.removeEventListener('neo-theme-changed', resize)
  document.removeEventListener('visibilitychange', onVisibility)
})
watch(() => props.kind, start)
</script>

<template>
  <div class="backdrop" aria-hidden="true">
    <canvas v-show="kind === 'stars'" ref="canvas" class="stars" />
    <!-- 规律纹理走 CSS：锐利、精确、零 JS。
         size 与线宽决定覆盖率（约 3%~7%），是实测过的值。 -->
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
