<!-- src/components/SectionHosts.vue —— 五宿主
     ★ 这一段的关键不是"列出五个名字"，而是说清**为什么五个宿主不是五份实现**：
     它们共享同一内核与同一事件流（T6 等价铁律，机器断言）。 -->
<script setup lang="ts">
import TuiKeyHints from './TuiKeyHints.vue'
const hosts = [
  {
    name: 'TUI',
    cmd: 'neo',
    desc: '真终端交互：7 套主题、鼠标、滚动搜索、审批前 diff 预览、which-key 键位提示',
    primary: true,
  },
  {
    name: 'Desktop',
    cmd: 'neo desktop',
    desc: '系统 webview 窗口（WKWebView / WebView2 / WebKitGTK），不捆绑 Chromium',
  },
  {
    name: 'Web',
    cmd: 'neo serve',
    desc: '浏览器界面 + SSE 实时事件流，页面内交互审批',
  },
  {
    name: 'Exec',
    cmd: 'neo exec',
    desc: '无头 / CI 场景，支持 --json 便于脚本消费与断言',
  },
  {
    name: 'app-server',
    cmd: 'neo app-server',
    desc: 'stdio 上的 JSON-RPC 2.0：编辑器 / IDE / 脚本接入，17 个方法覆盖全部 Op',
  },
]
</script>

<template>
  <section id="hosts" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">多宿主</span>
        <h2>五宿主，一个内核</h2>
        <p>
          宿主只是内核的消费者，不持有业务状态、不自行解析事件。同一条事件流喂给五个宿主，
          事实完全等价 —— 这条称为 <strong>T6 铁律</strong>，由机器断言，不靠约定。
        </p>
      </div>

      <div class="grid">
        <article v-for="h in hosts" :key="h.name" class="card host reveal">
          <header>
            <h3>{{ h.name }}</h3>
            <span v-if="h.primary" class="tag">主角</span>
          </header>
          <code class="cmd">{{ h.cmd }}</code>
          <p>{{ h.desc }}</p>
        </article>
      </div>

      <TuiKeyHints
        class="krow"
        :hints="[
          { key: 'neo', text: 'TUI' },
          { key: 'neo desktop', text: '系统 webview' },
          { key: 'neo serve', text: '浏览器' },
          { key: 'neo exec', text: '无头 / CI' },
          { key: 'neo app-server', text: 'stdio JSON-RPC' },
        ]"
        tail="同一事件流 → 五宿主事实等价"
      />
    </div>
  </section>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
  gap: 1rem;
}

.host header {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}
.host h3 {
  font-size: var(--fs-h3);
}

.tag {
  padding: 0.1rem 0.5rem;
  font-size: 0.68rem;
  color: var(--neo-accent);
  background: color-mix(in srgb, var(--neo-accent) 14%, transparent);
  border: 1px solid color-mix(in srgb, var(--neo-accent) 40%, transparent);
  border-radius: var(--neo-radius-pill);
}

.cmd {
  display: block;
  margin: 0.1rem 0 0.75rem;
  font-size: var(--fs-xs);
  color: var(--neo-primary);
}

.host p {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
  line-height: 1.65;
}
</style>
