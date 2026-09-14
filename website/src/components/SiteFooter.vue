<!-- src/components/SiteFooter.vue —— 页脚
     对标同行：页脚要有**完整目录**（移动端导航会收起，这里是唯一入口）
     与一句技术栈自述（既是信息也是 dogfooding 的展示）。 -->
<script setup lang="ts">
const REPO = 'https://github.com/proteus-vue/proteus-code'
const year = new Date().getFullYear()

const nav = [
  { href: '#hosts', text: '四宿主' },
  { href: '#features', text: '特性' },
  { href: '#arch', text: '架构' },
  { href: '#why', text: '对比' },
  { href: '#install', text: '安装' },
  { href: '#boundaries', text: '诚实边界' },
]

const ext = [
  { href: REPO, text: '源码仓库' },
  { href: `${REPO}/blob/main/docs/RELEASE.md`, text: '发布说明' },
  { href: 'https://github.com/proteus-vue/proteus', text: 'Proteus 框架' },
  { href: `${REPO}/blob/main/LICENSE`, text: 'MIT 许可' },
]
</script>

<template>
  <footer class="foot">
    <div class="wrap inner">
      <div class="brand-col">
        <div class="brand">
          <span class="mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
              stroke-linejoin="round">
              <path d="M12 3l9 9-9 9-9-9 9-9z" />
              <circle cx="12" cy="12" r="2.4" fill="currentColor" stroke="none" />
            </svg>
          </span>
          <span>neo</span>
        </div>
        <p class="tag">
          用 Rust 重写的编程 Agent 内核。<br />一套内核，四个宿主。
        </p>
      </div>

      <nav class="col">
        <h4>本页</h4>
        <a v-for="l in nav" :key="l.href" :href="l.href">{{ l.text }}</a>
      </nav>

      <nav class="col">
        <h4>链接</h4>
        <a v-for="l in ext" :key="l.href" :href="l.href" target="_blank" rel="noopener">
          {{ l.text }}
        </a>
      </nav>
    </div>

    <div class="wrap bottom">
      <p>© {{ year }} Proteus Contributors · MIT</p>
      <p class="built">
        本站用
        <a href="https://github.com/proteus-vue/proteus" target="_blank" rel="noopener">Proteus</a>
        框架构建 —— 同组织的 Vue 跨端框架，不是普通 Vite 模板
      </p>
    </div>
  </footer>
</template>

<style scoped>
.foot {
  margin-top: 2rem;
  padding-top: 3.5rem;
  border-top: 1px solid var(--neo-border);
  background: linear-gradient(180deg, transparent, color-mix(in srgb, var(--neo-panel) 55%, transparent));
}

.inner {
  display: grid;
  grid-template-columns: 1.4fr 1fr 1fr;
  gap: 2.5rem;
  padding-bottom: 3rem;
}

.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.55rem;
  font-size: 1.1rem;
  font-weight: 700;
}
.mark {
  display: inline-flex;
  color: var(--neo-primary);
}
.mark svg {
  width: 1.4rem;
  height: 1.4rem;
}
.tag {
  margin: 0.9rem 0 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
  line-height: 1.7;
}

.col {
  display: flex;
  flex-direction: column;
  gap: 0.55rem;
}
.col h4 {
  margin: 0 0 0.35rem;
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--neo-fg-faint);
}
.col a {
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
}
.col a:hover {
  color: var(--neo-primary);
}

.bottom {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  flex-wrap: wrap;
  padding-top: 1.5rem;
  padding-bottom: 2.5rem;
  border-top: 1px solid var(--neo-border);
  font-size: var(--fs-xs);
  color: var(--neo-fg-faint);
}
.bottom p {
  margin: 0;
}
.built a {
  color: var(--neo-fg-dim);
}

@media (max-width: 780px) {
  .inner {
    grid-template-columns: 1fr 1fr;
    gap: 2rem;
  }
  .brand-col {
    grid-column: 1 / -1;
  }
}
</style>
