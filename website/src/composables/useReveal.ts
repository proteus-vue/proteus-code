// src/composables/useReveal.ts —— 滚动揭示（IntersectionObserver，零依赖）
//
// 为什么不用 CSS-only 方案（如 animation-timeline: view()）：浏览器支持还不齐，
// 而这段逻辑只有十几行。也刻意**不**用 <Transition> 逐个组件包裹 —— 那是逐处重复。
//
// 渐进增强：JS 不可用时 .reveal 的初始 opacity:0 会让内容不可见，所以挂载时
// 立刻给 <html> 加 .js 标记，只有带该标记时才隐藏（见 global.css 的用法）。
import { onMounted, onBeforeUnmount } from 'vue'

export function useReveal() {
  let observer: IntersectionObserver | null = null

  onMounted(() => {
    const nodes = Array.from(document.querySelectorAll<HTMLElement>('.reveal'))
    if (!nodes.length) return

    // 不支持 IntersectionObserver 就直接显示（不藏在不可见里）
    if (typeof IntersectionObserver === 'undefined') {
      nodes.forEach((n) => n.classList.add('is-in'))
      return
    }

    observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue
          e.target.classList.add('is-in')
          observer?.unobserve(e.target) // 揭示一次即够，不做反复进出
        }
      },
      // 提前 12% 触发，滚动到位时已经揭示完，不会"追着看它出现"
      { rootMargin: '0px 0px -12% 0px', threshold: 0.05 },
    )
    nodes.forEach((n) => observer!.observe(n))
  })

  onBeforeUnmount(() => {
    observer?.disconnect()
    observer = null
  })
}
