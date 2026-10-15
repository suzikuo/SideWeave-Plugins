<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { availableMonitors, getCurrentWindow, PhysicalPosition, primaryMonitor } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '../../../ui-sdk/runtime'
import {
  CURRENT_BOOK_KEY,
  THIEF_EVENTS,
  THIEF_STYLE_DEFAULT,
  thiefGetConfig,
  thiefRead,
  thiefSaveProgress,
  thiefSaveReaderPosition,
  thiefRememberReaderHidden,
  thiefNotifyProgress
} from './api'
import type { ThiefBook, ThiefConfig, ThiefStyle } from './api'

const style = ref<ThiefStyle>({ ...THIEF_STYLE_DEFAULT })
const book = ref<ThiefBook | null>(null)
const pageText = ref('')
const measureText = ref('')
const measureElement = ref<HTMLElement | null>(null)
const offset = ref(0)
const total = ref(0)
const nextOffset = ref(0)
const error = ref('')
const loading = ref(true)

const atEnd = computed(() => total.value > 0 && nextOffset.value >= total.value)
const rootStyle = computed(() => ({
  background: `rgba(10, 10, 12, ${style.value.opacity})`,
  color: style.value.color
}))

const MAX_READ_CHARS = 32_768
const WHEEL_MS = 140
const SAVE_DEBOUNCE = 500
let readSeq = 0
let saveTimer = 0
let saveDirty = false
let lastWheelAt = 0
let styleRenderTimer = 0
let autoTimer = 0
let positionSaveTimer = 0
let pendingPosition: { x: number; y: number } | null = null
const unlisteners: UnlistenFn[] = []
const pageHistory: number[] = []

function readSavedCurrent(): string | null {
  try {
    return localStorage.getItem(CURRENT_BOOK_KEY)
  } catch {
    return null
  }
}

function pickCurrent(cfg: ThiefConfig): ThiefBook | null {
  const saved = cfg.current_book_id ?? readSavedCurrent()
  return cfg.books.find((item) => item.id === saved) ?? cfg.books[0] ?? null
}

function charRange(node: Text, start: number, length: number): DOMRect {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, start + length)
  return range.getBoundingClientRect()
}

/** Find the first source character that would be laid out on a row beyond the window. */
async function visibleCharCount(text: string): Promise<number> {
  const chars = Array.from(text)
  if (!chars.length) return 0
  await nextTick()
  const node = measureElement.value?.firstChild
  if (!(node instanceof Text)) return chars.length

  const offsets = new Array<number>(chars.length + 1)
  const glyphIndices: number[] = []
  offsets[0] = 0
  for (let index = 0; index < chars.length; index += 1) {
    offsets[index + 1] = offsets[index] + chars[index].length
    if (chars[index] !== '\n' && chars[index] !== '\r') glyphIndices.push(index)
  }

  const lines = style.value.lines
  const lineHeight = Number.parseFloat(getComputedStyle(measureElement.value!).lineHeight) || style.value.font_size * 1.9
  let leadingBreakRows = 0
  let firstGlyph = 0
  while (firstGlyph < chars.length && (chars[firstGlyph] === '\n' || chars[firstGlyph] === '\r')) {
    if (chars[firstGlyph] === '\r' && chars[firstGlyph + 1] === '\n') firstGlyph += 2
    else firstGlyph += 1
    leadingBreakRows += 1
  }
  if (!glyphIndices.length) {
    let cursor = 0
    let rows = 0
    while (cursor < chars.length && rows < lines) {
      if (chars[cursor] === '\r' && chars[cursor + 1] === '\n') cursor += 2
      else cursor += 1
      rows += 1
    }
    return cursor
  }

  const firstIndex = glyphIndices[0]
  const firstRect = charRange(node, offsets[firstIndex], chars[firstIndex].length)
  if (leadingBreakRows >= lines) {
    let cursor = 0
    for (let row = 0; row < lines && cursor < chars.length; row += 1) {
      if (chars[cursor] === '\r' && chars[cursor + 1] === '\n') cursor += 2
      else cursor += 1
    }
    return cursor
  }

  const rowOfGlyph = (index: number) => {
    const rect = charRange(node, offsets[index], chars[index].length)
    return leadingBreakRows + Math.round((rect.top - firstRect.top) / lineHeight)
  }

  let low = 0
  let high = glyphIndices.length
  while (low < high) {
    const middle = Math.floor((low + high) / 2)
    if (rowOfGlyph(glyphIndices[middle]) < lines) low = middle + 1
    else high = middle
  }

  if (low === glyphIndices.length) {
    const lastGlyph = glyphIndices[glyphIndices.length - 1]
    const lastRow = rowOfGlyph(lastGlyph)
    let trailing = lastGlyph + 1
    let trailingRows = 0
    while (trailing < chars.length && (chars[trailing] === '\n' || chars[trailing] === '\r')) {
      if (chars[trailing] === '\r' && chars[trailing + 1] === '\n') trailing += 2
      else trailing += 1
      trailingRows += 1
    }
    const allowedBreaks = Math.max(0, lines - 1 - lastRow)
    if (trailingRows > allowedBreaks) {
      let cursor = lastGlyph + 1
      for (let row = 0; row < allowedBreaks; row += 1) {
        if (chars[cursor] === '\r' && chars[cursor + 1] === '\n') cursor += 2
        else cursor += 1
      }
      return cursor
    }
    return chars.length
  }

  const overflowIndex = glyphIndices[low]
  let breakRunStart = overflowIndex
  while (breakRunStart > 0 && (chars[breakRunStart - 1] === '\n' || chars[breakRunStart - 1] === '\r')) {
    breakRunStart -= 1
  }
  if (breakRunStart < overflowIndex) {
    // Keep the first explicit line break with the line it terminates. Later
    // breaks remain for the following page, so blank lines are not swallowed.
    return breakRunStart + (chars[breakRunStart] === '\r' && chars[breakRunStart + 1] === '\n' ? 2 : 1)
  }
  return overflowIndex
}

