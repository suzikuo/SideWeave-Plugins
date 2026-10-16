<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** Thiefbook 书架器详情页：书架管理（导入 / 改名 / 打开悬浮窗 / 删除）
 *  + 悬浮窗设置（字号/颜色/透明/宽高/每页字数/固定/全局热键，即改即存热更新） */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen, type UnlistenFn } from '../../../ui-sdk/runtime'
import Icon from '../../../ui-sdk/Icon.vue'
import HotkeyRecorder from './HotkeyRecorder.vue'
import RangeSlider from '../../../ui-sdk/RangeSlider.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import {
  CURRENT_BOOK_KEY,
  STYLE_BOUNDS,
  THIEF_EVENTS,
  THIEF_STYLE_DEFAULT,
  formatProgress,
  thiefGetConfig,
  thiefFind,
  thiefJumpTo,
  thiefPickBook,
  thiefRemoveBook,
  thiefRenameBook,
  thiefSaveStyle,
  thiefSetPinned,
  thiefToggleReader,
  thiefOpenBook
} from './api'
import type { ThiefBook, ThiefStyle } from './api'

type StyleKey = keyof typeof STYLE_BOUNDS
type HotkeyKey = 'hotkey_toggle' | 'hotkey_prev' | 'hotkey_next'

// ---------- 状态 ----------

const books = ref<ThiefBook[]>([])
const style = ref<ThiefStyle>({ ...THIEF_STYLE_DEFAULT })
const currentId = ref<string | null>(null)
const confirmingId = ref<string | null>(null)
const importing = ref(false)
const findQuery = ref('')
const findStatus = ref('')
const findBusy = ref(false)
const lastFoundOffset = ref<number | null>(null)

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

let confirmTimer = 0
let unlistenProgress: UnlistenFn | null = null

/** 当前书排首位（列表第一本标"当前"徽标） */
const orderedBooks = computed(() => {
  const list = [...books.value]
  const idx = list.findIndex((b) => b.id === currentId.value)
  if (idx > 0) list.unshift(...list.splice(idx, 1))
  return list
})

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

