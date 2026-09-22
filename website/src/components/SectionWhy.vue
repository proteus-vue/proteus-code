<!-- src/components/SectionWhy.vue —— 对比与 FAQ
     对标同行的"为什么选我们"段落。★ 纪律：只做**可核查**的对比 ——
     同行那一列的取值来自它们的公开文档与仓库（Apache-2.0 / MIT），
     本项目这一列来自自己的门禁输出。不写营销形容词。

     FAQ 用原生 <details>：零 JS、可被搜索引擎与 a11y 工具理解，比自绘折叠面板更稳。 -->
<script setup lang="ts">
// 对比维度：挑"选型时真正会纠结"的，不挑我们赢的
const compare = [
  {
    dim: '内核实现',
    peer: 'TypeScript / Node 运行时',
    here: 'Rust 单二进制，无 GC 停顿',
  },
  {
    dim: '沙箱边界',
    peer: '取决于宿主，多为进程级约定',
    here: 'OS 级强制（macOS Seatbelt 真机验证）；未实现平台 fail-closed',
  },
  {
    dim: '可替换性',
    peer: '插件生态，接口未强制统一',
    here: '5 个有名 SPI，每个强制「契约 + ≥2 后端 + conformance」',
  },
  {
    dim: '多宿主一致性',
    peer: '各前端各自消费事件',
    here: 'T6 铁律：同事件流喂五宿主，事实等价由机器断言',
  },
  {
    dim: '内存有界性',
    peer: '依赖宿主与 GC 策略',
    here: '内核强制上限 + UTF-8 安全截断，计数分配器实测',
  },
  {
    dim: '成熟度（诚实说）',
    peer: '生态成熟、用户基数大',
    here: 'M0–M3/M5/M6 闭环；Linux/Windows 沙箱未实现',
  },
]

const faqs = [
  {
    q: '装完需要 API key 才能用吗？',
    a: '不需要。内置三个确定性桩 provider（mock / selftest / demo），没有 key 也能把界面、审批、落盘整条链路走一遍。要看真实推理质量才需要配置服务商密钥。',
  },
  {
    q: 'Linux / Windows 上能跑吗？',
    a: '能跑，但受限于沙箱：这两个平台的沙箱后端尚未实现，受限执行档位会 fail-closed（拒绝执行），不会静默降级放行。可用的是不落地的档位（如 plan）。',
  },
  {
    q: '为什么不用 Electron？',
    a: '桌面宿主用系统 webview（macOS WKWebView / Windows WebView2 / Linux WebKitGTK），不捆绑 Chromium 与 Node —— 安装体积从 Electron 的 ~78 MB 降到 5–10 MB。代价是三个平台的 webview 行为不完全一致。',
  },
  {
    q: '和直接用 Codex / DSH 有什么区别？',
    a: '借鉴了 Codex 的运行时架构（SQ/EQ、沙箱×审批双轴、Shell-First）与 DSH 的接入思路，但内核是重写而非 fork：目的是把 DSH 里约 90 个无名 seam 收敛为 5 个有名 SPI，让可替换性可被静态分析与 CI 校验。',
  },
  {
    q: '这个站点是用什么做的？',
    a: '用本组织自己的 Proteus 跨端框架构建（dogfooding）。框架目前全线 beta，故本站依赖钉死精确版本；官网那套语义组件未发布 npm，所以 UI 是手写的。',
  },
]
</script>

<template>
  <section id="why" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">选型</span>
        <h2>和同类工具比，差在哪、强在哪</h2>
        <p>
          只列<strong>选型时真正会纠结</strong>的维度。同行那一列取自它们的公开文档与仓库，
          这一列取自本项目的门禁输出 —— 包括对我们不利的那一行。
        </p>
      </div>

      <div class="panel table-wrap reveal">
        <table>
          <thead>
            <tr>
              <th>维度</th>
              <th>同类编程 Agent</th>
              <th>neo</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="c in compare" :key="c.dim">
              <!-- data-label 供窄屏堆叠时显示列名（否则手机上一行三列会被
                   横向裁掉，而这一节的重点恰恰是第三列） -->
              <td class="dim" data-label="维度">{{ c.dim }}</td>
              <td class="peer" data-label="同类编程 Agent">{{ c.peer }}</td>
              <td class="here" data-label="neo">{{ c.here }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="faq-head reveal">
        <span class="eyebrow">常见问题</span>
        <h2>装之前会想知道的事</h2>
      </div>

      <div class="faqs reveal">
        <details v-for="f in faqs" :key="f.q" class="faq panel">
          <summary>
            <span>{{ f.q }}</span>
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8"
              stroke-linecap="round" aria-hidden="true">
              <path d="M8 3.5v9M3.5 8h9" />
            </svg>
          </summary>
          <p>{{ f.a }}</p>
        </details>
      </div>
    </div>
  </section>
</template>

<style scoped>
.table-wrap {
  overflow-x: auto;
}

table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-sm);
  min-width: 40rem;
}

