<!-- src/components/HeroSection.vue —— 首屏 = 一帧真实的 TUI 首屏
     布局严格照搬 TUI 的首屏（crates/neo-host-tui/src/lib.rs 的主界面）：
       · 居中 ASCII 块字词标（LOGO_LARGE，6 行）
       · 词标下方一行副标题 + 版本/会话号
       · `▌` 左强调条的输入框（两行：提示 + 模式 ⏵ 模型）
       · 底部键位行（tab 补全 / ctrl+/ 键位提示 …）
       · 最底状态栏（左路径 / 右就绪）
     这就把"产品是什么"直接摆出来，而不是用一段宣传语去描述它。 -->
<script setup lang="ts">
import { computed } from 'vue'
import { LOGO_LARGE } from '../theme'
import { useTheme } from '../composables/useTheme'
import TerminalDemo from './TerminalDemo.vue'

const REPO = 'https://github.com/proteus-vue/proteus-code'
const { current } = useTheme()

// 真实执行输出（离线桩 selftest provider 实跑，无需 API key，可复现）
const demoLines = [
  { kind: 'cmd' as const, text: '$ neo exec "写一个文件" --provider selftest --mode auto-edit --allow-writes' },
  { kind: 'info' as const, text: '[neo] 模式   AutoEdit（沙箱 WorkspaceWrite / 审批 OnRequest / 文件编辑 Auto）' },
  { kind: 'info' as const, text: '[neo] 沙箱   已启用（OS 级强制）' },
  { kind: 'dim' as const, text: '' },
  { kind: 'plain' as const, text: '[turn] 开始' },
  { kind: 'plain' as const, text: '> 写一个文件' },
  { kind: 'tool' as const, text: '[tool] apply_patch (selftest-apply_patch)' },
  { kind: 'tool' as const, text: '[files] 1 个文件已改（+1 -0）' },
  { kind: 'ok' as const, text: '[tool] selftest-apply_patch → exit 0' },
  { kind: 'ok' as const, text: '已写入 /workspace/selftest.txt（48 字节）' },
  { kind: 'dim' as const, text: '[turn] 完成（in 0 / out 0 tokens）' },
]

// 词标逐行渐变（对齐 TUI 的 logo 渐变：主色 → 强调色）
const logoLines = computed(() => LOGO_LARGE)
</script>

<template>
  <section id="top" class="hero">
    <div class="wrap inner">
      <!-- 词标：与 TUI 首屏同一个 ASCII 块字，逐行做紫→品红渐变 -->
      <pre class="logo" aria-label="NEO"><span
          v-for="(line, i) in logoLines"
          :key="i"
          class="logo-line"
          :style="{ '--i': i / (logoLines.length - 1) }"
        >{{ line }}
</span></pre>

      <p class="tagline">—— 编程 Agent 内核</p>
      <p class="sub">Rust 内核 · TUI / Desktop / Web / Exec 共享同一内核</p>

      <p class="meta">
        <span class="v">v0.1.0</span>
        <span class="sep">·</span>
        <span>617 测试</span>
        <span class="sep">·</span>
        <span>零 unsafe</span>
        <span class="sep">·</span>
        <span>MIT</span>
      </p>

      <!-- 输入框：`▌` 左强调条 + 两行（提示 / 模式 ⏵ 模型），与 TUI 同构 -->
      <div class="prompt">
        <span class="caret" aria-hidden="true">▌</span>
        <div class="prompt-body">
          <p class="prompt-line">输入任务… 例：修一下代码里的 TODO</p>
          <p class="prompt-modes">
            <span class="mode">default</span>
            <span class="arrow">⏵</span>
            <span class="model">{{ current.label === 'Neo' ? 'mock' : 'mock' }}</span>
          </p>
        </div>
      </div>

      <!-- 键位行：对齐 TUI 底部 `tab 补全 ctrl+/ 键位提示 …` -->
      <p class="keys">
        <kbd>tab</kbd><span>补全</span>
        <kbd>ctrl+/</kbd><span>键位提示</span>
        <kbd>ctrl+t</kbd><span>换主题</span>
        <kbd>ctrl+c</kbd><span>退出</span>
      </p>

      <div class="cta">
        <a class="btn btn-primary" href="#install">
          立即安装
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8"
            stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M3 8h9M8.5 4.5L12 8l-3.5 3.5" />
          </svg>
        </a>
        <a class="btn" :href="REPO" target="_blank" rel="noopener">查看源码</a>
      </div>

      <p class="fine">无需 API key 也能试 —— 内置离线桩 provider。</p>

      <!-- 真实执行输出 -->
      <div class="stage">
        <TerminalDemo title="neo exec · 工具调用与真实落盘" :lines="demoLines" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.hero {
  position: relative;
  padding: 7.5rem 0 0;
}
.inner {
  text-align: center;
}

