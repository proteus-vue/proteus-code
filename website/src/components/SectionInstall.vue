<!-- src/components/SectionInstall.vue —— 安装
     对标同行的收尾方式：一个大号、可直接复制的主命令 + 折叠的其它方式，
     末尾一句"装好之后敲什么"。避免把三条命令平铺成等权重（那样用户不会选）。 -->
<script setup lang="ts">
import { ref } from 'vue'
import CopyCommands from './CopyCommands.vue'

const REPO = 'https://github.com/proteus-vue/proteus-code'

const primary = {
  label: '一键脚本',
  cmd: 'curl -fsSL https://raw.githubusercontent.com/proteus-vue/proteus-code/main/scripts/install.sh | sh',
}
const others = [
  { label: 'npm', cmd: 'npm install -g @proteus-vue/neo-code' },
  { label: 'cargo', cmd: `cargo install --git ${REPO} -p neo-code-cli --locked` },
  { label: '源码构建', cmd: 'cargo build --release -p neo-code-cli' },
]

const copied = ref(false)
async function copyPrimary() {
  try {
    await navigator.clipboard.writeText(primary.cmd)
    copied.value = true
    setTimeout(() => (copied.value = false), 1600)
  } catch {
    copied.value = false
  }
}
</script>

<template>
  <section id="install" class="section">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="eyebrow">开始</span>
        <h2>一条命令装上</h2>
        <p>
          预编译产物覆盖 macOS（Apple Silicon / Intel）与 Linux x86_64，
          安装脚本会校验 SHA-256。其它平台走 <code>cargo install</code>。
        </p>
      </div>

      <!-- 主命令：单独放大，权重要压过其它方式 -->
      <div class="primary reveal">
        <div class="cmd-row">
          <span class="prompt">$</span>
          <code>{{ primary.cmd }}</code>
          <button type="button" class="copy" @click="copyPrimary">
            {{ copied ? '已复制' : '复制' }}
          </button>
        </div>
      </div>

      <div class="others reveal">
        <p class="others-label">其它方式</p>
        <CopyCommands :commands="others" />
      </div>

      <div class="next reveal">
        <p class="next-label">装好之后</p>
        <!-- 命令与注释分列元素（而不是靠空格对齐）：窄屏折行时注释不会
             缩进到命令列里、被误读成命令的一部分。 -->
        <ul class="next-list">
          <li><code>neo tui --provider mock</code><span>离线全览界面与交互，无需 API key</span></li>
          <li><code>neo --help</code><span>查看全部用法</span></li>
          <li><code>neo --version</code><span>版本</span></li>
        </ul>
        <p class="hint">
          命令名始终是 <code>neo</code>。若提示 command not found，是安装目录不在
          PATH（脚本装到 <code>~/.local/bin</code>，cargo 装到 <code>~/.cargo/bin</code>）。
        </p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.primary {
  margin-bottom: 2.2rem;
}
.cmd-row {
  display: flex;
  align-items: center;
  gap: 0.85rem;
  padding: 1.05rem 1.2rem;
  background: linear-gradient(180deg, var(--neo-element), var(--neo-panel));
  border: 1px solid var(--neo-border-active);
  border-radius: var(--neo-radius);
  box-shadow: 0 0 0 1px rgba(167, 139, 250, 0.05), 0 18px 50px -24px rgba(139, 92, 246, 0.4);
}
.prompt {
  flex: 0 0 auto;
  font-family: var(--neo-mono);
  font-size: 1rem;
  color: var(--neo-primary);
}
.cmd-row code {
  flex: 1 1 auto;
  font-size: 0.86rem;
  overflow-x: auto;
  white-space: nowrap;
}
.copy {
  flex: 0 0 auto;
  padding: 0.38rem 0.85rem;
  font: inherit;
  font-size: var(--fs-xs);
  color: var(--neo-fg-dim);
  background: transparent;
  border: 1px solid var(--neo-border-active);
  border-radius: var(--neo-radius-sm);
  cursor: pointer;
  transition: color 0.16s ease, border-color 0.16s ease;
}
.copy:hover {
  color: var(--neo-primary);
  border-color: var(--neo-primary);
}

.others-label,
.next-label {
  margin: 0 0 0.7rem;
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--neo-fg-faint);
}

.others {
  margin-bottom: 2.2rem;
}

.next-list {
  list-style: none;
  margin: 0;
  padding: 1.1rem 1.2rem;
  background: var(--neo-panel);
  border: 1px solid var(--neo-border);
  border-radius: var(--neo-radius);
  font-size: 0.82rem;
}
.next-list li {
  display: flex;
  align-items: baseline;
  gap: 1rem;
  padding: 0.28rem 0;
}
.next-list code {
  flex: 0 0 auto;
  color: var(--neo-fg);
}
.next-list span {
  flex: 1 1 auto;
  color: var(--neo-fg-faint);
}

@media (max-width: 560px) {
  /* 窄屏改为两行：命令一行、说明一行，注释不会缩进到命令列里 */
  .next-list li {
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.35rem 0;
  }
}

.hint {
  margin: 0.85rem 0 0;
  font-size: var(--fs-sm);
  color: var(--neo-fg-dim);
}
.hint code,
.section-head code {
  padding: 0.08rem 0.35rem;
  font-size: 0.86em;
  color: var(--neo-primary);
  background: var(--neo-element);
  border-radius: 4px;
}

@media (max-width: 640px) {
  /* ★ 命令在窄屏必须**换行**而不是被裁掉：
     这些命令是用户要复制去执行的，截断会让人以为命令就是这么写的。
     同时把提示符与按钮排在上面，命令独占整行，才有完整宽度可用。 */
  .cmd-row {
    flex-wrap: wrap;
    row-gap: 0.6rem;
  }
  .cmd-row code {
    order: 3;
    flex-basis: 100%;
    font-size: 0.72rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere; /* URL 里没有空格，必须允许任意处断行 */
    overflow-x: visible;
  }
}
</style>
