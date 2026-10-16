<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 密码生成器：长度/字符集可配，crypto.getRandomValues 均匀取样，批量生成 + 点击复制 */
import { inject, onMounted, ref, computed } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const lengthText = ref('16')
const countText = ref('5')
const useUpper = ref(true)
const useLower = ref(true)
const useDigits = ref(true)
const useSymbols = ref(true)
const avoidAmbiguous = ref(true)
const list = ref<string[]>([])

const UPPER = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'
const LOWER = 'abcdefghijklmnopqrstuvwxyz'
const DIGITS = '0123456789'
const SYMBOLS = '!@#$%^&*()-_=+[]{};:,.<>?/'
const AMBIGUOUS = new Set(['0', 'O', '1', 'l', 'I', '|'])

/** 均匀随机整数（拒绝采样，避免取模偏差） */
function randInt(max: number): number {
  const range = 4294967296 // 2^32
  const limit = range - (range % max)
  const buf = new Uint32Array(1)
  let v = 0
  do {
    crypto.getRandomValues(buf)
    v = buf[0]
  } while (v >= limit)
  return v % max
}

function pick(pool: string): string {
  return pool[randInt(pool.length)]
}

const pool = computed(() => {
  let p = ''
  if (useUpper.value) p += UPPER
  if (useLower.value) p += LOWER
  if (useDigits.value) p += DIGITS
  if (useSymbols.value) p += SYMBOLS
  if (avoidAmbiguous.value) p = [...p].filter((ch) => !AMBIGUOUS.has(ch)).join('')
  return p
})

/** 每类至少一个（选中了的类），打乱后输出 */
function generateOne(length: number): string {
  const pools: string[] = []
  if (useUpper.value && (!avoidAmbiguous.value || UPPER.split('').some((ch) => !AMBIGUOUS.has(ch)))) pools.push(UPPER)
  if (useLower.value) pools.push(LOWER)
  if (useDigits.value && (!avoidAmbiguous.value || DIGITS.split('').some((ch) => !AMBIGUOUS.has(ch)))) pools.push(DIGITS)
  if (useSymbols.value) pools.push(SYMBOLS)
  const all = pool.value
  if (!all) return ''
  const chars: string[] = pools.filter((p) => p).map((p) => pick([...p].filter((ch) => !avoidAmbiguous.value || !AMBIGUOUS.has(ch)).join('') || p))
  while (chars.length < length) chars.push(pick(all))
  // Fisher-Yates 打乱
  for (let i = chars.length - 1; i > 0; i--) {
    const j = randInt(i + 1)
    ;[chars[i], chars[j]] = [chars[j], chars[i]]
  }
  return chars.slice(0, length).join('')
}

const strength = computed(() => {
  const len = Math.floor(Number(lengthText.value)) || 16
  let classes = 0
  if (useUpper.value) classes++
  if (useLower.value) classes++
  if (useDigits.value) classes++
  if (useSymbols.value) classes++
  const bits = Math.round(len * Math.log2(Math.max(pool.value.length, 2)))
  if (bits >= 100) return { label: '极强', pct: 100 }
  if (bits >= 80) return { label: '强', pct: 80 }
  if (bits >= 60) return { label: '中', pct: 60 }
  return { label: '弱', pct: Math.max(20, bits) }
})

function generate() {
  const all = pool.value
  if (!all) {
    list.value = []
    return
  }
  const len = Math.min(128, Math.max(4, Math.floor(Number(lengthText.value)) || 16))
  lengthText.value = String(len)
  const n = Math.min(50, Math.max(1, Math.floor(Number(countText.value)) || 5))
  countText.value = String(n)
  list.value = Array.from({ length: n }, () => generateOne(len))
}

function copyAll() {
  api.copyText(list.value.join('\n'), `已复制 ${list.value.length} 个密码`)
}

onMounted(generate)
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">密码生成</div>
          <div class="tb-desc">crypto.getRandomValues 均匀取样 · 每类至少一位 · 点击单条复制</div>
        </div>
        <span class="tb-flex" />
        <Button variant="default" size="sm" class="tb-btn primary" @click="generate">重新生成</Button>
      </div>

      <div class="opt-grid">
        <label class="tb-label">长度 <input v-model="lengthText" class="tb-input len-input" inputmode="numeric" /></label>
        <label class="tb-label">数量 <input v-model="countText" class="tb-input len-input" inputmode="numeric" /></label>
        <label class="tb-check"><input v-model="useUpper" type="checkbox" /> 大写 A-Z</label>
        <label class="tb-check"><input v-model="useLower" type="checkbox" /> 小写 a-z</label>
        <label class="tb-check"><input v-model="useDigits" type="checkbox" /> 数字 0-9</label>
        <label class="tb-check"><input v-model="useSymbols" type="checkbox" /> 符号</label>
        <label class="tb-check"><input v-model="avoidAmbiguous" type="checkbox" /> 排除易混淆 0O1lI</label>
      </div>

      <div class="strength">
        <div class="strength-bar"><div class="strength-fill" :style="{ width: strength.pct + '%' }" /></div>
        <span class="strength-label">强度：{{ strength.label }}（约 {{ strength.pct }} 位熵）</span>
      </div>

      <div class="pwd-list">
        <div v-for="(pwd, i) in list" :key="i" class="pwd-item" :title="pwd" @click="api.copyText(pwd, '已复制')">
          <span class="pwd-text">{{ pwd }}</span>
        </div>
      </div>
      <p v-if="list.length" class="tb-hint">点击任意密码复制；全部复制：</p>
      <Button variant="outline" size="sm" v-if="list.length" class="tb-btn" @click="copyAll">复制全部</Button>
    </div>
  </div>
</template>

<style scoped>
.opt-grid {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.len-input {
  width: 56px;
}
.tb-check {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 12.5px;
  color: var(--text);
  cursor: pointer;
}
.tb-check input {
  accent-color: var(--primary);
}
.strength {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}
.strength-bar {
  flex: 1;
  height: 6px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border) 60%, transparent);
  overflow: hidden;
}
.strength-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--primary);
  transition: width 0.2s ease;
}
.strength-label {
  font-size: 11.5px;
  color: var(--text-muted);
  white-space: nowrap;
}
.pwd-list {
  margin-bottom: 8px;
}
.pwd-item {
  padding: 8px 12px;
  margin-bottom: 6px;
  border: 1px solid var(--border);
  border-radius: 9px;
  cursor: pointer;
  font-family: 'Cascadia Mono', Consolas, monospace;
  font-size: 12.5px;
  word-break: break-all;
}
.pwd-item:hover {
  border-color: var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}
.pwd-text {
  color: var(--text);
  user-select: all;
}
</style>