th,
td {
  padding: 0.9rem 1.15rem;
  text-align: left;
  vertical-align: top;
  border-bottom: 1px solid var(--neo-border);
}

thead th {
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--neo-fg-faint);
  background: color-mix(in srgb, var(--neo-element) 60%, transparent);
}

tbody tr:last-child td {
  border-bottom: none;
}
tbody tr {
  transition: background 0.18s ease;
}
tbody tr:hover {
  background: color-mix(in srgb, var(--neo-element) 45%, transparent);
}

.dim {
  white-space: nowrap;
  font-weight: 600;
  color: var(--neo-fg);
}
.peer {
  color: var(--neo-fg-faint);
}
/* neo 那一列给主色，视觉上明确"这是我们的回答" */
.here {
  color: var(--neo-fg);
  border-left: 2px solid color-mix(in srgb, var(--neo-primary) 40%, transparent);
}

.faq-head {
  margin-top: 4rem;
  margin-bottom: 1.8rem;
}
/* 与其它区块标题同级 —— 同为「章节标题」，字号必须一致，
   否则 FAQ 看起来像被降了一级（同屏对比时很显眼）。 */
.faq-head h2 {
  font-size: var(--fs-h2);
}

.faqs {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.faq {
  overflow: hidden;
}
.faq summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 1.05rem 1.25rem;
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  list-style: none;
}
.faq summary::-webkit-details-marker {
  display: none;
}
.faq summary svg {
  flex: 0 0 auto;
  width: 1.05rem;
  height: 1.05rem;
  color: var(--neo-primary);
  transition: transform 0.22s ease;
}
.faq[open] summary svg {
  transform: rotate(45deg);
}
.faq p {
  margin: 0;
  padding: 0 1.25rem 1.15rem;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
  line-height: 1.75;
}

/* ── 窄屏：对比表改为堆叠卡片 ──────────────────────
   三列在 390px 下放不下，横向滚动会让**最关键的那一列（neo）默认在视口外**，
   等于这一节白写。改成每行一张卡、用 data-label 标出列名。 */
@media (max-width: 720px) {
  .table-wrap {
    overflow-x: visible;
    border: none;
    background: transparent;
  }
  table {
    min-width: 0;
  }
  thead {
    /* 列名移到每个单元格上，表头就不需要了 */
    display: none;
  }
  tbody tr {
    display: block;
    margin-bottom: 0.7rem;
    padding: 0.95rem 1.15rem;
    background: var(--neo-panel);
    border: 1px solid var(--neo-border);
    border-radius: var(--neo-radius);
  }
  tbody tr:hover {
    background: var(--neo-panel);
  }
  tbody td {
    display: block;
    padding: 0;
    border: none;
  }
  tbody td::before {
    content: attr(data-label);
    display: block;
    margin-bottom: 0.2rem;
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--neo-fg-faint);
  }
  .dim {
    font-size: 1rem;
    font-weight: 700;
    margin-bottom: 0.6rem;
  }
  .dim::before {
    display: none; /* 卡片标题就是维度名，不用再挂列名 */
  }
  .peer {
    margin-bottom: 0.55rem;
  }
  /* neo 那一列加左侧主色条，保持"这是我们的回答"的视觉标记 */
  .here {
    padding-left: 0.75rem;
    border-left: 2px solid color-mix(in srgb, var(--neo-primary) 45%, transparent);
  }
  .here::before {
    color: var(--neo-primary);
  }
}
</style>
