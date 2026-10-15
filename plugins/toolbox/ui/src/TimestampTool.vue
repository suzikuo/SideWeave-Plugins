<script setup lang="ts">
/** 时间戳工具：当前秒/毫秒实时走秒 + 双向转换（时间戳自动识别秒/毫秒） */
import { computed, inject, onActivated, onDeactivated, onMounted, onUnmounted, ref } from 'vue'
import { onPluginLifecycle } from '../../../ui-sdk/runtime'
import {
  FALLBACK_API,
  TOOLBOX_API_KEY,
  debouncedRef,
  formatLocal,
  toLocalInputValue
} from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const WEEK_CHARS = ['日', '一', '二', '三', '四', '五', '六']

// ---------- 当前时间戳（KeepAlive 激活期间走秒） ----------

const nowMs = ref(Date.now())
let ticker = 0
let hostActive = false
let componentActive = false

function startTicker() {
  if (!hostActive || !componentActive || ticker) return
  stopTicker()
  nowMs.value = Date.now()
  ticker = window.setInterval(() => {
    nowMs.value = Date.now()
  }, 1000)
}
function stopTicker() {
  if (ticker) {
    window.clearInterval(ticker)
    ticker = 0
  }
}

function syncTicker() {
  if (hostActive && componentActive) startTicker()
  else stopTicker()
}

const unlistenLifecycle = onPluginLifecycle((active) => {
  hostActive = active
  syncTicker()
})

onMounted(() => {
  componentActive = true
  syncTicker()
})
onUnmounted(() => {
  componentActive = false
  stopTicker()
  unlistenLifecycle()
})
onActivated(() => {
  componentActive = true
  syncTicker()
})
onDeactivated(() => {
  componentActive = false
  stopTicker()
})

const nowSecs = computed(() => String(Math.floor(nowMs.value / 1000)))
const nowMillis = computed(() => String(nowMs.value))
const nowLocal = computed(() => formatLocal(new Date(nowMs.value)))

// ---------- 时间戳 → 日期 ----------

const tsInput = ref(String(Math.floor(Date.now() / 1000)))
const debouncedTs = debouncedRef(tsInput, 200)

interface TsParse {
  ok: boolean
  hint: string
  date?: Date
}

const tsParsed = computed<TsParse>(() => {
  const raw = debouncedTs.value.trim()
  if (!raw) return { ok: false, hint: '输入数字时间戳，≤11 位按秒、更长按毫秒解析' }
  if (!/^-?\d{1,16}$/.test(raw)) return { ok: false, hint: '格式不正确：仅支持纯数字（可带负号）' }
  const unit: '秒' | '毫秒' = raw.replace(/^-/, '').length <= 11 ? '秒' : '毫秒'
  const ms = unit === '秒' ? Number(raw) * 1000 : Number(raw)
  const date = new Date(ms)
  if (!Number.isFinite(date.getTime()) || Math.abs(date.getFullYear()) > 9999) {
    return { ok: false, hint: '数值超出可表示的时间范围' }
  }
  return { ok: true, hint: `按 ${unit} 解析`, date }
})

const tsLocal = computed(() =>
  tsParsed.value.ok && tsParsed.value.date ? formatLocal(tsParsed.value.date) : '—'
)
const tsIso = computed(() =>
  tsParsed.value.ok && tsParsed.value.date ? tsParsed.value.date.toISOString() : '—'
)
const tsWeek = computed(() =>
  tsParsed.value.ok && tsParsed.value.date
    ? `星期${WEEK_CHARS[tsParsed.value.date.getDay()] ?? ''}`
    : '—'
)

// ---------- 日期 → 时间戳 ----------

const dateValue = ref(toLocalInputValue(new Date()))

const dateParsed = computed<Date | null>(() => {
  if (!dateValue.value) return null
  const d = new Date(dateValue.value)
  return Number.isNaN(d.getTime()) ? null : d
})
const dateSecs = computed(() =>
  dateParsed.value ? String(Math.floor(dateParsed.value.getTime() / 1000)) : '—'
)
const dateMillis = computed(() =>
  dateParsed.value ? String(dateParsed.value.getTime()) : '—'
)

