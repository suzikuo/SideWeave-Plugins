<script setup lang="ts">
/** 桌面便签窗：文本便签（正文自动保存）与任务便签（清单勾选、划线保留、回车添加、双击改字）。 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Directive } from 'vue'
import { getCurrentWindow, listen, type UnlistenFn } from '../../../ui-sdk/runtime'
import { NOTE_SKINS, noteAddItem, noteEditItem, noteGet, noteRemoveItem, noteSetItemDone, noteSetSkin, noteSetStyle, noteUpdateText } from './api'
import type { TodoItem } from './api'

/** 便签底色（与详情页色板一致，循环切换） */
const COLORS = ['#f9e2af', '#a6e3a1', '#89b4fa', '#f5c2e7', '#f38ba8', '#94e2d5']

const id = getCurrentWindow().label.replace(/^note-/, '')
const kind = ref<'text' | 'todo'>('text')
const skin = ref('classic')
const text = ref('')
const items = ref<TodoItem[]>([])
const color = ref(COLORS[0])
const opacity = ref(0.95)
const loaded = ref(false)
const draft = ref('')
const editingId = ref('')
const editingText = ref('')
let saveTimer = 0
const unlisteners: UnlistenFn[] = []

const rootStyle = computed(() => {
  const c = color.value
  const o = Math.round(opacity.value * 100)
  switch (skin.value) {
    case 'lined':
    case 'grid':
      // 纸张感：便签色淡混米白纸底，横线/方格纹理由 CSS 背景图绘制
      return {
        backgroundColor: `color-mix(in srgb, color-mix(in srgb, ${c} 30%, #fbf8ef) ${o}%, transparent)`
      }
    case 'minimal':
      return {
        backgroundColor: `color-mix(in srgb, ${c} 16%, transparent)`,
        borderLeft: `3px solid color-mix(in srgb, ${c} 75%, transparent)`
      }
    case 'terminal':
      return { backgroundColor: `rgba(25, 26, 36, ${opacity.value})` }
    case 'neon':
      return {
        backgroundColor: `rgba(16, 17, 24, ${opacity.value})`,
        border: `1.5px solid color-mix(in srgb, ${c} 70%, transparent)`,
        boxShadow: `0 0 16px color-mix(in srgb, ${c} 40%, transparent), 0 10px 28px rgba(0, 0, 0, 0.35)`
      }
    default:
      return { backgroundColor: `color-mix(in srgb, ${c} ${o}%, transparent)` }
  }
})
const skinClass = computed(() => `skin-${skin.value}`)
const skinLabel = computed(() => NOTE_SKINS.find((s) => s.id === skin.value)?.label ?? '经典')

/** 头部按钮循环切换皮肤 */
async function cycleSkin() {
  const idx = NOTE_SKINS.findIndex((s) => s.id === skin.value)
  const next = NOTE_SKINS[(idx + 1) % NOTE_SKINS.length]
  skin.value = next.id
  try {
    await noteSetSkin(id, next.id)
  } catch {
    // 保存失败保留本地预览
  }
}

/** 任务排序：未完成在前（保持原顺序），已完成的沉底（后完成的更靠下） */
const viewItems = computed(() => {
  const active = items.value.filter((i) => !i.done)
  const done = items.value
    .filter((i) => i.done)
    .sort((a, b) => (a.done_at ?? 0) - (b.done_at ?? 0))
  return [...active, ...done]
})
const doneCount = computed(() => items.value.filter((i) => i.done).length)

async function load() {
  try {
    const note = await noteGet(id)
    kind.value = note.kind === 'todo' ? 'todo' : 'text'
    skin.value = NOTE_SKINS.some((s) => s.id === note.skin) ? note.skin : 'classic'
    text.value = note.text
    items.value = note.items ?? []
    color.value = note.color
    opacity.value = note.opacity
    loaded.value = true
  } catch {
    // 窗口与数据短暂不同步时静默
  }
}

