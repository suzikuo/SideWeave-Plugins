<script setup lang="ts">
/** JSON 工具：格式化（2 空格缩进）/ 压缩 / 校验，错误定位到行列，输入即时计算 */
import { computed, inject, ref, watch } from 'vue'
import {
  FALLBACK_API,
  TOOLBOX_API_KEY,
  debouncedRef,
  tryParseJson,
  type JsonIssue
} from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const SAMPLE = '{\n  "name": "工具箱",\n  "version": 1,\n  "items": ["json", "base64"]\n}'

const input = ref(SAMPLE)
const mode = ref<'pretty' | 'min'>('pretty')
const debouncedInput = debouncedRef(input, 200)

const output = ref('')
const issue = ref<JsonIssue | null>(null)
const summary = ref('')

function typeName(v: unknown): string {
  if (v === null) return 'null'
  if (Array.isArray(v)) return '数组'
  const t = typeof v
  if (t === 'object') return '对象'
  if (t === 'string') return '字符串'
  if (t === 'number') return '数字'
  if (t === 'boolean') return '布尔'
  return t
}

/** 按当前模式处理输入：成功写输出 + 摘要，失败记错误对象 */
function process(text: string) {
  if (!text.trim()) {
    output.value = ''
    issue.value = null
    summary.value = '等待输入'
    return
  }
  const { value, error } = tryParseJson(text)
  if (error) {
    issue.value = error
    output.value = ''
    summary.value = '解析失败'
    return
  }
  issue.value = null
  output.value = mode.value === 'pretty' ? JSON.stringify(value, null, 2) : JSON.stringify(value)
  summary.value =
    mode.value === 'pretty'
      ? `${typeName(value)} · ${output.value.split('\n').length} 行`
      : `${typeName(value)} · ${output.value.length} 字符`
}

watch(debouncedInput, process, { immediate: true })

/** 格式化 / 压缩：切换模式并立即以当前输入计算（不等防抖） */
function setMode(next: 'pretty' | 'min') {
  mode.value = next
  process(input.value)
}

function validate() {
  if (!input.value.trim()) {
    api.showToast('请先输入 JSON 内容', 'error')
    return
  }
  const { error } = tryParseJson(input.value)
  if (error) api.showToast(`JSON 无效：第 ${error.line} 行 第 ${error.column} 列`, 'error')
  else api.showToast('JSON 校验通过', 'success')
}

function reset() {
  input.value = SAMPLE
  mode.value = 'pretty'
  process(input.value)
}

const statusText = computed(() =>
  issue.value ? `第 ${issue.value.line} 行 第 ${issue.value.column} 列：${issue.value.message}` : summary.value
)
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">JSON 工具</div>
          <div class="tb-desc">格式化 / 压缩 / 校验，输入即时计算</div>
        </div>
        <span class="tb-flex" />
        <button class="tb-btn" :class="{ active: mode === 'pretty' }" @click="setMode('pretty')">
          格式化
        </button>
        <button class="tb-btn" :class="{ active: mode === 'min' }" @click="setMode('min')">
          压缩
        </button>
        <button class="tb-btn" @click="validate">校验</button>
        <button class="tb-btn ghost" @click="reset">重置</button>
      </div>

      <div class="tb-cols">
        <div class="tb-col">
          <div class="tb-label-row"><span class="tb-label">输入</span></div>
          <textarea
            v-model="input"
            class="tb-textarea json-area"
            spellcheck="false"
            placeholder="在此粘贴 JSON…"
          ></textarea>
        </div>
        <div class="tb-col">
          <div class="tb-label-row">
            <span class="tb-label">输出</span>
            <span class="tb-flex" />
            <button class="tb-btn sm" :disabled="!output" @click="api.copyText(output, '已复制输出')">
              复制
            </button>
          </div>
          <pre v-if="issue" class="tb-out json-area error">{{ statusText }}</pre>
          <pre v-else class="tb-out json-area" :class="{ empty: !output }">{{ output || '输出将显示在这里' }}</pre>
        </div>
      </div>

      <div class="tb-status" :class="issue ? 'err' : input.trim() ? 'ok' : 'hint'">
        {{ statusText }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.json-area {
  height: 240px;
  min-height: 160px;
}

@media (max-width: 640px) {
  .json-area {
    height: 180px;
  }
}
</style>
