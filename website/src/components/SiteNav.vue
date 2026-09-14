<!-- src/components/SiteNav.vue —— 顶部导航
     对标同行的两点：① 吸顶 + 滚动后加底色与描边（初始透明，滚动才"浮"起来）；
     ② 右侧放一个 GitHub 入口按钮，而不是纯文字链接。 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'

const REPO = 'https://github.com/proteus-vue/proteus-code'
const scrolled = ref(false)

function onScroll() {
  scrolled.value = window.scrollY > 12
}

onMounted(() => {
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })
})
onBeforeUnmount(() => window.removeEventListener('scroll', onScroll))

const links = [
  { href: '#hosts', text: '宿主' },
  { href: '#features', text: '特性' },
  { href: '#arch', text: '架构' },
  { href: '#why', text: '对比' },
  { href: '#install', text: '安装' },
]
</script>

<template>
  <header class="nav" :class="{ scrolled }">
    <div class="wrap bar">
      <a class="brand" href="#top">
        <span class="mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
            stroke-linejoin="round">
            <path d="M12 3l9 9-9 9-9-9 9-9z" />
            <circle cx="12" cy="12" r="2.4" fill="currentColor" stroke="none" />
          </svg>
        </span>
        <span class="name">neo</span>
      </a>

      <nav class="links">
        <a v-for="l in links" :key="l.href" :href="l.href">{{ l.text }}</a>
      </nav>

      <a class="gh" :href="REPO" target="_blank" rel="noopener">
        <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
          <path
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0016 8c0-4.42-3.58-8-8-8z" />
        </svg>
        <span>GitHub</span>
      </a>
    </div>
  </header>
</template>

<style scoped>
.nav {
  position: fixed;
  inset: 0 0 auto;
  z-index: 50;
  transition: background 0.25s ease, border-color 0.25s ease, backdrop-filter 0.25s ease;
  border-bottom: 1px solid transparent;
}
/* 初始透明，滚动后才有"浮层"感 —— 首屏因此不被一条横线切断 */
.nav.scrolled {
  background: color-mix(in srgb, var(--neo-backdrop) 82%, transparent);
  border-bottom-color: var(--neo-border);
  backdrop-filter: blur(12px);
}

.bar {
  display: flex;
  align-items: center;
  gap: 1.5rem;
  height: 4rem;
}

.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.55rem;
  color: var(--neo-fg);
  font-weight: 700;
  font-size: 1.12rem;
}
.mark {
  display: inline-flex;
  color: var(--neo-primary);
}
.mark svg {
  width: 1.5rem;
  height: 1.5rem;
}
.name {
  letter-spacing: -0.01em;
}

.links {
  display: flex;
  gap: 1.6rem;
  margin-left: auto;
  font-size: var(--fs-sm);
}
.links a {
  color: var(--neo-fg-dim);
  transition: color 0.18s ease;
}
.links a:hover {
  color: var(--neo-fg);
}

.gh {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.42rem 0.9rem;
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--neo-fg);
  background: var(--neo-element);
  border: 1px solid var(--neo-border-active);
  border-radius: var(--radius-sm);
  transition: border-color 0.18s ease, background 0.18s ease;
}
.gh:hover {
  color: var(--neo-fg);
  border-color: var(--neo-primary);
  background: var(--neo-selected);
}
.gh svg {
  width: 1rem;
  height: 1rem;
}

@media (max-width: 780px) {
  .links {
    display: none; /* 移动端收起为锚点导航（页脚里有完整目录） */
  }
  .gh span {
    display: none;
  }
  .gh {
    padding: 0.42rem 0.6rem;
    margin-left: auto;
  }
}
</style>
