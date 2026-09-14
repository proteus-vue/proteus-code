<!-- src/components/TerminalDemo.vue —— 终端演示（静态，但按真实 TUI 布局还原）
     ★ 这是**示意**，不是录屏：作为静态块更好控、不会随版本失效。
       理由：TUI 显式要求真终端（stdin 必须是 tty，管道下直接报错），
       录屏需要额外 pty 工具链。
     ★ 内容全部取自**离线桩 provider 的实跑输出**（无需 API key，可复现）：

         $ neo exec "写一个文件" --provider selftest --mode auto-edit --allow-writes
         [neo] 模式   AutoEdit（沙箱 WorkspaceWrite / 审批 OnRequest / 文件编辑 Auto）
         [neo] 沙箱   已启用（OS 级强制）
         [turn] 开始
         > 写一个文件
         [tool] apply_patch (selftest-apply_patch)
         [files] 1 个文件已改（+1 -0）
         [tool] selftest-apply_patch → exit 0
         已写入 /workspace/selftest.txt（48 字节）
         [turn] 完成（in 0 / out 0 tokens）

       diff 内容也是真的：桩 provider 实际写入的文件正文是
       「由 selftest provider 经 apply_patch 写入。」（48 字节）

     布局照搬 TUI 主界面：左侧会话流（`┃` 面板条）→ 右侧侧栏（Context / Todo /
     Modified Files，README「亲测可用」里列的真实分区）→ 底部输入框与状态栏。
     这样展示的是"这个产品长什么样"，而不只是"它输出过什么文本"。 -->
<script setup lang="ts">
/** 会话流的一行；kind 决定配色与缩进 */
interface Line {
  kind?: 'cmd' | 'info' | 'tool' | 'ok' | 'dim' | 'plain' | 'user' | 'diff-add'
  text: string
}

withDefaults(
  defineProps<{
    title: string
    lines: Line[]
    /** 侧栏数据（可选）。不传则不渲染侧栏 —— 窄屏会自动隐藏。 */
    sidebar?: {
      context: { used: string; total: string; percent: number }
      todos: { text: string; done: boolean }[]
      files: { path: string; add: number; del: number }[]
    }
    /** 底部输入框内容 */
    prompt?: string
    /** 底部状态栏（左 / 右） */
    status?: { left: string; right: string }
  }>(),
  { prompt: '', status: undefined, sidebar: undefined },
)
</script>

<template>
  <figure class="term">
    <figcaption class="bar">
      <span class="dots" aria-hidden="true"><i /><i /><i /></span>
      <span class="title">{{ title }}</span>
      <span class="badge">静态示意</span>
    </figcaption>

    <div class="screen">
      <!-- 会话流（TUI 主体） -->
      <div class="stream">
        <pre class="body"><code><span
            v-for="(l, i) in lines"
            :key="i"
            class="line"
            :class="l.kind || 'plain'"
          >{{ l.text }}
</span></code></pre>
      </div>

      <!-- 右侧侧栏：对齐 TUI 的三段（占用 / 待办 / 改动文件） -->
      <aside v-if="sidebar" class="side">
        <section class="sblock">
          <h4>Context</h4>
          <div class="meter" :title="`${sidebar.context.used} / ${sidebar.context.total}`">
            <span :style="{ width: sidebar.context.percent + '%' }" />
          </div>
          <p class="smeta">{{ sidebar.context.used }} / {{ sidebar.context.total }}</p>
        </section>

        <section class="sblock">
          <h4>Todo</h4>
          <ul class="todos">
            <li v-for="t in sidebar.todos" :key="t.text" :class="{ done: t.done }">
              <span class="mark" aria-hidden="true">{{ t.done ? '✓' : '○' }}</span>
              <span>{{ t.text }}</span>
            </li>
          </ul>
        </section>

        <section class="sblock">
          <h4>Modified Files</h4>
          <ul class="files">
            <li v-for="f in sidebar.files" :key="f.path">
              <span class="fpath">{{ f.path }}</span>
              <span class="fstat"><i class="add">+{{ f.add }}</i> <i class="del">-{{ f.del }}</i></span>
            </li>
          </ul>
        </section>
      </aside>
    </div>

    <!-- 底部输入框：`▌` 左强调条 + 模式 ⏵ 模型（与 TUI 同构） -->
    <div v-if="prompt" class="prompt">
      <span class="caret" aria-hidden="true">▌</span>
      <span class="ptext">{{ prompt }}</span>
      <span class="pcursor" aria-hidden="true" />
    </div>

    <!-- 底部状态栏：左路径 / 右状态（TUI 最底行） -->
    <div v-if="status" class="sbar">
      <span class="sleft">{{ status.left }}</span>
      <span class="sright">{{ status.right }}</span>
    </div>

    <slot />
  </figure>
