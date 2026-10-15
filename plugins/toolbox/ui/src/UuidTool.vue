<script setup lang="ts">
/** UUID 工具：crypto.randomUUID（v4）批量生成，单条点击复制 + 一键复制全部 */
import { inject, onMounted, ref } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY, generateUuid } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const countText = ref('5')
const list = ref<string[]>([])

function generate() {
  const n = Math.floor(Number(countText.value))
  const safe = Number.isFinite(n) ? Math.min(100, Math.max(1, n)) : 5
  countText.value = String(safe)
  list.value = Array.from({ length: safe }, () => generateUuid())
}

function copyAll() {
  api.copyText(list.value.join('\n'), `已复制 ${list.value.length} 个 UUID`)
}

function reset() {
  countText.value = '5'
  generate()
}

onMounted(generate)
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">UUID 生成</div>
          <div class="tb-desc">v4 · crypto.randomUUID · 单条点击复制</div>
        </div>
        <span class="tb-flex" />
        <label class="count-wrap">
          <span class="tb-label">数量</span>
          <input
            v-model="countText"
            class="tb-input count-input"
            inputmode="numeric"
            spellcheck="false"
            @keyup.enter="generate"
          />
        </label>
        <button class="tb-btn primary" @click="generate">重新生成</button>
        <button class="tb-btn" @click="copyAll">复制全部</button>
        <button class="tb-btn ghost" @click="reset">重置</button>
      </div>

      <div class="uuid-list">
        <button
          v-for="(u, i) in list"
          :key="`${i}-${u}`"
          class="uuid-row"
          :title="`点击复制：${u}`"
          @click="api.copyText(u)"
        >
          <span class="uuid-idx tb-mono">{{ i + 1 }}</span>
          <span class="tb-mono uuid-val">{{ u }}</span>
        </button>
      </div>

      <div class="tb-status hint">
        共 {{ list.length }} 条 · 单条点击复制，「复制全部」以换行分隔
      </div>
    </div>
  </div>
</template>

<style scoped>
.count-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
}
.count-input {
  width: 58px;
  padding: 5px 8px;
  text-align: center;
}

.uuid-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 300px;
  overflow-y: auto;
}
.uuid-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--surface) 65%, transparent);
  color: var(--text);
  font-size: 12px;
  text-align: left;
  transition: border-color 0.15s, background 0.15s;
}
.uuid-row:hover {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, var(--surface));
}
.uuid-idx {
  flex: none;
  width: 24px;
  text-align: right;
  font-size: 11px;
  color: var(--text-muted);
}
.uuid-val {
  min-width: 0;
  overflow-wrap: anywhere;
  user-select: text;
}
</style>