async function refreshItems() {
  try {
    const note = await noteGet(id)
    items.value = note.items ?? []
  } catch {
    // 静默
  }
}

// 文本便签：正文防抖保存
watch(text, () => {
  if (!loaded.value || kind.value !== 'text') return
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => {
    void noteUpdateText(id, text.value).catch(() => {})
  }, 400)
})

// 详情页改色/透明度 → 热更新
onMounted(async () => {
  await load()
  try {
    // 事件载荷带便签 id：Tauri 的 Any 监听器会收到所有 emit_to 事件，必须按 id 过滤，
    // 否则改某张便签的皮肤/颜色会刷到所有便签窗
    unlisteners.push(
      await listen<{ id?: string; color: string; opacity: number }>('note://style', (e) => {
        if (e.payload.id && e.payload.id !== id) return
        color.value = e.payload.color
        opacity.value = e.payload.opacity
      })
    )
    unlisteners.push(
      await listen<{ id?: string; skin: string }>('note://skin', (e) => {
        if (e.payload.id && e.payload.id !== id) return
        if (NOTE_SKINS.some((s) => s.id === e.payload.skin)) skin.value = e.payload.skin
      })
    )
  } catch {
    // 事件权限缺失时静默
  }
})

onUnmounted(() => {
  window.clearTimeout(saveTimer)
  unlisteners.forEach((fn) => fn())
})

function cycleColor() {
  const idx = COLORS.indexOf(color.value)
  const next = COLORS[(idx + 1) % COLORS.length]
  color.value = next
  void noteSetStyle(id, next, opacity.value).catch(() => {})
}

/** 隐藏（关闭按钮 → CloseRequested → 转隐藏，详情页可再打开） */
function hideNote() {
  void getCurrentWindow().close()
}

async function addDraft() {
  const t = draft.value.trim()
  if (!t) return
  draft.value = ''
  try {
    await noteAddItem(id, t)
    await refreshItems()
  } catch {
    // 静默
  }
}

async function toggleItem(item: TodoItem) {
  try {
    await noteSetItemDone(id, item.id, !item.done)
    await refreshItems()
  } catch {
    // 静默
  }
}

async function removeItem(item: TodoItem) {
  try {
    await noteRemoveItem(id, item.id)
    await refreshItems()
  } catch {
    // 静默
  }
}

function startEdit(item: TodoItem) {
  editingId.value = item.id
  editingText.value = item.text
}

async function commitEdit() {
  const itemId = editingId.value
  const t = editingText.value.trim()
  editingId.value = ''
  if (!itemId || !t) return
  try {
    await noteEditItem(id, itemId, t)
    await refreshItems()
  } catch {
    // 静默
  }
}

/** 编辑框 / 新增框自动聚焦 */
const vFocus: Directive<HTMLInputElement> = {
  mounted: (el) => el.focus()
}
</script>

<template>
  <div class="note-root" :class="skinClass" :style="rootStyle">
    <div class="note-head" data-tauri-drag-region>
      <button class="dot" :style="{ background: '#3a3325' }" title="换颜色" @click="cycleColor" />
      <button class="head-btn skin-btn" :title="`皮肤：${skinLabel}（点击切换）`" @click="cycleSkin">✦</button>
      <span v-if="kind === 'todo' && items.length" class="count">{{ doneCount }}/{{ items.length }}</span>
      <span class="grow" data-tauri-drag-region />
      <button class="head-btn" title="隐藏（可在书架详情页重新打开）" @click="hideNote">✕</button>
    </div>

    <template v-if="kind === 'todo'">
      <div class="todo-list">
        <div v-for="it in viewItems" :key="it.id" class="todo-row" :class="{ done: it.done }">
          <button
            class="check"
            :class="{ on: it.done }"
            :title="it.done ? '标记为未完成' : '完成'"
            @click="toggleItem(it)"
          >
            ✓
          </button>
          <input
            v-if="editingId === it.id"
            v-model="editingText"
            v-focus
            class="todo-edit"
            spellcheck="false"
            @keyup.enter="commitEdit"
            @blur="commitEdit"
          />
          <span v-else class="todo-text" title="双击编辑" @dblclick="startEdit(it)">{{ it.text }}</span>
          <button class="row-del" title="删除任务" @click="removeItem(it)">✕</button>
        </div>
        <div v-if="items.length === 0" class="todo-empty">还没有任务 · 下方输入框添加</div>
      </div>
      <input
        v-model="draft"
        class="todo-add"
        placeholder="+ 输入任务，回车添加"
        spellcheck="false"
        @keyup.enter="addDraft"
      />
    </template>
    <textarea v-else v-model="text" class="note-text" placeholder="写点什么…" spellcheck="false" />
  </div>