</template>

<style scoped>
.term {
  position: relative;
  margin: 0;
  /* 颜色全部走主题变量：亮色主题下自动变亮底，不会成为一块"黑砖" */
  background: linear-gradient(
    180deg,
    var(--neo-panel),
    color-mix(in srgb, var(--neo-backdrop) 60%, var(--neo-panel))
  );
  border: 1px solid var(--neo-border-active);
  border-radius: var(--neo-radius);
  overflow: hidden;
  /* 深色底上阴影不可见，用同色系光晕；亮色主题由 --neo-glow-strength 压到近零 */
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--neo-primary) 8%, transparent),
    0 24px 70px -20px
      color-mix(in srgb, var(--neo-primary) calc(var(--neo-glow-strength) * 100%), transparent);
}

/* 顶部一道高光边（同行截图常见，让"窗口"有玻璃感） */
.term::before {
  content: "";
  position: absolute;
  inset: 0 0 auto;
  height: 1px;
  background: linear-gradient(
    90deg,
    transparent,
    color-mix(in srgb, var(--neo-primary) 60%, transparent),
    transparent
  );
}

.bar {
  display: flex;
  align-items: center;
  gap: 0.7rem;
  padding: 0.66rem 0.95rem;
  background: color-mix(in srgb, var(--neo-element) 70%, transparent);
  border-bottom: 1px solid var(--neo-border);
}

.dots {
  display: inline-flex;
  gap: 0.4rem;
}
.dots i {
  width: 0.66rem;
  height: 0.66rem;
  border-radius: 50%;
  background: var(--neo-border-active);
}
.dots i:first-child {
  background: #4a3a52;
}