function initialReadLimit(): number {
  const estimated = Math.ceil((style.value.width / style.value.font_size) * style.value.lines * 4 + 256)
  return Math.min(MAX_READ_CHARS, Math.max(512, estimated))
}

async function render() {
  const current = book.value
  if (!current) return
  const seq = ++readSeq
  loading.value = true
  error.value = ''
  try {
    let limit = initialReadLimit()
    let result = await thiefRead(current.id, offset.value, limit)
    if (seq !== readSeq) return
    let visible = 0
    for (;;) {
      measureText.value = result.text
      visible = await visibleCharCount(result.text)
      if (seq !== readSeq) return
      const chunkLength = Array.from(result.text).length
      if (visible < chunkLength || result.offset + chunkLength >= result.total || limit >= MAX_READ_CHARS) break
      limit = Math.min(MAX_READ_CHARS, limit * 2)
      result = await thiefRead(current.id, offset.value, limit)
      if (seq !== readSeq) return
    }
    const chars = Array.from(result.text)
    visible = Math.max(visible, result.total > result.offset ? 1 : 0)
    offset.value = result.offset
    total.value = result.total
    nextOffset.value = Math.min(result.total, result.offset + visible)
    pageText.value = chars.slice(0, visible).join('')
  } catch (cause) {
    if (seq !== readSeq) return
    error.value = String(cause)
    pageText.value = ''
    nextOffset.value = offset.value
  } finally {
    if (seq === readSeq) {
      measureText.value = ''
      loading.value = false
    }
  }
}

function switchTo(target: ThiefBook) {
  window.clearTimeout(saveTimer)
  saveDirty = false
  pageHistory.splice(0, pageHistory.length, ...(target.page_history ?? []).slice(-1000))
  book.value = target
  pageText.value = ''
  measureText.value = ''
  offset.value = target.offset
  total.value = target.total_chars
  nextOffset.value = target.offset
  error.value = ''
  loading.value = true
  void render()
}

function pageBy(direction: number) {
  if (!book.value) return
  let target = offset.value
  if (direction > 0) {
    target = nextOffset.value
    if (target >= total.value) return
    pageHistory.push(offset.value)
    if (pageHistory.length > 1000) pageHistory.shift()
  } else {
    const previous = pageHistory.pop()
    if (previous === undefined) return
    target = previous
  }
  if (target === offset.value) return
  offset.value = target
  scheduleSave()
  void render()
}

function scheduleSave() {
  saveDirty = true
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => void flushSave(), SAVE_DEBOUNCE)
}

