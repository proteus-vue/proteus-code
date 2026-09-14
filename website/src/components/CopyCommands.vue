<!-- src/components/CopyCommands.vue —— 安装命令块（多行命令 + 复制按钮）
     放在 components/ 下：Proteus 约定扫描该目录自动可用（与 TUI 的"复用原语"同一条纪律，
     命令块只写一次，Hero 与安装区复用同一组件）。 -->
<script setup lang="ts">
import { ref } from 'vue'

defineProps<{
  /** 每行一条命令；点"复制"复制该行 */
  commands: { label: string; cmd: string }[]
}>()

const copiedIdx = ref<number | null>(null)

async function copy(cmd: string, idx: number) {
  try {
    await navigator.clipboard.writeText(cmd)
    copiedIdx.value = idx
    setTimeout(() => {
      if (copiedIdx.value === idx) copiedIdx.value = null
    }, 1600)
  } catch {
    // 剪贴板不可用（非 https / 权限拒绝）时静默失败：用户仍可手动选中复制
    copiedIdx.value = null
  }
}
</script>

<template>
  <div class="cmds">
    <div v-for="(c, i) in commands" :key="c.cmd" class="cmd">
      <span class="label">{{ c.label }}</span>
      <code class="code">{{ c.cmd }}</code>
      <button class="copy" type="button" @click="copy(c.cmd, i)">
        {{ copiedIdx === i ? '已复制' : '复制' }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.cmds {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.cmd {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.7rem 0.9rem;
  background: var(--neo-element);
  border: 1px solid var(--neo-border);
  /* 与安装区的其它代码块用同一档圆角（原来这里是 8px、别处 14px，
     相邻两块圆角不一致看着像两套组件）。 */
  border-radius: var(--neo-radius);
}

.label {
  flex: 0 0 auto;
  min-width: 4.6rem;
  font-size: 0.78rem;
  color: var(--neo-fg-dim);
}

.code {
  flex: 1 1 auto;
  font-size: 0.84rem;
  color: var(--neo-fg);
  overflow-x: auto;
  white-space: nowrap;
}

.copy {
  flex: 0 0 auto;
  padding: 0.3rem 0.7rem;
  font-size: 0.76rem;
  font-family: inherit;
  color: var(--neo-fg-dim);
  background: transparent;
  border: 1px solid var(--neo-border);
  border-radius: 4px;
  cursor: pointer;
  transition: color 0.15s ease, border-color 0.15s ease;
}
.copy:hover {
  color: var(--neo-primary);
  border-color: var(--neo-primary);
}

@media (max-width: 560px) {
  .cmd {
    flex-wrap: wrap;
  }
  .label {
    min-width: auto;
  }
  /* ★ 命令换行而非横向裁切 —— 它是给用户复制执行的，截断会误导。
     按钮与标签留在第一行，命令独占整行拿到完整宽度。 */
  .code {
    order: 3;
    flex-basis: 100%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    overflow-x: visible;
  }
}
</style>
