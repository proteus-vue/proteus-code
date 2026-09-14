<!-- src/pages/index.vue —— 站点首页（单页 landing）
     内容原则（沿用本项目一贯风格）：
       · 只讲**已落地可验证**的能力，"未实现/边界"如实列出（SectionBoundaries），不粉饰
       · 数字给出来源：23 crate / 617 测试 / 零 unsafe / 零 warning 都能在仓库门禁里复现

     结构：Hero（含终端）→ 数据带 → 四宿主 → 特性 → 架构 → 对比+FAQ → 安装 → 诚实边界 → 页脚
     每个区块的"进入方式"都不同（辉光 / 数据带 / 卡片 / 图解 / 表格 / 大命令），
     避免通篇 h2 + 网格变成一张长表。 -->
<route>
{
  "meta": {
    "title": "neo —— 用 Rust 重写的编程 Agent 内核",
    "isTab": true
  }
}
</route>

<script setup lang="ts">
import { useReveal } from '../composables/useReveal'
import SiteNav from '../components/SiteNav.vue'
import HeroSection from '../components/HeroSection.vue'
import StatsBand from '../components/StatsBand.vue'
import SectionHosts from '../components/SectionHosts.vue'
import SectionFeatures from '../components/SectionFeatures.vue'
import SectionArch from '../components/SectionArch.vue'
import SectionWhy from '../components/SectionWhy.vue'
import SectionInstall from '../components/SectionInstall.vue'
import SectionBoundaries from '../components/SectionBoundaries.vue'
import SiteFooter from '../components/SiteFooter.vue'

// 滚动揭示（IntersectionObserver；尊重 prefers-reduced-motion）
useReveal()
</script>

<template>
  <SiteNav />
  <main>
    <HeroSection />
    <StatsBand />
    <SectionHosts />
    <SectionFeatures />
    <SectionArch />
    <SectionWhy />
    <SectionInstall />
    <SectionBoundaries />
  </main>
  <SiteFooter />
</template>
