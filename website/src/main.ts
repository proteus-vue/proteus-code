// src/main.ts —— 站点入口（标准 Vue SPA）
import { createApp } from 'vue'
import App from './App.vue'
import './styles/global.css'

// 在挂载**之前**打 .js 标记：滚动揭示的初始隐藏只对 .js 生效（见 global.css）。
// 放这里而不是组件 onMounted，是为了消除"先可见、后隐藏"的一帧闪烁；
// 同时也保证 JS 不可用时内容从一开始就是可见的（而不是等不到标记的白屏）。
document.documentElement.classList.add('js')

createApp(App).mount('#app')
