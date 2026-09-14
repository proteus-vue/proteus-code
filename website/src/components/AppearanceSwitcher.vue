<!-- src/components/AppearanceSwitcher.vue —— 外观切换器（站点自己就是最好的演示）
     ★ 这个组件是"官网即产品"的关键：TUI 里 `ctrl+t` 切主题、`/background` 切纹理，
     官网把这两件事**真的做出来**。访客点几下就理解了「风格包 vs 换色」的区别
     （切到 Terminal 会连圆角一起变直角），这比任何文案都有说服力。
     同时它也是**可访问性**的：亮色主题（Light）本身就是一个可选项，不用系统偏好也能拿。 -->
<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { THEME_ORDER, THEMES, BG_ORDER, BG_LABEL, type ThemeName } from '../theme'
import { useTheme } from '../composables/useTheme'

const { theme, bg, setTheme, setBg, cycleTheme } = useTheme()
const open = ref(false)

// 键盘快捷键对齐 TUI：ctrl+t 循环切主题
function onKey(e: KeyboardEvent) {
  if (e.ctrlKey && (e.key === 't' || e.key === 'T')) {
    e.preventDefault()
    cycleTheme()
  }
  if (e.key === 'Escape') open.value = false
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

function pick(t: ThemeName) {
  setTheme(t)
  // 不自动关闭面板：让用户连着试几套（切主题是"试"出来的）
}
</script>

<template>
  <div class="switcher">
    <button
      class="trigger"
      type="button"
      :aria-expanded="open"
      aria-controls="appearance-panel"
      @click="open = !open"
    >
      <!-- 当前主题的色点：不用文字也能看出正在用哪套 -->
      <span class="chip" :style="{ background: THEMES[theme].primary }" aria-hidden="true" />
      <span class="label">{{ THEMES[theme].label }}</span>
      <span class="caret" aria-hidden="true">{{ open ? '▴' : '▾' }}</span>
    </button>

    <div v-if="open" id="appearance-panel" class="panel" role="dialog" aria-label="外观设置">
      <p class="head">
        <span class="sigil" aria-hidden="true">◆</span> 主题
        <kbd>ctrl+t</kbd>
      </p>
      <ul class="themes">
        <li v-for="t in THEME_ORDER" :key="t">
          <button
            type="button"
            class="theme"
            :class="{ on: t === theme }"
            :title="THEMES[t].note"
            @click="pick(t)"
          >
            <span class="swatch" aria-hidden="true">
              <i :style="{ background: THEMES[t].backdrop }" />
              <i :style="{ background: THEMES[t].primary }" />
              <i :style="{ background: THEMES[t].accent }" />
            </span>
            <span class="tname">{{ THEMES[t].label }}</span>
            <span class="mark" aria-hidden="true">{{ t === theme ? '●' : '○' }}</span>
          </button>
        </li>
      </ul>

      <p class="head second">
        <span class="sigil" aria-hidden="true">◆</span> 背景
      </p>
      <div class="bgs">
        <button
          v-for="b in BG_ORDER"
          :key="b"
          type="button"
          class="bg"
          :class="{ on: b === bg }"
          @click="setBg(b)"
        >
          {{ BG_LABEL[b] }}
        </button>
      </div>

      <p class="foot">与 TUI 的 <kbd>ctrl+t</kbd> / <kbd>/background</kbd> 同源</p>
    </div>
  </div>
</template>

<style scoped>
.switcher {
  position: relative;
}

.trigger {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.42rem 0.8rem;
  font: inherit;
  font-size: var(--fs-xs);
  color: var(--neo-fg);
  background: var(--neo-element);
  border: 1px solid var(--neo-border-active);
  border-radius: var(--neo-radius-sm);
  cursor: pointer;
  transition: border-color 0.16s ease, background 0.16s ease;
}
.trigger:hover {
  border-color: var(--neo-primary);
}
.chip {
  width: 0.6rem;
  height: 0.6rem;
  border-radius: 50%;
}
.caret {
  color: var(--neo-fg-faint);
}

/* 面板：TUI 的模态就是「底色 + 边」，这里保持同一语言 */
.panel {
  position: absolute;
  top: calc(100% + 0.5rem);
  right: 0;
  z-index: 60;
  width: 16.5rem;
  padding: 0.9rem 1rem;
  background: var(--neo-panel);
  border: 1px solid var(--neo-border-active);
  border-radius: var(--neo-radius);
  box-shadow: 0 18px 50px -14px rgba(0, 0, 0, 0.6);
}

.head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0 0 0.6rem;
  font-size: 0.72rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--neo-fg-dim);
}
.head kbd {
  margin-left: auto;
  font-family: var(--neo-mono);
  font-size: 0.68rem;
  color: var(--neo-primary);
  text-transform: none;
  letter-spacing: 0;
}
.head.second {
  margin-top: 0.9rem;
}
.sigil {
  color: var(--neo-primary);
}

.themes {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  max-height: 15.5rem;
  overflow-y: auto;
}
.theme {
  display: flex;
  align-items: center;
  gap: 0.55rem;
  width: 100%;
  padding: 0.34rem 0.45rem;
  font: inherit;
  font-size: var(--fs-xs);
  color: var(--neo-fg-dim);
  background: transparent;
  border: none;
  border-radius: var(--neo-radius-sm);
  cursor: pointer;
  text-align: left;
}
.theme:hover {
  background: color-mix(in srgb, var(--neo-element) 80%, transparent);
  color: var(--neo-fg);
}
/* 选中态：整块填充（对齐 TUI 选项条的 bg_selected + ● 单选标记） */
.theme.on {
  background: var(--neo-selected);
  color: var(--neo-fg);
  font-weight: 600;
}

.swatch {
  display: inline-flex;
  flex: 0 0 auto;
}
.swatch i {
  width: 0.55rem;
  height: 0.9rem;
  border: 1px solid var(--neo-border);
}
.swatch i:first-child {
  border-radius: 3px 0 0 3px;
}
.swatch i:last-child {
  border-radius: 0 3px 3px 0;
}

.tname {
  flex: 1 1 auto;
}
.mark {
  color: var(--neo-primary);
}

.bgs {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 0.3rem;
}
.bg {
  padding: 0.32rem 0.2rem;
  font: inherit;
  font-size: 0.7rem;
  color: var(--neo-fg-dim);
  background: transparent;
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius-sm);
  cursor: pointer;
}
.bg:hover {
  border-color: var(--neo-primary);
  color: var(--neo-fg);
}
.bg.on {
  background: var(--neo-selected);
  border-color: var(--neo-primary);
  color: var(--neo-fg);
}

.foot {
  margin: 0.85rem 0 0;
  padding-top: 0.7rem;
  border-top: 1px solid var(--neo-border);
  font-size: 0.68rem;
  color: var(--neo-fg-faint);
}
.foot kbd {
  font-family: var(--neo-mono);
  color: var(--neo-primary);
}
</style>
