<!-- src/pages/index.vue —— 站点首页（单页 landing）
     内容原则（沿用本项目一贯风格）：
       · 只讲**已落地可验证**的能力，"未实现/边界"如实列出（第 6 区），不粉饰
       · 数字给出来源：23 crate / 617 测试 / 零 unsafe / 零 warning 都可在仓库门禁里复现 -->
<route>
{
  "meta": {
    "title": "neo —— 用 Rust 重写的编程 Agent 内核",
    "isTab": true
  }
}
</route>

<script setup lang="ts">
import CopyCommands from '../components/CopyCommands.vue'
import TerminalDemo from '../components/TerminalDemo.vue'

const REPO = 'https://github.com/proteus-vue/proteus-code'

const installs = [
  { label: '一键脚本', cmd: 'curl -fsSL https://raw.githubusercontent.com/proteus-vue/proteus-code/main/scripts/install.sh | sh' },
  { label: 'npm', cmd: 'npm install -g @proteus-vue/neo-code' },
  { label: 'cargo', cmd: `cargo install --git ${REPO} -p neo-code-cli --locked` },
]

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

const hosts = [
  { name: 'TUI', cmd: 'neo', desc: '真终端交互：7 套主题、鼠标、滚动搜索、审批 diff 预览、which-key 键位提示' },
  { name: 'Desktop', cmd: 'neo desktop', desc: '系统 webview 窗口（WKWebView / WebView2 / WebKitGTK），不捆绑 Chromium' },
  { name: 'Web', cmd: 'neo serve', desc: '浏览器界面 + SSE 实时事件流，页面内审批' },
  { name: 'Exec', cmd: 'neo exec', desc: '无头 / CI，支持 --json 便于脚本消费' },
]

const features = [
  {
    title: '真实 OS 级沙箱',
    body: 'macOS 用系统 Seatbelt 真机验证越权拦截（6 项测试）。沙箱是内核的结构保证——工具在类型层面就没有绕开沙箱的出口。',
  },
  {
    title: '沙箱 × 审批正交双轴',
    body: '沙箱决定"能做什么"，审批决定"何时必须问"。两者独立配置，不是同一个旋钮的两档。',
  },
  {
    title: '5 个有名 SPI',
    body: 'ModelProvider / SandboxBackend / SessionPersistence / HostBackend / Tool。每个都强制「契约 + ≥2 后端 + conformance」，门禁机器校验。',
  },
  {
    title: '离线也能全览',
    body: '三个内置桩 provider（mock / selftest / demo），没有 API key 也能把界面与内核交互走一遍。',
  },
  {
    title: '内存有界',
    body: 'Rust 只消除 UB，不保证有界。输出上限、UTF-8 安全截断、上下文上限都由内核强制，并有 6 项实测（计数分配器量化）。',
  },
  {
    title: '可回放的会话',
    body: 'JSONL append-only 会话日志，凡进入模型请求的内容都能从日志重建——回放与审计的前提。',
  },
]

const layers = [
  { id: 'L5', name: '宿主', detail: 'TUI / Desktop / Web / Exec', crates: 'neo-host-* · neo-exec · neo-code-cli' },
  { id: 'L4', name: '编排', detail: 'Goal 引擎（Plan → Code → Review → Learn）', crates: 'neo-orchestration' },
  { id: 'L3', name: '能力', detail: 'Shell-First 工具集 + 各 SPI 实现', crates: 'neo-capability · *-local · neo-mcp' },
  { id: 'L2', name: '内核', detail: 'turn/step 主循环 · 三维闸门 · SPI 契据', crates: 'neo-core' },
  { id: 'L1', name: '平台', detail: '命令包裹 · 进程加固 · fs notify', crates: 'neo-sandbox · neo-platform' },
  { id: 'L0', name: '协议', detail: 'Op / EventMsg / 双轴枚举（零业务依赖）', crates: 'neo-protocol' },
]

const boundaries = [
  { item: 'Linux / Windows 沙箱', note: '未实现。受限档位 fail-closed（拒绝执行），不会降级放行' },
  { item: 'Desktop 打包', note: '窗口层已机器验证；正式发布仍需图标、签名与公证' },
  { item: 'MCP 提示模板', note: 'stdio + Streamable HTTP 与 tools/resources 已就绪；prompts 未接入' },
  { item: 'Goal 挂钟停止条件', note: '四项确定性停止条件生效；挂钟条件未实现（会破坏回放确定性）' },
  { item: '技能 / AGENTS.md', note: '启动时加载一次；会话中途新增需重启才可见' },
  { item: 'npm 预编译产物', note: 'Linux 产物不含桌面窗口（避免纯终端用户被迫安装整套 webkit）' },
]
</script>