function onWindowMoved(position: { x: number; y: number }) {
  pendingPosition = { x: position.x, y: position.y }
  window.clearTimeout(positionSaveTimer)
  positionSaveTimer = window.setTimeout(() => void flushPositionSave(), 300)
}

async function flushPositionSave(position?: { x: number; y: number }) {
  window.clearTimeout(positionSaveTimer)
  let target = position ?? pendingPosition
  pendingPosition = null
  try {
    if (!target) {
      const current = await getCurrentWindow().outerPosition()
      target = { x: current.x, y: current.y }
    }
    await thiefSaveReaderPosition(target.x, target.y)
  } catch {
    // Position persistence is best-effort while the reader closes.
  }
}

async function installWindowListeners() {
  const currentWindow = getCurrentWindow()
  try {
    unlisteners.push(await currentWindow.onMoved((event) => onWindowMoved(event.payload)))
    unlisteners.push(await currentWindow.onCloseRequested((event) => {
      event.preventDefault()
      void (async () => {
        try {
          const position = await currentWindow.outerPosition()
          await flushSave()
          await thiefRememberReaderHidden(position.x, position.y)
        } catch {
          // The native plugin host already hides windows on close requests.
        }
      })()
    }))
  } catch {
    // Native move events are optional; toolbar hide also saves the position.
  }
}

async function flushSave() {
  window.clearTimeout(saveTimer)
  if (!saveDirty || !book.value) return
  saveDirty = false
  const current = book.value
  const at = offset.value
  const history = [...pageHistory]
  current.offset = at
  current.page_history = history
  try {
    await thiefSaveProgress(current.id, at, history)
    void thiefNotifyProgress(current.id, at).catch(() => {})
  } catch {
    saveDirty = true
  }
}

function onWheel(event: WheelEvent) {
  const now = Date.now()
  if (now - lastWheelAt < WHEEL_MS) return
  lastWheelAt = now
  if (event.deltaY > 0) pageBy(1)
  else if (event.deltaY < 0) pageBy(-1)
}

function onPointerDown(event: PointerEvent) {
  if (style.value.pinned || event.button !== 0) return
  void getCurrentWindow().startDragging().catch(() => {})
}

function onStyleEvent(payload: ThiefStyle) {
  const layoutChanged = style.value.font_size !== payload.font_size
    || style.value.width !== payload.width
    || style.value.lines !== payload.lines
  style.value = payload
  if (layoutChanged) {
    window.clearTimeout(styleRenderTimer)
    styleRenderTimer = window.setTimeout(() => void render(), 100)
  }
}

async function onOpenEvent(bookId: string) {
  try {
    const cfg = await thiefGetConfig()
    const target = cfg.books.find((item) => item.id === bookId)
    if (target) {
      await flushSave()
      switchTo(target)
    }
  } catch {
    // Keep the current page if config refresh fails.
  }
}

async function onJumpEvent(payload: { book_id: string; offset: number }) {
  try {
    // Flush a pending page save first, then write the requested target again so
    // a delayed save from the old page cannot undo the jump.
    await flushSave()
    const cfg = await thiefGetConfig()
    const found = cfg.books.find((item) => item.id === payload.book_id)
    if (!found) return
    const target = { ...found, offset: payload.offset, page_history: [] }
    await thiefSaveProgress(target.id, target.offset, [])
    switchTo(target)
  } catch {
    // Keep the current page if the target could not be loaded.
  }
}

function onPageEvent(direction: number) {
  pageBy(direction)
}

function setupAutoPage() {
  window.clearInterval(autoTimer)
  const seconds = style.value.auto_page_secs
  if (!seconds) return
  autoTimer = window.setInterval(() => {
    if (!atEnd.value) pageBy(1)
  }, seconds * 1000)
}

async function keepWindowOnScreen() {
  try {
    const currentWindow = getCurrentWindow()
    const [position, size, monitors] = await Promise.all([
      currentWindow.outerPosition(),
      currentWindow.outerSize(),
      availableMonitors()
    ])
    const visible = monitors.some((monitor) => {
      const left = monitor.position.x
      const top = monitor.position.y
      const right = left + monitor.size.width
      const bottom = top + monitor.size.height
      return position.x + size.width > left + 48
        && position.x < right - 48
        && position.y + size.height > top + 32
        && position.y < bottom - 32
    })
    if (visible) return
    const monitor = await primaryMonitor() ?? monitors[0]
    if (!monitor) return
    const x = monitor.position.x + Math.max(0, monitor.size.width - size.width - 24)
    const y = monitor.position.y + 24
    await currentWindow.setPosition(new PhysicalPosition(x, y))
  } catch {
    // Monitor information can be unavailable in restricted desktop environments.
  }
}

