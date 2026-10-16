<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 全局热键录制器：展示态键帽 + 录制态捕获 keydown 生成组合串，由父组件负责持久化 */
import { computed, onUnmounted, ref } from 'vue'

const props = defineProps<{
  /** 当前热键串，如 "alt+space" */
  modelValue: string
}>()

const emit = defineEmits<{ (e: 'commit', value: string): void }>()

// ---------- 录制状态 ----------

const recording = ref(false)
/** 录制中已按下的修饰键（去重集合） */
const draftMods = ref<Set<string>>(new Set())

const MODIFIER_CODES = new Set([
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'ShiftLeft',
  'ShiftRight',
  'MetaLeft',
  'MetaRight'
])

/** KeyboardEvent → 规范化修饰键名（与 Rust 侧 ctrl/alt/shift/meta 对齐） */
function modifierOf(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code) || e.key === 'AltGraph') {
    if (e.code.startsWith('Control')) return 'ctrl'
    if (e.code.startsWith('Alt') || e.key === 'AltGraph') return 'alt'
    if (e.code.startsWith('Shift')) return 'shift'
    if (e.code.startsWith('Meta')) return 'meta'
    return null
  }
  switch (e.key) {
    case 'Control':
      return 'ctrl'
    case 'Alt':
      return 'alt'
    case 'Shift':
      return 'shift'
    case 'Meta':
      return 'meta'
    default:
      return null
  }
}

/** 非修饰键 → 组合串主键：字母/数字小写、空格=space、其余剥 Key/Digit 前缀，F 键与剩余 code 小写保留 */
function mainKeyOf(e: KeyboardEvent): string {
  if (e.code.startsWith('Key') && e.code.length === 4) return e.code.slice(3).toLowerCase()
  if (e.code.startsWith('Digit') && e.code.length === 6) return e.code.slice(5).toLowerCase()
  if (e.key === ' ') return 'space'
  if (/^F\d{1,2}$/.test(e.code)) return e.code.toLowerCase()
  return e.code.toLowerCase()
}

/** 修饰键按 ctrl/alt/shift/meta 顺序去重拼接 */
function buildCombo(main: string): string {
  const order = ['ctrl', 'alt', 'shift', 'meta']
  const mods = order.filter((m) => draftMods.value.has(m))
  return [...mods, main].join('+')
}

// ---------- 事件处理（录制期间挂到 window capture，拦截一切按键） ----------

function onKeyDownCapture(e: KeyboardEvent) {
  e.preventDefault()
  e.stopPropagation()

  if (e.code === 'Escape') {
    cancelRecording()
    return
  }
  if (e.code === 'Backspace') {
    // 清空草稿，继续录制
    draftMods.value = new Set()
    return
  }

  const mod = modifierOf(e)
  if (mod) {
    const next = new Set(draftMods.value)
    next.add(mod)
    draftMods.value = next
    return
  }

  // 首个非修饰键：立即生成组合并提交
  const value = buildCombo(mainKeyOf(e))
  stopRecording()
  emit('commit', value)
}

/** 修饰键抬起时同步草稿，避免"按下后松开"残留在组合里 */
function onKeyUpCapture(e: KeyboardEvent) {
  e.preventDefault()
  e.stopPropagation()
  const mod = modifierOf(e)
  if (!mod) return
  const next = new Set(draftMods.value)
  next.delete(mod)
  draftMods.value = next
}

/** 录制期间窗口失焦（如误触系统级 Alt+Tab）直接取消，避免残留脏状态 */
function onBlurWindow() {
  if (recording.value) cancelRecording()
}

function startRecording() {
  draftMods.value = new Set()
  recording.value = true
  window.addEventListener('keydown', onKeyDownCapture, true)
  window.addEventListener('keyup', onKeyUpCapture, true)
  window.addEventListener('blur', onBlurWindow)
}

function stopRecording() {
  recording.value = false
  draftMods.value = new Set()
  window.removeEventListener('keydown', onKeyDownCapture, true)
  window.removeEventListener('keyup', onKeyUpCapture, true)
  window.removeEventListener('blur', onBlurWindow)
}

function cancelRecording() {
  stopRecording()
}

onUnmounted(stopRecording)

// ---------- 展示 ----------

const HOTKEY_LABELS: Record<string, string> = {
  ctrl: 'Ctrl',
  alt: 'Alt',
  shift: 'Shift',
  meta: 'Meta',
  space: 'Space'
}

/** 键帽文案：修饰键/空格用惯用名，单字符与 F 键大写，其余首字母大写 */
function capLabel(seg: string): string {
  const s = seg.trim()
  if (!s) return ''
  const lower = s.toLowerCase()
  if (HOTKEY_LABELS[lower]) return HOTKEY_LABELS[lower]
  if (lower.length === 1 || /^f\d{1,2}$/.test(lower)) return lower.toUpperCase()
  return lower.charAt(0).toUpperCase() + lower.slice(1)
}

/** 展示分段：录制中显示已按下的修饰键草稿，否则显示当前值 */
const caps = computed<string[]>(() => {
  if (recording.value) {
    const order = ['ctrl', 'alt', 'shift', 'meta']
    return order.filter((m) => draftMods.value.has(m))
  }
  return props.modelValue
    .split('+')
    .map((s) => s.trim())
    .filter(Boolean)
})
</script>

<template>
  <div class="hotkey-recorder">
    <template v-if="recording">
      <span v-if="caps.length" class="caps draft">
        <kbd v-for="seg in caps" :key="seg">{{ capLabel(seg) }}</kbd>
        <em>+ …</em>
      </span>
      <span v-else class="rec-hint">请按下新的组合键（Esc 取消）</span>
      <Button variant="ghost" size="sm" class="btn ghost" type="button" @click="cancelRecording">取消</Button>
    </template>
    <template v-else>
      <span v-if="caps.length" class="caps">
        <kbd v-for="seg in caps" :key="seg">{{ capLabel(seg) }}</kbd>
      </span>
      <span v-else class="rec-hint">未设置</span>
      <Button variant="outline" size="sm" class="btn" type="button" @click="startRecording">修改</Button>
    </template>
  </div>
</template>

<style scoped>
.hotkey-recorder {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}

.caps {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.caps kbd {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-bottom-width: 2px;
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
  font-family: inherit;
  font-size: 10.5px;
  white-space: nowrap;
}

/* 录制中的草稿键帽：强调色描边 + 轻微脉动提示 */
.caps.draft kbd {
  border-color: var(--primary);
  color: var(--primary);
}

.caps em {
  font-style: normal;
  font-size: 10.5px;
  color: var(--text-muted);
}

.rec-hint {
  font-size: 11px;
  color: var(--primary);
}

.btn {
  display: inline-flex;
  align-items: center;
  padding: 5px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-size: 11.5px;
  white-space: nowrap;
}
.btn:hover {
  border-color: var(--primary);
  color: var(--primary);
}
.btn.ghost {
  background: transparent;
  color: var(--text-muted);
}
</style>