/* ── ASCII 词标 ─────────────────────────────────── */
.logo {
  margin: 0;
  font-family: var(--neo-mono);
  font-size: clamp(0.42rem, 1.62vw, 1.05rem);
  line-height: 1.14;
  letter-spacing: 0;
  white-space: pre;
  overflow-x: hidden;
  /* 逐行渐变：每行一个色相步进，视觉上像 TUI 的 logo 渐变 */
  color: var(--neo-primary);
}
.logo-line {
  display: block;
  background: linear-gradient(
    100deg,
    var(--neo-primary) calc(var(--i) * 40%),
    var(--neo-accent) calc(var(--i) * 40% + 60%)
  );
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  /* 磷绿等主题下加一点字幕辉光，模拟 CRT */
  filter: drop-shadow(0 0 18px color-mix(in srgb, var(--neo-primary) 32%, transparent));
}

.tagline {
  margin: 1.4rem 0 0;
  font-size: 1.05rem;
  color: var(--neo-fg);
  letter-spacing: 0.02em;
}
.sub {
  margin: 0.5rem 0 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
}

.meta {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  align-items: center;
  gap: 0.5rem;
  margin: 1.1rem 0 0;
  font-family: var(--neo-mono);
  font-size: var(--fs-xs);
  color: var(--neo-fg-faint);
}
.meta .v {
  color: var(--neo-primary);
}
.meta .sep {
  color: var(--neo-border-active);
}

/* ── 输入框（对齐 TUI 的 ▌ 左强调条）─────────────── */
.prompt {
  display: flex;
  gap: 0.7rem;
  max-width: 40rem;
  margin: 2.2rem auto 0;
  padding: 0.85rem 1.05rem;
  text-align: left;
  background: color-mix(in srgb, var(--neo-element) 66%, transparent);
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius);
}
.caret {
  flex: 0 0 auto;
  color: var(--neo-primary);
  font-family: var(--neo-mono);
}
.prompt-body {
  min-width: 0;
}
.prompt-line {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
}
.prompt-modes {
  display: flex;
  align-items: center;
  gap: 0.45rem;
  margin: 0.3rem 0 0;
  font-family: var(--neo-mono);
  font-size: var(--fs-xs);
}
.mode {
  color: var(--neo-fg-faint);
}
.arrow {
  color: var(--neo-primary);
}
.model {
  color: var(--neo-accent);
}

/* ── 键位行 ─────────────────────────────────────── */
.keys {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  align-items: center;
  gap: 0.35rem 0.7rem;
  max-width: 42rem;
  margin: 1.6rem auto 0;
  font-size: var(--fs-xs);
  color: var(--neo-fg-faint);
}
.keys kbd {
  font-family: var(--neo-mono);
  color: var(--neo-primary);
}

.cta {
  display: flex;
  gap: 0.8rem;
  justify-content: center;
  flex-wrap: wrap;
  margin-top: 2.2rem;
}
.cta .btn-primary svg {
  width: 1.05rem;
  height: 1.05rem;
}

.fine {
  margin: 1.1rem 0 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-faint);
}

.stage {
  max-width: 54rem;
  margin: clamp(2.8rem, 5vw, 4rem) auto 0;
  text-align: left;
}

@media (max-width: 700px) {
  .hero {
    padding-top: 6rem;
  }
  /* 词标在窄屏降到能完整放下 6 行的字号（不换行，块字换行就散了） */
  .logo {
    font-size: 0.44rem;
    line-height: 1.2;
  }
  .tagline {
    font-size: var(--fs-sm);
  }
  .stage {
    margin-top: 2.4rem;
  }
}
</style>