watch(() => style.value.auto_page_secs, setupAutoPage)

async function installListeners() {
  try {
    const listeners = await Promise.all([
      listen<ThiefStyle>(THIEF_EVENTS.styleChanged, (event) => onStyleEvent(event.payload)),
      listen<string>(THIEF_EVENTS.openBook, (event) => void onOpenEvent(event.payload)),
      listen<{ book_id: string; offset: number }>(THIEF_EVENTS.jump, (event) => void onJumpEvent(event.payload)),
      listen<number>(THIEF_EVENTS.page, (event) => onPageEvent(event.payload))
    ])
    unlisteners.push(...listeners)
  } catch {
    // The reader still works without optional cross-window events.
  }
}

async function loadInitialState() {
  try {
    const cfg = await thiefGetConfig()
    style.value = cfg.style
    setupAutoPage()
    const current = pickCurrent(cfg)
    if (current) switchTo(current)
    else loading.value = false
    // These native window calls must not delay rendering the reader content.
    void keepWindowOnScreen()
  } catch (cause) {
    error.value = String(cause)
    loading.value = false
  }
}

onMounted(() => {
  void installWindowListeners()
  void installListeners()
  void loadInitialState()
  window.addEventListener('blur', onWindowBlur)
  window.addEventListener('beforeunload', onWindowBlur)
})

function onWindowBlur() {
  void flushSave()
  void flushPositionSave()
}

onUnmounted(() => {
  window.clearInterval(autoTimer)
  window.clearTimeout(saveTimer)
  window.clearTimeout(styleRenderTimer)
  window.clearTimeout(positionSaveTimer)
  window.removeEventListener('blur', onWindowBlur)
  window.removeEventListener('beforeunload', onWindowBlur)
  void flushSave()
  unlisteners.forEach((unlisten) => unlisten())
})
</script>

<template>
  <div
    class="reader-root"
    :data-tauri-drag-region="style.pinned ? undefined : ''"
    :style="rootStyle"
    @contextmenu.prevent
    @wheel="onWheel"
    @pointerdown="onPointerDown"
  >
    <template v-if="book">
      <div class="page" :data-tauri-drag-region="style.pinned ? undefined : ''" :style="{ fontSize: style.font_size + 'px' }">
        <div v-if="loading" class="loading">正在加载…</div>
        <div v-else-if="error" class="err">{{ error }}</div>
        <div v-else class="text" :data-tauri-drag-region="style.pinned ? undefined : ''">{{ pageText }}</div>
      </div>
      <div
        ref="measureElement"
        class="measure text"
        aria-hidden="true"
        :style="{ fontSize: style.font_size + 'px' }"
      >{{ measureText }}</div>
    </template>
    <div v-else class="empty" :data-tauri-drag-region="style.pinned ? undefined : ''">
      {{ loading ? '正在打开书架…' : (error || '书架为空，请先导入 TXT 文件') }}
    </div>
  </div>
</template>

<style scoped>
.reader-root {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  border-radius: var(--radius, 12px);
  overflow: hidden;
  color: #e8e8ea;
  user-select: none;
  cursor: default;
}

.page {
  flex: 1;
  min-height: 0;
  padding: 8px 14px 4px;
  line-height: 1.9;
  overflow: hidden;
  font-family: 'Segoe UI', 'Microsoft YaHei UI', system-ui, sans-serif;
}

.text {
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
  line-height: 1.9;
}

.measure {
  position: absolute;
  top: 8px;
  left: 14px;
  right: 14px;
  visibility: hidden;
  pointer-events: none;
  font-family: 'Segoe UI', 'Microsoft YaHei UI', system-ui, sans-serif;
}

.loading,
.err {
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 12px;
}

.err {
  color: #ffb4ab;
}

.empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 20px;
  text-align: center;
  font-size: 12.5px;
  opacity: 0.5;
}
</style>
