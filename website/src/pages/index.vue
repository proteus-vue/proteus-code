<!-- src/pages/index.vue —— 站点首页（单页 landing）
     ★ 站点本身就是一帧 TUI：星场背景常驻、底部常驻状态栏、分组一律用
     `┃` 单边条面板（TuiPanel），键位提示行（TuiKeyHints）复刻 TUI 底栏。
     外观（主题 / 背景纹理）可实时切换，与 TUI 的 ctrl+t / /background 同源。

     内容原则（沿用本项目一贯风格）：
       · 只讲**已落地可验证**的能力，"未实现/边界"如实列出（SectionBoundaries）
       · 数字给出来源：23 crate / 617 测试 / 零 unsafe / 零 warning 都能在门禁里复现 -->
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
import { useTheme } from '../composables/useTheme'

import TuiBackdrop from '../components/TuiBackdrop.vue'
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
import TuiStatusBar from '../components/TuiStatusBar.vue'

const { bg, init } = useTheme()
init() // 恢复本地保存的外观并写到 CSS 变量（同时会派发 neo-theme-changed）

useReveal()
</script>

<template>
  <TuiBackdrop :kind="bg" />
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
  <TuiStatusBar />
</template>
