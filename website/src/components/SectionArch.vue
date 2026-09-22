<!-- src/components/SectionArch.vue —— 六层架构（视觉分层图）
     原来是一张平表，扫不出"依赖只能向下"这件事。改成**堆叠的层**：
     每层左侧一条渐变色条（越底层越偏向主色深处）、右上角标 crate，
     整体左侧一根向下箭头明确方向 —— 结构本身变成图，不用读文字。 -->
<script setup lang="ts">
// 自上而下：宿主 → 协议（依赖方向也是自上而下）
const layers = [
  { id: 'L5', name: '宿主', detail: 'TUI / Desktop / Web / Exec / app-server', crates: 'neo-host-* · neo-exec · neo-code-cli', depth: 5 },
  { id: 'L4', name: '编排', detail: 'Goal 引擎（Plan → Code → Review → Learn）', crates: 'neo-orchestration', depth: 4 },
  { id: 'L3', name: '能力', detail: 'Shell-First 工具集 + 各 SPI 实现', crates: 'neo-capability · *-local · neo-mcp', depth: 3 },
  { id: 'L2', name: '内核', detail: 'turn/step 主循环 · 三维闸门 · SPI 契据', crates: 'neo-core', depth: 2 },
  { id: 'L1', name: '平台', detail: '命令包裹 · 进程加固 · fs notify', crates: 'neo-sandbox · neo-platform', depth: 1 },
  { id: 'L0', name: '协议', detail: 'Op / EventMsg / 双轴枚举（零业务依赖）', crates: 'neo-protocol', depth: 0 },
]
</script>

<template>
  <section id="arch" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">结构</span>
        <h2>六层，依赖只能向下</h2>
        <p>
          层与层之间的依赖方向由守卫脚本强制（<code>check_architecture.py</code>），
          向上依赖、依赖成环、宿主互相引用都会让改动直接失败 —— 架构不靠自觉维持。
        </p>
      </div>

      <div class="diagram reveal">
        <div class="axis" aria-hidden="true">
          <span class="axis-label">依赖方向</span>
          <span class="arrow" />
          <span class="axis-note">只能向下</span>
        </div>

        <ol class="layers">
          <li
            v-for="l in layers"
            :key="l.id"
            class="layer"
            :style="{ '--t': (5 - l.depth) / 5 }"
          >
            <span class="lid">{{ l.id }}</span>
            <span class="lname">{{ l.name }}</span>
            <span class="ldetail">{{ l.detail }}</span>
            <code class="lcrates">{{ l.crates }}</code>
          </li>
        </ol>
      </div>
    </div>
  </section>
</template>

<style scoped>
.diagram {
  display: grid;
  grid-template-columns: 3.2rem 1fr;
  gap: 1.2rem;
  align-items: stretch;
}

/* 左侧方向轴：一根竖线 + 箭头，把"只能向下"画出来 */
.axis {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  padding-top: 0.2rem;
}
.axis-label,
.axis-note {
  font-size: 0.66rem;
  letter-spacing: 0.08em;
  color: var(--neo-fg-faint);
  writing-mode: vertical-rl;
  text-orientation: mixed;
}
.arrow {
  flex: 1 1 auto;
  width: 1px;
  background: linear-gradient(
    180deg,
    transparent,
    var(--neo-primary) 12%,
    var(--neo-accent) 88%,
    transparent
  );
  position: relative;
}
.arrow::after {
  content: "";
  position: absolute;
  bottom: -1px;
  left: 50%;
  width: 0.42rem;
  height: 0.42rem;
  border-right: 1px solid var(--neo-accent);
  border-bottom: 1px solid var(--neo-accent);
  transform: translateX(-50%) rotate(45deg);
}

.layers {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
}

.layer {
  position: relative;
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.8rem 1.1rem 0.8rem 1.25rem;
  background: var(--neo-panel);
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius-sm);
  font-size: var(--fs-sm);
  transition: border-color 0.2s ease, transform 0.2s ease;
  /* 越往下越靠近协议层（更深的主色），把"层深"编码成颜色 */
  border-left: 3px solid color-mix(in srgb, var(--neo-primary) calc(35% + var(--t) * 50%), var(--neo-border));
}
.layer:hover {
  border-color: var(--neo-border-active);
  transform: translateX(3px);
}

.lid {
  flex: 0 0 auto;
  padding: 0.1rem 0.5rem;
  font-family: var(--neo-mono);
  font-size: 0.72rem;
  font-weight: 700;
  color: var(--neo-primary);
  background: var(--neo-selected);
  border-radius: 4px;
}

.lname {
  flex: 0 0 3.2rem;
  font-weight: 600;
}

.ldetail {
  flex: 1 1 auto;
  color: var(--neo-fg-dim);
}

.lcrates {
  flex: 0 0 auto;
  font-size: 0.72rem;
  color: var(--neo-fg-faint);
}

@media (max-width: 860px) {
  .diagram {
    grid-template-columns: 1fr;
  }
  .axis {
    flex-direction: row;
    justify-content: flex-start;
    gap: 0.6rem;
  }
  .arrow {
    display: none;
  }
  .axis-label,
  .axis-note {
    writing-mode: horizontal-tb;
  }
  /* ★ 窄屏把每层改成两行堆叠，**保留全部信息**：
     早先把 .ldetail / .lcrates 直接 display:none，结果是只留下
     「L5 宿主」这种标签、卡片右半空空 —— 那一屏等于没讲清楚分层是什么。
     信息不该为了排版被删掉，只能重排。 */
  .layer {
    flex-wrap: wrap;
    gap: 0.3rem 0.7rem;
    padding: 0.75rem 1rem 0.75rem 1.1rem;
  }
  .lname {
    flex: 1 1 auto;
  }
  .ldetail {
    flex-basis: 100%;
    font-size: var(--fs-xs);
    padding-left: 0; /* 与层名左对齐，视觉上属于同一层 */
  }
  .lcrates {
    flex-basis: 100%;
    font-size: 0.68rem;
    padding-left: 0;
    opacity: 0.85;
  }
}
</style>
