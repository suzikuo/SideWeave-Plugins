<script setup lang="ts">
import { ref } from 'vue'
import { quickTranslate, type ScreenshotSettings, type TranslateEngine } from '../api'

const props = defineProps<{
  settings: ScreenshotSettings
}>()

const emit = defineEmits<{
  toast: [message: string]
}>()

const sourceLang = ref('auto')
const targetLang = ref(props.settings.translateTargetLang || 'zh-CN')
const provider = ref<TranslateEngine>(props.settings.translateEngine || 'youdao')
const sourceText = ref('')
const resultText = ref('')
const providerUsed = ref('')
const loading = ref(false)
const history = ref<{ src: string; dst: string; engine: string; time: string }[]>([])

const langs = [
  { code: 'auto', label: '自动检测' },
  { code: 'zh-CN', label: '简体中文' },
  { code: 'zh-TW', label: '繁体中文' },
  { code: 'en', label: 'English' },
  { code: 'ja', label: '日本語' },
  { code: 'ko', label: '한국어' },
  { code: 'fr', label: 'Français' },
  { code: 'de', label: 'Deutsch' },
  { code: 'ru', label: 'Русский' },
]

const engines: { id: TranslateEngine; label: string }[] = [
  { id: 'youdao', label: '有道翻译' },
  { id: 'microsoft', label: '微软翻译' },
  { id: 'google', label: '谷歌翻译' },
  { id: 'baidu', label: '百度翻译' },
]

function swapLanguages() {
  if (sourceLang.value === 'auto') {
    sourceLang.value = targetLang.value
    targetLang.value = targetLang.value === 'en' ? 'zh-CN' : 'en'
  } else {
    const tmp = sourceLang.value
    sourceLang.value = targetLang.value
    targetLang.value = tmp
  }
  if (resultText.value) {
    sourceText.value = resultText.value
    resultText.value = ''
  }
}

async function runTranslate() {
  const input = sourceText.value.trim()
  if (!input || loading.value) return
  loading.value = true
  try {
    const resp = await quickTranslate(input, sourceLang.value, targetLang.value, provider.value)
    resultText.value = resp.text
    providerUsed.value = resp.providerUsed
    history.value.unshift({
      src: input.slice(0, 80),
      dst: resp.text.slice(0, 120),
      engine: resp.providerUsed,
      time: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
    })
    if (history.value.length > 10) history.value.pop()
  } catch (err) {
    emit('toast', `翻译失败：${String(err)}`)
  } finally {
    loading.value = false
  }
}

async function copyResult() {
  if (!resultText.value) return
  try {
    await navigator.clipboard.writeText(resultText.value)
    emit('toast', '译文已复制到剪贴板')
  } catch {
    emit('toast', '复制失败')
  }
}

function onKeyDown(e: KeyboardEvent) {
  if (e.ctrlKey && e.key === 'Enter') {
    e.preventDefault()
    void runTranslate()
  }
}
</script>