// ---------- 重置 ----------

function reset() {
  nowMs.value = Date.now()
  tsInput.value = String(Math.floor(Date.now() / 1000))
  dateValue.value = toLocalInputValue(new Date())
}
</script>

<template>
  <div class="tb-stack">
    <!-- 当前时间戳 -->
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">当前时间戳</div>
          <div class="tb-desc">本地时区 · 实时走秒</div>
        </div>
        <span class="tb-flex" />
      </div>
      <div class="now-grid">
        <div class="now-cell">
          <span class="now-k">秒</span>
          <code class="tb-mono now-v">{{ nowSecs }}</code>
          <button class="tb-btn sm" @click="api.copyText(nowSecs, '已复制秒级时间戳')">复制</button>
        </div>
        <div class="now-cell">
          <span class="now-k">毫秒</span>
          <code class="tb-mono now-v">{{ nowMillis }}</code>
          <button class="tb-btn sm" @click="api.copyText(nowMillis, '已复制毫秒时间戳')">复制</button>
        </div>
      </div>
      <div class="tb-status hint">此刻：{{ nowLocal }}</div>
    </div>

    <!-- 双向转换 -->
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">双向转换</div>
          <div class="tb-desc">输入即时换算，自动识别秒 / 毫秒</div>
        </div>
        <span class="tb-flex" />
        <button class="tb-btn ghost" @click="reset">重置</button>
      </div>

      <div class="tb-cols">
        <div class="tb-col">
          <div class="tb-label-row"><span class="tb-label">时间戳 → 本地时间</span></div>
          <input
            v-model="tsInput"
            class="tb-input tb-mono"
            inputmode="numeric"
            spellcheck="false"
            placeholder="例如 1735689600 或 1735689600000"
          />
          <div class="tb-status" :class="tsParsed.ok ? 'ok' : tsInput.trim() ? 'err' : 'hint'">
            {{ tsParsed.hint }}
          </div>
          <div class="tb-kv">
            <span class="k">本地</span>
            <code class="tb-mono">{{ tsLocal }}</code>
            <button class="tb-btn sm" :disabled="!tsParsed.ok" @click="api.copyText(tsLocal)">
              复制
            </button>
          </div>
          <div class="tb-kv">
            <span class="k">ISO</span>
            <code class="tb-mono">{{ tsIso }}</code>
            <button class="tb-btn sm" :disabled="!tsParsed.ok" @click="api.copyText(tsIso)">
              复制
            </button>
          </div>
          <div class="tb-kv">
            <span class="k">星期</span>
            <code>{{ tsWeek }}</code>
          </div>
        </div>

        <div class="tb-col">
          <div class="tb-label-row"><span class="tb-label">日期时间 → 时间戳</span></div>
          <input v-model="dateValue" type="datetime-local" step="1" class="tb-input tb-mono" />
          <div class="tb-status" :class="dateParsed ? 'ok' : dateValue ? 'err' : 'hint'">
            {{ dateParsed ? '按本地时区解析' : dateValue ? '日期无效' : '请选择日期时间' }}
          </div>
          <div class="tb-kv">
            <span class="k">秒</span>
            <code class="tb-mono">{{ dateSecs }}</code>
            <button class="tb-btn sm" :disabled="!dateParsed" @click="api.copyText(dateSecs)">
              复制
            </button>
          </div>
          <div class="tb-kv">
            <span class="k">毫秒</span>
            <code class="tb-mono">{{ dateMillis }}</code>
            <button class="tb-btn sm" :disabled="!dateParsed" @click="api.copyText(dateMillis)">
              复制
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.now-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}
.now-cell {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--surface);
}
.now-k {
  flex: none;
  font-size: 11.5px;
  color: var(--text-muted);
}
.now-v {
  flex: 1;
  min-width: 0;
  font-size: 19px;
  font-weight: 700;
  letter-spacing: 0.5px;
  overflow-wrap: anywhere;
  user-select: text;
}

@media (max-width: 560px) {
  .now-grid {
    grid-template-columns: 1fr;
  }
}
</style>