async function refresh() {
  try {
    const cfg = await thiefGetConfig()
    books.value = cfg.books
    style.value = cfg.style
    const saved = readSavedCurrent()
    currentId.value = saved && cfg.books.some((b) => b.id === saved) ? saved : (cfg.books[0]?.id ?? null)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function readSavedCurrent(): string | null {
  try {
    return localStorage.getItem(CURRENT_BOOK_KEY)
  } catch {
    return null
  }
}

// ---------- 书架 ----------

async function importBook() {
  if (importing.value) return
  importing.value = true
  try {
    const book = await thiefPickBook()
    if (book) {
      resetFind()
      const existing = books.value.findIndex((item) => item.id === book.id)
      if (existing >= 0) books.value[existing] = book
      else books.value.push(book)
      currentId.value = book.id
      try {
        localStorage.setItem(CURRENT_BOOK_KEY, book.id)
      } catch {
        // The backend has already saved the book; the in-memory list is enough for this view.
      }
      showToast(`已导入《${book.name}》`, 'success')
    }
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    importing.value = false
  }
}

/** 打开：把该书记为当前（悬浮窗显示当前书）并调起悬浮窗 */
async function openBook(book: ThiefBook) {
  resetFind()
  currentId.value = book.id
  try {
    localStorage.setItem(CURRENT_BOOK_KEY, book.id)
  } catch {
    // 存储不可用时回落：悬浮窗新建时会取 books[0]
  }
  try {
    await thiefOpenBook(book.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function resetFind() {
  lastFoundOffset.value = null
  findStatus.value = ''
}

async function findAndOpen(direction: -1 | 1) {
  const query = findQuery.value.trim()
  if (!query) return
  const book = books.value.find((item) => item.id === currentId.value) ?? books.value[0]
  if (!book || findBusy.value) return

  findBusy.value = true
  try {
    const fromOffset = lastFoundOffset.value === null
      ? book.offset
      : (direction > 0 ? lastFoundOffset.value + 1 : lastFoundOffset.value)
    const result = await thiefFind(book.id, query, fromOffset, direction)
    if (result.offset === null) {
      findStatus.value = `《${book.name}》中没有找到“${query}”`
      lastFoundOffset.value = null
      return
    }

    const offset = result.offset
    lastFoundOffset.value = offset
    book.offset = offset
    currentId.value = book.id
    findStatus.value = `《${book.name}》· 全文第 ${offset + 1} 字（共 ${result.total.toLocaleString()} 字）`
    try {
      localStorage.setItem(CURRENT_BOOK_KEY, book.id)
    } catch {
      // The plugin config is authoritative if local storage is unavailable.
    }
    await thiefJumpTo(book.id, offset)
  } catch (e) {
    findStatus.value = '查找失败'
    showToast(String(e), 'error')
  } finally {
    findBusy.value = false
  }
}

async function toggleReader() {
  try {
    await thiefToggleReader()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 删除：trash 两次点击确认 */
function askRemove(book: ThiefBook) {
  if (confirmingId.value !== book.id) {
    confirmingId.value = book.id
    window.clearTimeout(confirmTimer)
    confirmTimer = window.setTimeout(() => {
      confirmingId.value = null
    }, 2600)
    return
  }
  window.clearTimeout(confirmTimer)
  confirmingId.value = null
  void doRemove(book)
}

async function doRemove(book: ThiefBook) {
  try {
    resetFind()
    await thiefRemoveBook(book.id)
    if (currentId.value === book.id) {
      try {
        localStorage.removeItem(CURRENT_BOOK_KEY)
      } catch {
        // 忽略
      }
      currentId.value = null
    }
    await refresh()
    showToast('已删除', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 样式设置（即改即存，热更新悬浮窗） ----------

/** 正文色快捷色板 */
const COLOR_SWATCHES = ['#e8e8ea', '#a6e3a1', '#89b4fa', '#f9e2af', '#f5c2e7', '#94e2d5']

async function applyStyle(next: ThiefStyle) {
  style.value = next
  try {
    await thiefSaveStyle(next)
    showToast('已应用', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function applyNumber(key: StyleKey, value: number) {
  const next: ThiefStyle = { ...style.value }
  if (key === 'font_size') next.font_size = value
  else if (key === 'opacity') next.opacity = value
  else if (key === 'width') next.width = value
  else if (key === 'lines') next.lines = value
  else if (key === 'auto_page_secs') next.auto_page_secs = value
  void applyStyle(next)
}

function applyColor(color: string) {
  void applyStyle({ ...style.value, color })
}

function applyHotkey(key: HotkeyKey, value: string) {
  void applyStyle({ ...style.value, [key]: value })
}

function applyHideOnDesktop(value: boolean) {
  void applyStyle({ ...style.value, hide_on_desktop: value })
}

/** 固定（点击穿透）开关 */
async function togglePinned() {
  const next = !style.value.pinned
  style.value = { ...style.value, pinned: next }
  try {
    await thiefSetPinned(next)
    showToast(next ? '已固定（点击穿透，详情页可恢复）' : '已取消固定', 'success')
  } catch (e) {
    style.value = { ...style.value, pinned: !next }
    showToast(String(e), 'error')
  }
}

const opacityText = computed(() => `${Math.round(style.value.opacity * 100)}%`)

// ---------- 书架改名 ----------

const renamingId = ref<string | null>(null)
const renameDraft = ref('')

function startRename(book: ThiefBook) {
  renamingId.value = book.id
  renameDraft.value = book.name
}

function commitRename(book: ThiefBook) {
  const name = renameDraft.value.trim()
  renamingId.value = null
  if (!name || name === book.name) return
  void (async () => {
    try {
      const updated = await thiefRenameBook(book.id, name)
      await refresh()
      showToast(`展示名已改为「${updated.name}」`, 'success')
    } catch (e) {
      showToast(String(e), 'error')
    }
  })()
}

// ---------- 生命周期 ----------

onMounted(async () => {
  await refresh()
  // 悬浮窗翻页后同步进度徽标
  try {
    unlistenProgress = await listen<{ book_id: string; offset: number }>(
      THIEF_EVENTS.progressChanged,
      (e) => {
        const b = books.value.find((x) => x.id === e.payload.book_id)
        if (b) b.offset = e.payload.offset
      }
    )
  } catch {
    // 事件权限缺失时静默降级
  }
})

onUnmounted(() => {
  window.clearTimeout(confirmTimer)
  unlistenProgress?.()
})
</script>

<template>
  <div class="thief-view">
    <!-- 工具栏 -->
    <div class="toolbar">
      <Button variant="default" size="sm" class="btn primary" :disabled="importing" @click="importBook">
        <Icon name="plus" :size="14" /> 导入 txt
      </Button>
      <span class="spacer" />
      <Button variant="outline" size="sm" class="btn" title="显示 / 隐藏悬浮阅读窗" @click="toggleReader">
        <Icon name="book-open" :size="14" /> 悬浮窗
      </Button>
      <Button variant="outline" size="sm"
        class="btn"
        :class="{ 'btn-on': style.pinned }"
        :title="style.pinned ? '取消固定（恢复鼠标交互与老板键）' : '固定（点击穿透：翻页点击/拖拽/老板键全部失效，仅显示文字）'"
        @click="togglePinned"
      >
        <Icon name="eye-off" :size="14" /> {{ style.pinned ? '取消固定' : '固定' }}
      </Button>
    </div>

    <section class="find-panel">
      <div class="find-row">
        <select v-model="currentId" class="find-book" aria-label="查找范围" :disabled="books.length === 0" @change="resetFind">
          <option v-for="b in books" :key="b.id" :value="b.id">{{ b.name }}</option>
        </select>
        <input
          v-model="findQuery"
          class="find-input"
          type="search"
          maxlength="256"
          autocomplete="off"
          placeholder="输入关键词定位到正文"
          aria-label="关键词"
          @input="resetFind"
          @keydown.enter.prevent="findAndOpen($event.shiftKey ? -1 : 1)"
        />
        <Button variant="outline" size="sm" class="btn small" :disabled="findBusy || !findQuery.trim() || books.length === 0" @click="findAndOpen(-1)">上一个</Button>
        <Button variant="default" size="sm" class="btn small primary" :disabled="findBusy || !findQuery.trim() || books.length === 0" @click="findAndOpen(1)">下一个</Button>
      </div>
      <div class="find-status" aria-live="polite">{{ findStatus || '在所选书籍中查找；Enter 下一个，Shift+Enter 上一个' }}</div>
    </section>

    <!-- 书架 -->
    <div class="shelf">
      <div
        v-for="b in orderedBooks"
        :key="b.id"
        class="book"
        :class="{ current: b.id === currentId }"
      >
        <div class="meta">
          <div class="name-row">
            <template v-if="renamingId === b.id">
              <input
                v-model="renameDraft"
                class="rename-input"
                placeholder="展示名（如：需求文档.txt）"
                maxlength="60"
                @keyup.enter="commitRename(b)"
                @keyup.esc="renamingId = null"
                @blur="commitRename(b)"
              />
            </template>
            <template v-else>
              <span class="name" :title="b.path">{{ b.name }}</span>
              <span v-if="b.id === currentId" class="badge">当前</span>
              <Button variant="outline" size="sm" class="icon-btn" title="修改展示名（防路人围观）" @click="startRename(b)">
                <Icon name="pencil" :size="13" />
              </Button>
            </template>
          </div>
          <span class="pct">{{ formatProgress(b.offset, b.total_chars) }}</span>
        </div>
        <div class="actions">
          <Button variant="outline" size="sm" class="btn small" title="在悬浮窗中打开本书" @click="openBook(b)">打开</Button>
          <Button variant="destructive" size="sm"
            class="icon-btn danger"
            :class="{ armed: confirmingId === b.id }"
            :title="confirmingId === b.id ? '再点一次确认删除' : '删除'"
            @click="askRemove(b)"
          >
            <Icon name="trash" :size="14" />
          </Button>
        </div>
      </div>
      <div v-if="books.length === 0" class="empty">书架空空，点击"导入 txt"添加第一本书</div>
    </div>

    <!-- 样式设置 -->
    <section class="style-panel">
      <h3>样式设置 <span class="hint">即改即存，悬浮窗实时热更新</span></h3>
      <div class="style-row">
        <span class="label">字号</span>
        <RangeSlider
          :model-value="style.font_size"
          :min="STYLE_BOUNDS.font_size.min"
          :max="STYLE_BOUNDS.font_size.max"
          :step="STYLE_BOUNDS.font_size.step"
          aria-label="字号"
          @commit="(v) => applyNumber('font_size', v)"
        />
        <span class="val">{{ style.font_size }}px</span>
      </div>
      <div class="style-row">
        <span class="label">文字颜色</span>
        <div class="color-row">
          <button
            v-for="c in COLOR_SWATCHES"
            :key="c"
            type="button"
            class="swatch"
            :class="{ on: style.color === c }"
            :style="{ background: c }"
            :title="c"
            @click="applyColor(c)"
          />
          <input
            type="color"
            class="swatch custom"
            :value="style.color"
            title="自定义颜色"
            @input="applyColor(($event.target as HTMLInputElement).value)"
          />
        </div>
      </div>
      <div class="style-row">
        <span class="label">不透明度</span>
        <RangeSlider
          :model-value="style.opacity"
          :min="STYLE_BOUNDS.opacity.min"
          :max="STYLE_BOUNDS.opacity.max"
          :step="STYLE_BOUNDS.opacity.step"
          :format="(v) => `${Math.round(v * 100)}%`"
          aria-label="窗口不透明度（0 = 背景完全透明）"
          @commit="(v) => applyNumber('opacity', v)"
        />
        <span class="val">{{ opacityText }}</span>
      </div>
      <div class="style-row">
        <span class="label">窗口宽度</span>
        <RangeSlider
          :model-value="style.width"
          :min="STYLE_BOUNDS.width.min"
          :max="STYLE_BOUNDS.width.max"
          :step="STYLE_BOUNDS.width.step"
          :format="(v) => `${v}px`"
          aria-label="窗口宽度"
          @commit="(v) => applyNumber('width', v)"
        />
        <span class="val">{{ style.width }}px</span>
      </div>
      <div class="style-row">
        <span class="label">显示行数</span>
        <RangeSlider
          :model-value="style.lines"
          :min="STYLE_BOUNDS.lines.min"
          :max="STYLE_BOUNDS.lines.max"
          :step="STYLE_BOUNDS.lines.step"
          :format="(v) => `${v} 行`"
          aria-label="显示行数"
          @commit="(v) => applyNumber('lines', v)"
        />
        <span class="val">{{ style.lines }} 行</span>
      </div>
    </section>

    <!-- 自动翻页 -->
    <section class="style-panel">
      <h3>自动翻页 <span class="hint">到书尾自动停止；拖动即改即存</span></h3>
      <div class="style-row">
        <span class="label">间隔</span>
        <RangeSlider
          :model-value="style.auto_page_secs"
          :min="STYLE_BOUNDS.auto_page_secs.min"
          :max="STYLE_BOUNDS.auto_page_secs.max"
          :step="STYLE_BOUNDS.auto_page_secs.step"
          :format="(v: number) => (v === 0 ? '关闭' : `${v}s`)"
          aria-label="自动翻页间隔"
          @commit="(v: number) => applyNumber('auto_page_secs', v)"
        />
        <span class="val">{{ style.auto_page_secs === 0 ? '关闭' : `${style.auto_page_secs}s` }}</span>
      </div>
    </section>

    <section class="style-panel">
      <h3>自动隐藏</h3>
      <label class="desktop-toggle">
        <input
          type="checkbox"
          :checked="style.hide_on_desktop"
          @change="applyHideOnDesktop(($event.target as HTMLInputElement).checked)"
        />
        <span>回到 Windows 桌面时隐藏悬浮阅读窗</span>
      </label>
      <p class="hint">从桌面切回应用不会自动显示；可用书架按钮或全局热键重新打开。</p>
    </section>

    <!-- 全局热键 -->
    <section class="style-panel">
      <h3>全局热键 <span class="hint">空 = 不启用；录制后点空白处生效</span></h3>
      <div class="style-row">
        <span class="label">打开 / 关闭</span>
        <div class="hotkey-row">
          <HotkeyRecorder
            :model-value="style.hotkey_toggle"
            @commit="(v) => applyHotkey('hotkey_toggle', v)"
          />
          <Button variant="outline" size="sm"
            v-if="style.hotkey_toggle"
            type="button"
            class="btn small"
            @click="applyHotkey('hotkey_toggle', '')"
          >
            清除
          </Button>
        </div>
      </div>
      <div class="style-row">
        <span class="label">上一页</span>
        <div class="hotkey-row">
          <HotkeyRecorder
            :model-value="style.hotkey_prev"
            @commit="(v) => applyHotkey('hotkey_prev', v)"
          />
          <Button variant="outline" size="sm"
            v-if="style.hotkey_prev"
            type="button"
            class="btn small"
            @click="applyHotkey('hotkey_prev', '')"
          >
            清除
          </Button>
        </div>
      </div>
      <div class="style-row">
        <span class="label">下一页</span>
        <div class="hotkey-row">
          <HotkeyRecorder
            :model-value="style.hotkey_next"
            @commit="(v) => applyHotkey('hotkey_next', v)"
          />
          <Button variant="outline" size="sm"
            v-if="style.hotkey_next"
            type="button"
            class="btn small"
            @click="applyHotkey('hotkey_next', '')"
          >
            清除
          </Button>
        </div>
      </div>
      <p class="hint">打开/关闭 即老板键；热键全局生效（无需悬浮窗聚焦）。</p>
    </section>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.thief-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}

/* ---------- 工具栏 ---------- */
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.spacer {
  flex: 1;
}

.find-panel {
  flex: none;
  padding: 9px 10px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.find-row {
  display: flex;
  align-items: center;
  gap: 7px;
}
.find-book,
.find-input {
  min-width: 0;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface-subtle);
  color: var(--text);
  font: inherit;
  font-size: 12px;
}
.find-book {
  flex: 0 1 180px;
  max-width: 30%;
}
.find-input {
  flex: 1;
}
.find-status {
  min-height: 16px;
  padding: 5px 2px 0;
  overflow: hidden;
  color: var(--text-muted);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 12.5px;
}
.btn:hover {
  border-color: var(--border-strong);
}
.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.btn.primary:hover {
  filter: brightness(1.1);
}
.btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.btn.small {
  padding: 4px 11px;
  font-size: 12px;
  border-radius: 8px;
}

/* ---------- 书架 ---------- */
.shelf {
  flex: 1;
  min-height: 120px;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}

.book {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 45%, transparent);
}
.book:last-child {
  border-bottom: none;
}
.book:hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}
.book.current {
  box-shadow: inset 3px 0 0 var(--accent);
}

.meta {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
}
.name-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.name {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  flex: none;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--accent) 60%, transparent);
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--text);
  font-size: 10.5px;
  line-height: 1.6;
}
.pct {
  flex: none;
  padding: 1px 8px;
  border-radius: 999px;
  background: var(--surface-container);
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 4px;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
}
.icon-btn:hover {
  background: var(--surface-container);
  color: var(--danger);
}
.icon-btn.armed {
  background: color-mix(in srgb, var(--danger) 20%, transparent);
  color: var(--danger);
}

.empty {
  padding: 36px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: 12.5px;
}

/* ---------- 样式设置 ---------- */
.style-panel {
  flex: none;
  padding: 12px 14px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.style-panel h3 {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin: 0 0 10px;
  font-size: 13px;
}
.hint {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-muted);
}

.style-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 0;
}
.label {
  flex: none;
  width: 60px;
  font-size: 12px;
  color: var(--text-muted);
}
.val {
  flex: none;
  width: 52px;
  text-align: right;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

/* ---------- 新控件 ---------- */
.btn.btn-on {
  border-color: var(--accent);
  color: var(--accent);
}
.rename-input {
  flex: 1;
  min-width: 0;
  padding: 3px 8px;
  border-radius: 7px;
  border: 1px solid var(--accent);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 12.5px;
  outline: none;
}
.color-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.swatch {
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: 50%;
  border: 2px solid var(--border);
  cursor: pointer;
}
.swatch.on {
  border-color: var(--text);
}
.swatch.custom {
  width: 26px;
  height: 26px;
  border: 1px dashed var(--border-strong);
  background: transparent;
}
.hotkey-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  justify-content: flex-end;
}
.desktop-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}
.desktop-toggle input {
  accent-color: var(--accent);
}
</style>