<template>
  <div class="page">
    <!-- 顶部导航 -->
    <header class="nav">
      <div class="wrap">
        <a class="brand" href="#top">
          <span class="logo" aria-hidden="true">◈</span> neo
        </a>
        <nav class="links">
          <a href="#hosts">宿主</a>
          <a href="#features">特性</a>
          <a href="#arch">架构</a>
          <a href="#install">安装</a>
          <a :href="REPO" target="_blank" rel="noopener">GitHub ↗</a>
        </nav>
      </div>
    </header>

    <main id="top">
      <!-- Hero -->
      <section class="hero wrap">
        <p class="eyebrow">Rust 内核 · 四宿主共享</p>
        <h1>
          用 Rust 重写的<br />
          <span class="grad">编程 Agent 内核</span>
        </h1>
        <p class="lede">
          一套内核，四个宿主。真实 OS 级沙箱，沙箱与审批是正交双轴，
          会话可完整回放。<strong>23 个 crate，零 unsafe，零 warning。</strong>
        </p>
        <div class="cta">
          <a class="btn btn-primary" href="#install">立即安装</a>
          <a class="btn" :href="REPO" target="_blank" rel="noopener">查看源码</a>
        </div>
        <p class="fine">无需 API key 也能试：内置离线桩 provider。</p>
      </section>

      <!-- 终端演示 -->
      <section class="wrap section">
        <TerminalDemo title="neo exec · 工具调用与真实落盘" :lines="demoLines">
          <p class="caption">
            上图取自真实执行输出。复现：
            <code>neo exec "写一个文件" --provider selftest --mode auto-edit --allow-writes</code>
            —— 桩 provider 会真的调用 <code>apply_patch</code> 并落盘，走的是与真实模型相同的沙箱链路。
          </p>
        </TerminalDemo>
      </section>

      <!-- 四宿主 -->
      <section id="hosts" class="wrap section">
        <h2>四宿主，一个内核</h2>
        <p class="sub">
          宿主只是内核的消费者，不持有业务状态。同一事件流喂给四个宿主，语义完全等价 ——
          这条称为 <strong>T6 铁律</strong>，由机器断言，不靠约定。
        </p>
        <div class="grid grid-4">
          <article v-for="h in hosts" :key="h.name" class="panel card">
            <h3>{{ h.name }}</h3>
            <code class="cmd">{{ h.cmd }}</code>
            <p>{{ h.desc }}</p>
          </article>
        </div>
      </section>

      <!-- 特性 -->
      <section id="features" class="wrap section">
        <h2>为什么是 Rust，为什么这样分层</h2>
        <p class="sub">
          目标不是"用 Rust 写一遍"，而是机制层面两个硬要求：无 GC 停顿的高性能主循环，
          以及类型层面保证的内存安全。附带收益是去掉过重的壳。
        </p>
        <div class="grid grid-3">
          <article v-for="f in features" :key="f.title" class="panel card">
            <h3>{{ f.title }}</h3>
            <p>{{ f.body }}</p>
          </article>
        </div>
      </section>

      <!-- 架构 -->
      <section id="arch" class="wrap section">
        <h2>六层，依赖只能向下</h2>
        <p class="sub">
          层与层之间的依赖方向由守卫脚本强制（<code>check_architecture.py</code>），
          违反的改动直接失败。三个宿主之间也不允许相互依赖。
        </p>
        <ol class="layers">
          <li v-for="l in layers" :key="l.id" class="panel">
            <span class="badge">{{ l.id }}</span>
            <span class="lname">{{ l.name }}</span>
            <span class="ldetail">{{ l.detail }}</span>
            <code class="lcrates">{{ l.crates }}</code>
          </li>
        </ol>
      </section>

      <!-- 安装 -->
      <section id="install" class="wrap section">
        <h2>安装</h2>
        <p class="sub">三种方式，按「省事 → 可控」排列。</p>
        <CopyCommands :commands="installs" />
        <p class="caption">
          命令名始终是 <code>neo</code>。预编译产物覆盖 macOS（Apple Silicon / Intel）与 Linux x86_64；
          其它平台用 <code>cargo install</code>。装好后若提示 command not found，是安装目录不在 PATH。
        </p>
      </section>

      <!-- 诚实边界 -->
      <section id="boundaries" class="wrap section">
        <h2>诚实边界</h2>
        <p class="sub">
          下面是<strong>当前未实现或已知受限</strong>的部分。写在官网而不是埋在文档里，
          是因为"哪些路走不通"与"哪些路走通了"同样重要。
        </p>
        <div class="panel">
          <table class="bounded">
            <thead>
              <tr><th>项</th><th>状态</th></tr>
            </thead>
            <tbody>
              <tr v-for="b in boundaries" :key="b.item">
                <td class="bitem">{{ b.item }}</td>
                <td>{{ b.note }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </main>

    <footer class="foot">
      <div class="wrap">
        <p>
          MIT 许可 ·
          <a :href="REPO" target="_blank" rel="noopener">github.com/proteus-vue/proteus-code</a>
        </p>
        <p class="fine">
          本站用 <a href="https://github.com/proteus-vue/proteus" target="_blank" rel="noopener">Proteus</a>
          框架构建（同组织的 Vue 跨端框架）—— 不是普通 Vite 模板。
        </p>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.wrap {
  max-width: var(--neo-maxw);
  margin: 0 auto;
  padding: 0 1.5rem;
}

/* ── 导航 ───────────────────────────────────────── */
.nav {
  position: sticky;
  top: 0;
  z-index: 10;
  background: color-mix(in srgb, var(--neo-backdrop) 88%, transparent);
  backdrop-filter: blur(8px);
  border-bottom: 1px solid var(--neo-border);
}
.nav .wrap {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 3.6rem;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.45rem;
  font-weight: 700;
  font-size: 1.05rem;
  color: var(--neo-fg);
}
.logo {
  color: var(--neo-primary);
}
.links {
  display: flex;
  gap: 1.15rem;
  font-size: 0.86rem;
}
.links a {
  color: var(--neo-fg-dim);
}
.links a:hover {
  color: var(--neo-primary);
}

/* ── Hero ───────────────────────────────────────── */
.hero {
  padding: 5.5rem 1.5rem 1.5rem;
  text-align: center;
}
.eyebrow {
  margin: 0 0 0.9rem;
  font-size: 0.8rem;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--neo-fg-dim);
}
.hero h1 {
  font-size: clamp(2rem, 5.2vw, 3.4rem);
  letter-spacing: -0.02em;
}
.grad {
  background: linear-gradient(120deg, var(--neo-primary), var(--neo-accent));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
.lede {
  max-width: 44rem;
  margin: 1.2rem auto 0;
  color: var(--neo-fg-dim);
  font-size: 1.03rem;
}
.lede strong {
  color: var(--neo-fg);
}
.cta {
  display: flex;
  gap: 0.8rem;
  justify-content: center;
  margin-top: 1.8rem;
  flex-wrap: wrap;
}
.fine {
  margin-top: 1rem;
  font-size: 0.82rem;
  color: var(--neo-fg-dim);
}

/* ── 区块通用 ───────────────────────────────────── */
.section {
  padding: 3.4rem 1.5rem;
}
.section h2 {
  font-size: clamp(1.35rem, 2.6vw, 1.8rem);
}
.sub {
  margin: 0.7rem 0 1.7rem;
  max-width: 48rem;
  color: var(--neo-fg-dim);
}
.sub strong {
  color: var(--neo-fg);
}
.caption {
  margin: 0.85rem 0 0;
  font-size: 0.82rem;
  color: var(--neo-fg-dim);
  line-height: 1.65;
}
.caption code,
.sub code {
  padding: 0.08rem 0.35rem;
  font-size: 0.85em;
  color: var(--neo-primary);
  background: var(--neo-element);
  border-radius: 4px;
}

/* ── 卡片网格 ───────────────────────────────────── */
.grid {
  display: grid;
  gap: 1rem;
}
.grid-4 {
  grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
}
.grid-3 {
  grid-template-columns: repeat(auto-fit, minmax(17rem, 1fr));
}
.card {
  padding: 1.15rem 1.2rem;
}
.card h3 {
  font-size: 1rem;
  margin-bottom: 0.15rem;
}
.card .cmd {
  display: inline-block;
  margin-bottom: 0.6rem;
  font-size: 0.78rem;
  color: var(--neo-accent);
}
.card p {
  margin: 0;
  font-size: 0.88rem;
  color: var(--neo-fg-dim);
}

/* ── 架构层 ─────────────────────────────────────── */
.layers {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.layers li {
  display: flex;
  align-items: center;
  gap: 0.85rem;
  padding: 0.7rem 1rem;
  font-size: 0.88rem;
}
.badge {
  flex: 0 0 auto;
  padding: 0.1rem 0.5rem;
  font-family: var(--neo-mono);
  font-size: 0.74rem;
  font-weight: 700;
  color: var(--neo-primary);
  background: var(--neo-selected);
  border-radius: 4px;
}
.lname {
  flex: 0 0 3.4rem;
  font-weight: 600;
}
.ldetail {
  flex: 1 1 auto;
  color: var(--neo-fg-dim);
}
.lcrates {
  flex: 0 0 auto;
  font-size: 0.74rem;
  color: var(--neo-fg-dim);
  opacity: 0.75;
}

/* ── 边界表 ─────────────────────────────────────── */
.bounded {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.87rem;
}
.bounded th,
.bounded td {
  padding: 0.7rem 1rem;
  text-align: left;
  border-bottom: 1px solid var(--neo-border);
  vertical-align: top;
}
.bounded th {
  font-size: 0.76rem;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--neo-fg-dim);
}
.bounded tr:last-child td {
  border-bottom: none;
}
.bitem {
  white-space: nowrap;
  font-weight: 600;
}
.bounded td:last-child {
  color: var(--neo-fg-dim);
}

/* ── 页脚 ───────────────────────────────────────── */
.foot {
  margin-top: 2rem;
  padding: 2rem 0 3rem;
  border-top: 1px solid var(--neo-border);
  text-align: center;
  font-size: 0.85rem;
  color: var(--neo-fg-dim);
}
.foot p {
  margin: 0.35rem 0;
}

@media (max-width: 640px) {
  .links {
    gap: 0.8rem;
  }
  .links a:nth-child(-n + 2) {
    display: none;
  }
  .hero {
    padding-top: 3.4rem;
  }
  .ldetail,
  .lcrates {
    display: none;
  }
}
</style>