.title {
  flex: 1 1 auto;
  min-width: 0;
  font-family: var(--neo-mono);
  font-size: var(--fs-xs);
  color: var(--neo-fg-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.badge {
  flex: 0 0 auto;
  padding: 0.14rem 0.55rem;
  font-size: 0.68rem;
  color: var(--neo-warning);
  border: 1px solid color-mix(in srgb, var(--neo-warning) 55%, transparent);
  border-radius: var(--neo-radius-pill);
  opacity: 0.9;
}

/* 主体：会话流 + 侧栏两栏（与 TUI 主界面同构） */
.screen {
  display: flex;
  align-items: stretch;
}

.stream {
  flex: 1 1 auto;
  min-width: 0;
  /* 左侧 `┃` 面板条：TUI 全站的分组惯例（不画四角框） */
  border-left: 3px solid color-mix(in srgb, var(--neo-primary) 70%, transparent);
}

.body {
  margin: 0;
  padding: 0.85rem 1rem;
  overflow-x: auto;
  font-size: 0.78rem;
  line-height: 1.62;
}

.line {
  display: block;
  white-space: pre;
}
.line.cmd {
  color: var(--neo-fg);
  font-weight: 600;
}
.line.info {
  color: var(--neo-info);
}
.line.tool {
  color: var(--neo-primary);
}
.line.ok {
  color: var(--neo-success);
}
.line.dim {
  color: var(--neo-fg-faint);
}
/* 用户输入行：前置 `>`（与 TUI 的会话流一致） */
.line.user {
  color: var(--neo-fg);
}
/* diff 新增行：带 + 号与成功色，暗示"真正落盘" */
.line.diff-add {
  color: var(--neo-success);
}

/* ── 侧栏 ─────────────────────────────────────────── */
.side {
  flex: 0 0 13.5rem;
  padding: 0.9rem 0.9rem 0.9rem 0;
  border-left: 1px solid var(--neo-border);
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
  font-size: 0.72rem;
}

.sblock h4 {
  margin: 0 0 0.4rem;
  font-size: 0.66rem;
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  /* 段标题用 fg-dim 而非 fg-faint：后者在暗底上偏弱（视觉验收指出
     MODIFIED FILES 贴近背景）。这是**分区标签**，要能一眼扫到。 */
  color: var(--neo-fg-dim);
}

/* 占用条：TUI 侧栏的 Context 条 */
.meter {
  height: 0.4rem;
  background: var(--neo-element);
  border: 1px solid var(--neo-border);
  border-radius: 999px;
  overflow: hidden;
}
.meter span {
  display: block;
  height: 100%;
  background: linear-gradient(90deg, var(--neo-primary), var(--neo-accent));
}
.smeta {
  margin: 0.3rem 0 0;
  font-family: var(--neo-mono);
  font-size: 0.66rem;
  color: var(--neo-fg-dim);
}

.todos,
.files {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.todos li {
  display: flex;
  gap: 0.4rem;
  color: var(--neo-fg-dim);
}
.todos .mark {
  color: var(--neo-primary);
}
.todos li.done .mark {
  color: var(--neo-success);
}
.todos li.done span:last-child {
  color: var(--neo-fg-faint);
  text-decoration: line-through;
}

.files li {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
}
.fpath {
  font-family: var(--neo-mono);
  color: var(--neo-fg-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fstat {
  flex: 0 0 auto;
  font-family: var(--neo-mono);
  font-style: normal;
}
.fstat .add {
  color: var(--neo-success);
  font-style: normal;
}
.fstat .del {
  color: var(--neo-error);
  font-style: normal;
}

/* ── 底部输入框 / 状态栏 ──────────────────────────── */
.prompt {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 1rem;
  border-top: 1px solid var(--neo-border);
  font-size: 0.78rem;
}
.caret {
  color: var(--neo-primary);
  font-family: var(--neo-mono);
}
.ptext {
  color: var(--neo-fg-dim);
}
/* 光标：一闪一闪的方块（TUI 的编辑态提示） */
.pcursor {
  width: 0.5rem;
  height: 1rem;
  background: var(--neo-primary);
  animation: blink 1.1s step-end infinite;
}
@keyframes blink {
  0%,
  50% {
    opacity: 1;
  }
  50.01%,
  100% {
    opacity: 0;
  }
}

.sbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.34rem 1rem;
  border-top: 1px solid var(--neo-border);
  background: color-mix(in srgb, var(--neo-element) 55%, transparent);
  font-family: var(--neo-mono);
  font-size: 0.68rem;
  color: var(--neo-fg-faint);
}
.sright {
  color: var(--neo-success);
}

@media (max-width: 760px) {
  /* 窄屏优先保证会话流可读：侧栏收起（内容是示意，不缺信息） */
  .side {
    display: none;
  }
}

@media (max-width: 640px) {
  .body {
    font-size: 0.7rem;
    line-height: 1.75;
  }
  .line {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .title {
    white-space: normal;
    overflow: visible;
    line-height: 1.35;
  }
  .bar {
    align-items: flex-start;
  }
}

@media (prefers-reduced-motion: reduce) {
  .pcursor {
    animation: none;
    opacity: 1;
  }
}
</style>
