// src/composables/useTheme.ts —— 主题 / 背景 / 词标的全局状态
//
// 对齐 TUI 的三层外观系统：主题（ctrl+t）· 背景纹理（/background）· 词标（/logo）。
// 状态放在模块级单例里（不是 provide/inject）：官网只有一处消费者
// （App 根 + 少量组件），单例比透传简单，且天然共享。
import { computed, ref } from 'vue'
import { THEMES, THEME_ORDER, BG_ORDER, type BgKind, type ThemeName } from '../theme'

const STORAGE_KEY = 'neo-site-appearance'

const theme = ref<ThemeName>('neo')
const bg = ref<BgKind>('stars')

const current = computed(() => THEMES[theme.value])

/** 把主题写成 CSS 变量挂到 <html>：所有组件只依赖变量，不各自持有主题知识 */
function apply() {
  const t = current.value
  const el = document.documentElement
  const map: Record<string, string> = {
    '--neo-primary': t.primary,
    '--neo-accent': t.accent,
    '--neo-success': t.success,
    '--neo-error': t.error,
    '--neo-warning': t.warning,
    '--neo-info': t.info,
    '--neo-fg': t.text,
    '--neo-fg-dim': t.muted,
    '--neo-border': t.border,
    '--neo-border-active': t.borderActive,
    '--neo-panel': t.bgPanel,
    '--neo-element': t.bgElement,
    '--neo-selected': t.bgSelected,
    '--neo-backdrop': t.backdrop,
  }
  for (const [k, v] of Object.entries(map)) el.style.setProperty(k, v)

  // 方角主题（Terminal 的 CRT 风格）：一个开关切换全部圆角 —— 这就是
  // 「风格包」与「换色」的区别，TUI 里也是同一个 square 标志驱动的。
  el.style.setProperty('--neo-radius', t.square ? '0px' : '14px')
  el.style.setProperty('--neo-radius-sm', t.square ? '0px' : '8px')
  el.dataset.neoSquare = t.square ? '1' : '0'
  el.dataset.neoLight = t.light ? '1' : '0'

  // 亮度相关的派生量：亮色主题下需要不同的强度（深色底上的点光是"星场"，
  // 亮色底上对比度天然更高，要压低，否则显得脏）。
  el.style.setProperty('--neo-glow-strength', t.light ? '0.06' : '0.3')
  el.style.setProperty('--neo-grid-alpha', t.light ? '0.1' : '0.055')
  // 纹理强度：这是 **draw 时会相乘的系数**（1 = 全强度）。深色主题用满，
  // 亮色主题压到约 4 成 —— 数值由截图实测确定，不是拍脑袋。
  el.style.setProperty('--neo-star-alpha', t.light ? '0.45' : '1')

  // 通知需要重绘的组件（canvas 背景的颜色来自 CSS 变量，改变量不会自动重画）。
  // 用事件而不是 provide/inject：背景组件与主题状态是弱耦合，一处派发一处监听最简单。
  window.dispatchEvent(new Event('neo-theme-changed'))
}

function persist() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme: theme.value, bg: bg.value }))
  } catch {
    /* 隐私模式等场景下静默失败：不影响使用 */
  }
}

function restore() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return
    const saved = JSON.parse(raw) as { theme?: ThemeName; bg?: BgKind }
    if (saved.theme && THEME_ORDER.includes(saved.theme)) theme.value = saved.theme
    if (saved.bg && BG_ORDER.includes(saved.bg)) bg.value = saved.bg
  } catch {
    /* 存的内容坏了就按默认走 */
  }
}

export function useTheme() {
  function setTheme(name: ThemeName) {
    theme.value = name
    apply()
    persist()
  }
  /** 循环切到下一套（对齐 TUI `ctrl+t`） */
  function cycleTheme() {
    const i = THEME_ORDER.indexOf(theme.value)
    setTheme(THEME_ORDER[(i + 1) % THEME_ORDER.length])
  }
  function setBg(kind: BgKind) {
    bg.value = kind
    // 用户显式选了纹理就覆盖主题默认（否则切主题会把选择冲掉）
    apply()
    persist()
  }
  function cycleBg() {
    const i = BG_ORDER.indexOf(bg.value)
    setBg(BG_ORDER[(i + 1) % BG_ORDER.length])
  }

  return { theme, bg, current, setTheme, cycleTheme, setBg, cycleBg, init: () => { restore(); apply() } }
}
