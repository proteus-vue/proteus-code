<!-- src/components/SectionFeatures.vue —— 特性网格（带图标）
     对标同行：特性卡片必须有图标 —— 纯文字网格扫起来是一堵墙。图标是内联 SVG
     （见 FeatureIcon.vue），不引图标库（本站依赖全钉 beta，少一个依赖少一处漂移）。 -->
<script setup lang="ts">
import FeatureIcon from './FeatureIcon.vue'

const features = [
  {
    icon: 'shield',
    title: '真实 OS 级沙箱',
    body: 'macOS 用系统 Seatbelt，真机验证越权拦截（6 项）。沙箱是内核的结构保证：工具在类型层面就没有绕开沙箱的出口。',
  },
  {
    icon: 'axes',
    title: '沙箱 × 审批正交双轴',
    body: '沙箱决定「能做什么」，审批决定「何时必须问」。两者独立配置，不是同一个旋钮的两档 —— 最常见的安全配置误解就在这。',
  },
  {
    icon: 'plug',
    title: '5 个有名 SPI',
    body: 'ModelProvider / SandboxBackend / SessionPersistence / HostBackend / Tool，每个都强制「契约 + ≥2 后端 + conformance」。',
  },
  {
    icon: 'terminal',
    title: '离线也能全览',
    body: '内置三个确定性桩 provider，没有 API key 也能把界面与内核交互完整走一遍 —— 装完即可上手。',
  },
  {
    icon: 'ruler',
    title: '内存有界',
    body: 'Rust 只消除 UB，不保证有界：一条命令就可能撑爆进程。输出上限、UTF-8 安全截断、上下文上限都由内核强制，并有实测。',
  },
  {
    icon: 'replay',
    title: '会话可完整回放',
    body: 'JSONL append-only 日志，凡进入模型请求的内容都能从日志重建。这是回放与审计成立的前提，不是可选项。',
  },
]
</script>

<template>
  <section id="features" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">设计取舍</span>
        <h2>为什么是 Rust，为什么这样分层</h2>
        <p>
          目标不是「用 Rust 写一遍」，而是机制层面的两个硬要求：无 GC 停顿的高性能主循环，
          与类型层面保证的内存安全。附带收益是去掉了过重的壳。
        </p>
      </div>

      <div class="grid">
        <article v-for="f in features" :key="f.title" class="card reveal">
          <FeatureIcon :name="f.icon" />
          <h3>{{ f.title }}</h3>
          <p>{{ f.body }}</p>
        </article>
      </div>
    </div>
  </section>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
  gap: 1rem;
}

h3 {
  font-size: var(--fs-h3);
  margin-bottom: 0.5rem;
}

p {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
  line-height: 1.7;
}
</style>
