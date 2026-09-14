<!-- src/components/SectionBoundaries.vue —— 诚实边界
     ★ 这一段是本站**刻意保留**的：同行站点普遍只讲能力，我们把"未实现/受限"也列在官网。
     理由是项目一贯的判断 —— 知道哪些路走不通，和知道哪些走通了同样重要；
     而且这些边界都能在仓库里核实，写出来反而降低试错成本。 -->
<script setup lang="ts">
const boundaries = [
  { item: 'Linux / Windows 沙箱', state: '未实现', note: '受限档位 fail-closed（拒绝执行），不会降级放行' },
  { item: 'Desktop 打包', state: '未产品化', note: '窗口层已机器验证；正式发布仍需图标、签名与公证' },
  { item: 'MCP 提示模板', state: '未接入', note: '两种传输 + tools/resources 已就绪，prompts 未接' },
  { item: 'Goal 挂钟停止条件', state: '未实现', note: '四项确定性停止条件生效；挂钟条件会破坏回放确定性' },
  { item: '技能 / AGENTS.md', state: '启动时加载', note: '会话中途新增需重启才可见（引用是热路径，扫盘是冷路径）' },
  { item: 'Linux 预编译包', state: '不含桌面', note: '避免纯终端用户被迫安装整套 webkit' },
]
</script>

<template>
  <section id="boundaries" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">诚实边界</span>
        <h2>没做到的部分，也写在官网上</h2>
        <p>
          下面每一条都能在仓库里核实。<strong>知道哪些路走不通，和知道哪些走通了同样重要</strong>
          —— 埋进文档让用户去撞，代价更高。
        </p>
      </div>

      <ul class="list reveal">
        <li v-for="b in boundaries" :key="b.item" class="panel">
          <span class="state">{{ b.state }}</span>
          <span class="item">{{ b.item }}</span>
          <span class="note">{{ b.note }}</span>
        </li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.list li {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.85rem 1.15rem;
  font-size: var(--fs-sm);
  transition: border-color 0.2s ease;
}
.list li:hover {
  border-color: var(--neo-border-active);
}

/* 状态用药丸标出，扫一眼就知道"这条是缺的" */
.state {
  flex: 0 0 auto;
  min-width: 5.2rem;
  padding: 0.16rem 0.6rem;
  font-size: var(--fs-xs);
  text-align: center;
  color: var(--neo-warning);
  background: color-mix(in srgb, var(--neo-warning) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--neo-warning) 32%, transparent);
  border-radius: var(--neo-radius-pill);
}

.item {
  flex: 0 0 11rem;
  font-weight: 600;
  color: var(--neo-fg);
}

.note {
  flex: 1 1 auto;
  color: var(--neo-fg-dim);
}

@media (max-width: 720px) {
  .list li {
    flex-wrap: wrap;
    gap: 0.4rem 0.8rem;
  }
  .item {
    flex: 1 1 auto;
  }
  .note {
    flex-basis: 100%;
  }
}
</style>
