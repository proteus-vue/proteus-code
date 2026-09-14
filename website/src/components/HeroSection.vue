<!-- src/components/HeroSection.vue —— 首屏
     对标同行的做法：① 背后有辉光 + 网格（深色站没有这个就是"什么都没画"）；
     ② 标题上方一行徽标（版本 / 许可 / 平台）传递可信度；
     ③ 终端**进首屏**，因为它就是产品本体 —— 同行都让产品自己站 C 位。 -->
<script setup lang="ts">
import TerminalDemo from './TerminalDemo.vue'

const REPO = 'https://github.com/proteus-vue/proteus-code'

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
</script>

<template>
  <section id="top" class="hero">
    <!-- 背景装饰：网格 + 两团辉光 -->
    <div class="grid-bg" aria-hidden="true" />
    <div class="glow g1" aria-hidden="true" />
    <div class="glow g2" aria-hidden="true" />

    <div class="wrap inner">
      <div class="badges">
        <span class="pill"><i class="dot" />617 测试通过</span>
        <span class="pill">23 crates</span>
        <span class="pill">零 unsafe</span>
        <span class="pill">MIT</span>
      </div>

      <h1>
        用 Rust 重写的<br />
        <span class="grad">编程 Agent 内核</span>
      </h1>

      <p class="lede">
        一套内核，四个宿主。真实 OS 级沙箱，沙箱与审批是<strong>正交双轴</strong>，
        会话可完整回放 —— 不靠约定，全部由门禁机器校验。
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

      <!-- 产品本体进首屏 -->
      <div class="stage">
        <TerminalDemo title="neo exec · 工具调用与真实落盘" :lines="demoLines" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.hero {
  position: relative;
  padding: 8.5rem 0 0;
  overflow: hidden;
}

/* subtle 网格与辉光（.grid-bg / .glow 定义在 global.css） */
.g1 {
  top: -14rem;
  left: 50%;
  width: 52rem;
  height: 34rem;
  margin-left: -26rem;
  background: radial-gradient(closest-side, rgba(167, 139, 250, 0.3), transparent);
}
.g2 {
  top: -6rem;
  left: 50%;
  width: 30rem;
  height: 22rem;
  margin-left: 4rem;
  background: radial-gradient(closest-side, rgba(232, 121, 249, 0.2), transparent);
}

.inner {
  position: relative;
  z-index: 1;
  text-align: center;
}

.badges {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 0.5rem;
  margin-bottom: 1.6rem;
}

h1 {
  font-size: var(--fs-hero);
  letter-spacing: -0.03em;
  line-height: 1.08;
}
.grad {
  background: linear-gradient(100deg, var(--neo-primary) 10%, var(--neo-accent) 70%);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.lede {
  max-width: 40rem;
  margin: 1.5rem auto 0;
  font-size: 1.08rem;
  color: var(--neo-fg-dim);
}
.lede strong {
  color: var(--neo-fg);
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
  margin: clamp(3rem, 6vw, 4.5rem) auto 0;
  text-align: left;
}

@media (max-width: 640px) {
  .hero {
    padding-top: 6.5rem;
  }
  .lede {
    font-size: 1rem;
  }
  /* 徽标行：4 个 pill 在 390px 下会「3 + 1」折行，末项孤立居中很失衡。
     收窄字号与间距，让它们能均匀占两行。 */
  .badges {
    gap: 0.4rem;
  }
  .badges :deep(.pill) {
    font-size: 0.72rem;
    padding: 0.24rem 0.6rem;
  }
}
</style>
