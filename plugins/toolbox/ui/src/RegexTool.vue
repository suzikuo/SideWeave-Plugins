<script setup lang="ts">
/** 正则测试：pattern + flags 输入，mark 包裹高亮全部匹配并统计匹配数 */
import { computed, inject, ref } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY, debouncedRef } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const SAMPLE_PATTERN = '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{2,}'
const SAMPLE_TEXT = '联系 support@example.com 或 sales@shop.org\n电话：12345\n备用：dev-team@corp.cn'

const pattern = ref(SAMPLE_PATTERN)
const text = ref(SAMPLE_TEXT)
const activeFlags = ref<string[]>(['g'])

const FLAG_DEFS = [
  { key: 'g', label: 'g 全局', title: '全局匹配（高亮所有结果）' },
  { key: 'i', label: 'i 忽略大小写', title: '忽略大小写' },
  { key: 'm', label: 'm 多行', title: '^ $ 匹配行首行尾' },
  { key: 's', label: 's 点号通配', title: '点号匹配换行符' },
  { key: 'u', label: 'u Unicode', title: 'Unicode 模式' }
]

function toggleFlag(key: string) {
  const i = activeFlags.value.indexOf(key)
  if (i >= 0) activeFlags.value.splice(i, 1)
  else activeFlags.value.push(key)
}

const flagsStr = computed(() =>
  FLAG_DEFS.map((f) => f.key)
    .filter((k) => activeFlags.value.includes(k))
    .join('')
)

const debouncedPattern = debouncedRef(pattern, 200)
const debouncedText = debouncedRef(text, 200)

interface Segment {
  key: number
  text: string
  match: boolean
}

interface RegexState {
  segments: Segment[]
  count: number
  truncated: boolean
  error: string
  hasPattern: boolean
  hasText: boolean
}

/** 渲染段上限：防止灾难性输入把详情卡卡死 */
const MAX_SEGMENTS = 3000

const state = computed<RegexState>(() => {
  const patternText = debouncedPattern.value
  const source = debouncedText.value
  const result: RegexState = {
    segments: [],
    count: 0,
    truncated: false,
    error: '',
    hasPattern: patternText.trim().length > 0,
    hasText: source.length > 0
  }
  if (!result.hasPattern) {
    result.segments = source ? [{ key: 0, text: source, match: false }] : []
    return result
  }

  let re: RegExp
  try {
    re = new RegExp(patternText, flagsStr.value)
  } catch (e) {
    result.error = e instanceof Error ? e.message : '正则表达式语法错误'
    return result
  }
  if (!source) return result

  const segments: Segment[] = []
  let last = 0
  let count = 0
  let truncated = false

  const pushMatch = (start: number, length: number) => {
    if (start > last) segments.push({ key: segments.length, text: source.slice(last, start), match: false })
    segments.push({ key: segments.length, text: source.slice(start, start + length), match: true })
    last = start + length
    count++
  }

  if (re.global) {
    // matchAll 不依赖 / 不前进原正则的 lastIndex，空匹配也会自动步进
    for (const m of source.matchAll(re)) {
      pushMatch(m.index ?? 0, m[0].length)
      if (segments.length > MAX_SEGMENTS) {
        truncated = true
        break
      }
    }
  } else {
    const m = re.exec(source)
    if (m) pushMatch(m.index ?? 0, m[0].length)
  }
  if (last < source.length) segments.push({ key: segments.length, text: source.slice(last), match: false })

  result.segments = segments
  result.count = count
  result.truncated = truncated
  return result
})

const statusText = computed(() => {
  const s = state.value
  if (s.error) return s.error
  if (!s.hasPattern) return '输入正则表达式与测试文本'
  if (!s.hasText) return '输入测试文本以查看匹配'
  return `共 ${s.count}${s.truncated ? '+' : ''} 处匹配${s.truncated ? '（仅渲染前 3000 段）' : ''}`
})

/** 复制全部匹配片段（换行分隔；超过渲染上限时仅含已渲染部分） */
function copyMatches() {
  const matches = state.value.segments
    .filter((s) => s.match)
    .map((s) => s.text)
    .join('\n')
  api.copyText(matches, `已复制 ${state.value.count} 处匹配`)
}

function reset() {
  pattern.value = SAMPLE_PATTERN
  text.value = SAMPLE_TEXT
  activeFlags.value = ['g']
}
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">正则测试</div>
          <div class="tb-desc">输入即匹配，高亮所有结果并计数</div>
        </div>
        <span class="tb-flex" />
        <button class="tb-btn ghost" @click="reset">重置</button>
      </div>

      <div class="regex-row">
        <span class="tb-mono slash">/</span>
        <input
          v-model="pattern"
          class="tb-input tb-mono regex-pattern"
          spellcheck="false"
          placeholder="正则表达式，例如 \d+"
        />
        <span class="tb-mono slash">/{{ flagsStr }}</span>
      </div>

      <div class="flags">
        <button
          v-for="f in FLAG_DEFS"
          :key="f.key"
          class="flag-chip"
          :class="{ on: activeFlags.includes(f.key) }"
          :title="f.title"
          @click="toggleFlag(f.key)"
        >
          {{ f.label }}
        </button>
      </div>

      <div class="tb-label-row"><span class="tb-label">测试文本</span></div>
      <textarea
        v-model="text"
        class="tb-textarea regex-text"
        spellcheck="false"
        placeholder="粘贴测试文本…"
      ></textarea>

      <div class="tb-label-row">
        <span class="tb-label">匹配结果</span>
        <span class="tb-flex" />
        <button class="tb-btn sm" :disabled="!state.count" @click="copyMatches">复制匹配</button>
        <span class="tb-status" :class="state.error ? 'err' : state.hasPattern && state.hasText ? 'ok' : 'hint'">
          {{ statusText }}
        </span>
      </div>
      <pre v-if="state.error" class="tb-out regex-view error">{{ state.error }}</pre>
      <pre v-else-if="!state.hasText" class="tb-out regex-view empty">匹配结果将在此高亮显示</pre>
      <pre v-else class="tb-out regex-view"><template v-for="seg in state.segments" :key="seg.key"><mark v-if="seg.match" class="tb-mark">{{ seg.text }}</mark><template v-else>{{ seg.text }}</template></template></pre>
    </div>
  </div>
</template>

<style scoped>
.regex-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.slash {
  color: var(--text-muted);
  font-size: 13px;
}
.regex-pattern {
  flex: 1;
  min-width: 0;
}

.flags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.flag-chip {
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: transparent;
  color: var(--text-muted);
  font-size: 11.5px;
  transition: border-color 0.15s, background 0.15s;
}
.flag-chip:hover {
  border-color: var(--border-strong);
}
.flag-chip.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: color-mix(in srgb, var(--accent) 60%, var(--text));
}

.regex-text {
  min-height: 96px;
}
.regex-view {
  min-height: 90px;
  max-height: 300px;
}
</style>