<template>
  <div class="sn-panel">
    <div class="sn-section-head">
      <div>
        <h2 class="sn-title">快捷翻译</h2>
        <p class="sn-subtitle">支持有道 / 微软 / 谷歌 / 百度四引擎自动回退，按 Ctrl + Enter 快速翻译</p>
      </div>
    </div>

    <div class="tr-toolbar">
      <div class="tr-lang-group">
        <select v-model="sourceLang" class="sn-select">
          <option v-for="l in langs" :key="l.code" :value="l.code">{{ l.label }}</option>
        </select>
        <button type="button" class="tr-swap-btn" title="互换语言" @click="swapLanguages">⇄</button>
        <select v-model="targetLang" class="sn-select">
          <option v-for="l in langs.filter(x => x.code !== 'auto')" :key="l.code" :value="l.code">
            {{ l.label }}
          </option>
        </select>
      </div>

      <div class="tr-engine-pills">
        <button
          v-for="eng in engines"
          :key="eng.id"
          type="button"
          class="tr-engine-pill"
          :class="{ active: provider === eng.id }"
          @click="provider = eng.id"
        >
          {{ eng.label }}
        </button>
      </div>
    </div>

    <div class="tr-grid">
      <div class="tr-box">
        <div class="tr-box-head">
          <span>原文</span>
          <button
            v-if="sourceText"
            type="button"
            class="tr-link"
            @click="sourceText = ''; resultText = ''"
          >
            清空
          </button>
        </div>
        <textarea
          v-model="sourceText"
          class="tr-textarea"
          placeholder="在此输入或粘贴需要翻译的文本，按 Ctrl + Enter 立即翻译…"
          @keydown="onKeyDown"
        />
        <div class="tr-box-foot">
          <span class="tr-count">{{ sourceText.length }} 字符</span>
          <button
            type="button"
            class="sn-primary-btn"
            :disabled="loading || !sourceText.trim()"
            @click="runTranslate"
          >
            {{ loading ? '翻译中…' : '立即翻译' }}
          </button>
        </div>
      </div>

      <div class="tr-box tr-box-out">
        <div class="tr-box-head">
          <span>
            译文
            <span v-if="providerUsed" class="sn-badge">{{ providerUsed }}</span>
          </span>
          <button
            v-if="resultText"
            type="button"
            class="tr-link accent"
            @click="copyResult"
          >
            复制译文
          </button>
        </div>
        <div class="tr-result" :class="{ empty: !resultText }">
          {{ resultText || '译文将显示在这里…' }}
        </div>
      </div>
    </div>

    <div v-if="history.length" class="tr-history">
      <div class="tr-history-title">本次翻译记录</div>
      <div class="tr-history-list">
        <div
          v-for="(item, idx) in history"
          :key="idx"
          class="tr-history-item"
          @click="sourceText = item.src; resultText = item.dst; providerUsed = item.engine"
        >
          <div class="tr-h-top">
            <span class="tr-h-src">{{ item.src }}</span>
            <span class="tr-h-meta">{{ item.engine }} · {{ item.time }}</span>
          </div>
          <div class="tr-h-dst">{{ item.dst }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tr-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 14px;
}
.tr-lang-group {
  display: flex;
  align-items: center;
  gap: 8px;
}
.tr-swap-btn {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-text);
  cursor: pointer;
  font-size: 14px;
}
.tr-swap-btn:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}
.tr-engine-pills {
  display: flex;
  padding: 3px;
  border-radius: 10px;
  background: var(--sn-subtle);
  border: 1px solid var(--sn-border);
  gap: 2px;
}
.tr-engine-pill {
  padding: 5px 10px;
  border-radius: 7px;
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.14s ease;
}
.tr-engine-pill.active {
  background: var(--sn-card);
  color: var(--sn-accent);
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.14);
}
.tr-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
@media (max-width: 720px) {
  .tr-grid {
    grid-template-columns: 1fr;
  }
}
.tr-box {
  display: flex;
  flex-direction: column;
  min-height: 240px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  overflow: hidden;
}
.tr-box-out {
  background: var(--sn-card);
}
.tr-box-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--sn-border);
  font-size: 12px;
  font-weight: 600;
  color: var(--sn-muted);
}
.tr-textarea {
  flex: 1;
  width: 100%;
  min-height: 150px;
  padding: 12px 14px;
  border: none;
  outline: none;
  resize: none;
  background: transparent;
  color: var(--sn-text);
  font-size: 13.5px;
  line-height: 1.6;
  font-family: inherit;
}
.tr-box-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-top: 1px solid var(--sn-border);
}
.tr-count {
  font-size: 11px;
  color: var(--sn-muted);
}
.tr-result {
  flex: 1;
  padding: 12px 14px;
  font-size: 13.5px;
  line-height: 1.65;
  color: var(--sn-text);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
}
.tr-result.empty {
  color: var(--sn-muted);
}
.tr-link {
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 12px;
  cursor: pointer;
}
.tr-link:hover,
.tr-link.accent {
  color: var(--sn-accent);
  font-weight: 600;
}
.tr-history {
  margin-top: 16px;
}
.tr-history-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--sn-muted);
  margin-bottom: 8px;
}
.tr-history-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.tr-history-item {
  padding: 9px 12px;
  border-radius: 10px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  cursor: pointer;
}
.tr-history-item:hover {
  border-color: var(--sn-accent);
}
.tr-h-top {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
  color: var(--sn-text);
  font-weight: 500;
}
.tr-h-meta {
  color: var(--sn-muted);
  font-size: 11px;
  white-space: nowrap;
}
.tr-h-dst {
  margin-top: 3px;
  font-size: 12px;
  color: var(--sn-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