</template>

<style scoped>
.note-root {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  border-radius: 10px;
  overflow: hidden;
  color: #3a3325;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28);
}
.note-head {
  flex: none;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 6px 0;
}
.dot {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 1.5px solid rgba(58, 51, 37, 0.55);
  cursor: pointer;
  padding: 0;
}
.count {
  font-size: 11px;
  color: rgba(58, 51, 37, 0.65);
  font-variant-numeric: tabular-nums;
}
.grow {
  flex: 1;
}
.skin-btn {
  font-size: 12px;
  line-height: 1;
  padding: 1px 5px;
}
.head-btn {
  border: none;
  background: transparent;
  color: rgba(58, 51, 37, 0.7);
  font-size: 11px;
  cursor: pointer;
  padding: 0 4px;
  border-radius: 5px;
}
.head-btn:hover {
  background: rgba(58, 51, 37, 0.12);
  color: #3a3325;
}
.note-text {
  flex: 1;
  min-height: 0;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  padding: 6px 12px 10px;
  font-family: 'Segoe UI', 'Microsoft YaHei UI', system-ui, sans-serif;
  font-size: 13px;
  line-height: 1.7;
  color: inherit;
}

/* ---------- 任务清单 ---------- */
.todo-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 8px;
}
.todo-row {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 3px 2px;
  border-radius: 6px;
}
.todo-row:hover {
  background: rgba(58, 51, 37, 0.07);
}
.check {
  flex: none;
  width: 16px;
  height: 16px;
  margin-top: 1px;
  border-radius: 50%;
  border: 1.5px solid rgba(58, 51, 37, 0.55);
  background: transparent;
  color: transparent;
  font-size: 10px;
  line-height: 1;
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.check.on {
  background: #3a3325;
  border-color: #3a3325;
  color: #fffdf5;
}
.todo-text {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  line-height: 1.55;
  word-break: break-word;
  cursor: text;
}
.todo-row.done .todo-text {
  text-decoration: line-through;
  opacity: 0.55;
}
.todo-edit {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: rgba(255, 255, 255, 0.45);
  border-radius: 4px;
  font: inherit;
  font-size: 12.5px;
  color: inherit;
  padding: 0 4px;
}
.row-del {
  flex: none;
  border: none;
  background: transparent;
  color: rgba(58, 51, 37, 0.45);
  font-size: 10px;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  visibility: hidden;
}
.todo-row:hover .row-del {
  visibility: visible;
}
.row-del:hover {
  background: rgba(58, 51, 37, 0.15);
  color: #3a3325;
}
.todo-empty {
  padding: 14px 4px;
  text-align: center;
  font-size: 11.5px;
  color: rgba(58, 51, 37, 0.5);
}
.todo-add {
  flex: none;
  border: none;
  outline: none;
  background: rgba(58, 51, 37, 0.08);
  margin: 0 8px 8px;
  border-radius: 7px;
  padding: 6px 10px;
  font-size: 12px;
  font-family: inherit;
  color: inherit;
}
.todo-add::placeholder {
  color: rgba(58, 51, 37, 0.5);
}

/* ---------- 皮肤 ---------- */
/* 横线纸：衬线字 + 26px 行距对齐横线（仅文本便签正文画线） */
.skin-lined .note-text {
  line-height: 26px;
  padding-top: 7px;
  background-image: repeating-linear-gradient(
    to bottom,
    transparent 0 25px,
    rgba(120, 100, 60, 0.2) 25px 26px
  );
  background-position: 0 7px;
  font-family: Georgia, 'Songti SC', 'SimSun', serif;
}

/* 方格纸：22px 网格 */
.skin-grid .note-text {
  background-image: linear-gradient(rgba(100, 100, 120, 0.13) 1px, transparent 1px),
    linear-gradient(90deg, rgba(100, 100, 120, 0.13) 1px, transparent 1px);
  background-size: 22px 22px;
}

/* 极简：去阴影、小圆角，只剩左侧色条 */
.skin-minimal {
  box-shadow: none;
  border-radius: 6px;
}

/* 终端：暗底等宽字 */
.skin-terminal {
  color: #cdd6f4;
  border-radius: 8px;
}
.skin-terminal .note-text,
.skin-terminal .todo-text,
.skin-terminal .todo-add,
.skin-terminal .todo-edit {
  font-family: 'Cascadia Code', Consolas, 'Courier New', monospace;
}
.skin-terminal .dot {
  border-color: rgba(205, 214, 244, 0.6);
}
.skin-terminal .head-btn,
.skin-terminal .count {
  color: rgba(205, 214, 244, 0.6);
}
.skin-terminal .head-btn:hover {
  background: rgba(205, 214, 244, 0.14);
  color: #cdd6f4;
}
.skin-terminal .check {
  border-color: rgba(205, 214, 244, 0.5);
}
.skin-terminal .check.on {
  background: #89b4fa;
  border-color: #89b4fa;
  color: #11111b;
}
.skin-terminal .todo-row:hover {
  background: rgba(205, 214, 244, 0.08);
}
.skin-terminal .row-del {
  color: rgba(205, 214, 244, 0.4);
}
.skin-terminal .row-del:hover {
  background: rgba(205, 214, 244, 0.15);
  color: #f38ba8;
}
.skin-terminal .todo-add,
.skin-terminal .todo-edit {
  background: rgba(205, 214, 244, 0.08);
  color: #cdd6f4;
}
.skin-terminal .todo-add::placeholder {
  color: rgba(205, 214, 244, 0.42);
}
.skin-terminal .todo-empty {
  color: rgba(205, 214, 244, 0.45);
}

/* 霓虹：暗底 + 便签色发光描边（描边/辉光在 rootStyle 内联） */
.skin-neon {
  color: #e6e8f2;
}
.skin-neon .dot {
  border-color: rgba(230, 232, 242, 0.6);
}
.skin-neon .head-btn,
.skin-neon .count {
  color: rgba(230, 232, 242, 0.62);
}
.skin-neon .head-btn:hover {
  background: rgba(230, 232, 242, 0.14);
  color: #fff;
}
.skin-neon .check {
  border-color: rgba(230, 232, 242, 0.55);
}
.skin-neon .check.on {
  background: #f5c2e7;
  border-color: #f5c2e7;
  color: #1a1023;
}
.skin-neon .todo-row:hover {
  background: rgba(230, 232, 242, 0.08);
}
.skin-neon .row-del {
  color: rgba(230, 232, 242, 0.4);
}
.skin-neon .row-del:hover {
  background: rgba(230, 232, 242, 0.15);
  color: #f38ba8;
}
.skin-neon .todo-add,
.skin-neon .todo-edit {
  background: rgba(230, 232, 242, 0.08);
  color: #e6e8f2;
}
.skin-neon .todo-add::placeholder {
  color: rgba(230, 232, 242, 0.42);
}
.skin-neon .todo-empty {
  color: rgba(230, 232, 242, 0.45);
}
</style>
