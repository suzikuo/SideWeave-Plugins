<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 哈希工具：SHA-1 / SHA-256 / SHA-512（crypto.subtle），输入即算，HEX + Base64 双输出 */
import { inject, ref, watch } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY, bufferToBase64, bufferToHex, debouncedRef } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const ALGOS = ['SHA-1', 'SHA-256', 'SHA-512'] as const

interface HashItem {
  name: string
  hex: string
  b64: string
}

const input = ref('')
const debouncedInput = debouncedRef(input, 200)
const results = ref<HashItem[]>([])
const computing = ref(false)
let seq = 0

watch(
  debouncedInput,
  async (text) => {
    const ticket = ++seq
    computing.value = true
    try {
      const data = new TextEncoder().encode(text)
      const items = await Promise.all(
        ALGOS.map(async (algo) => {
          const digest = await crypto.subtle.digest(algo, data)
          return { name: algo, hex: bufferToHex(digest), b64: bufferToBase64(digest) }
        })
      )
      if (ticket !== seq) return
      results.value = items
    } catch {
      if (ticket === seq) {
        results.value = []
        api.showToast('哈希计算失败', 'error')
      }
    } finally {
      if (ticket === seq) computing.value = false
    }
  },
  { immediate: true }
)

function reset() {
  input.value = ''
}
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">哈希计算</div>
          <div class="tb-desc">SHA-1 / SHA-256 / SHA-512 · 输入即算 · HEX 与 Base64 双输出</div>
        </div>
        <span class="tb-flex" />
        <Button variant="ghost" size="sm" class="tb-btn ghost" @click="reset">重置</Button>
      </div>

      <textarea
        v-model="input"
        class="tb-textarea hash-input"
        spellcheck="false"
        placeholder="输入任意文本，实时计算哈希（按 UTF-8 编码）…"
      ></textarea>

      <div v-if="computing" class="tb-status hint">计算中…</div>

      <div v-for="r in results" :key="r.name" class="hash-block">
        <div class="hash-name">{{ r.name }}</div>
        <div class="hash-line">
          <span class="hash-k">HEX</span>
          <code class="tb-mono">{{ r.hex }}</code>
          <Button variant="outline" size="sm" class="tb-btn sm" @click="api.copyText(r.hex, `已复制 ${r.name} HEX`)">复制</Button>
        </div>
        <div class="hash-line">
          <span class="hash-k">Base64</span>
          <code class="tb-mono">{{ r.b64 }}</code>
          <Button variant="outline" size="sm" class="tb-btn sm" @click="api.copyText(r.b64, `已复制 ${r.name} Base64`)">
            复制
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.hash-input {
  min-height: 84px;
}

.hash-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.hash-name {
  font-size: 12px;
  font-weight: 600;
  color: color-mix(in srgb, var(--primary) 55%, var(--text));
}
.hash-line {
  display: grid;
  grid-template-columns: 46px 1fr auto;
  align-items: center;
  gap: 8px;
  padding: 6px 9px;
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--surface) 65%, transparent);
}
.hash-k {
  font-size: 10.5px;
  color: var(--text-muted);
}
.hash-line code {
  min-width: 0;
  font-size: 12px;
  word-break: break-all;
  user-select: text;
}
</style>
