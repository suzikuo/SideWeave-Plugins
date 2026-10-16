<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 便签详情页：新建文本/任务便签、显示/隐藏、循环颜色；删除进历史（可恢复/彻底删除） */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import { onPluginLifecycle, pluginOnEvent, type UnlistenFn } from '../../../ui-sdk/runtime'
import { noteClearHistory, noteCreate, noteDelete, noteList, NOTE_SKINS, notePurge, noteRestore, noteSetSkin, noteToggleVisible } from './api'
import type { Note, NoteKind } from './api'

const notes = ref<Note[]>([])
const confirmId = ref('')
const purgeId = ref('')
const clearArmed = ref(false)
let confirmTimer = 0
let unlistenChanges: UnlistenFn | null = null
let unlistenLifecycle: (() => void) | null = null

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

const active = computed(() => notes.value.filter((n) => !n.deleted))
const history = computed(() =>
  notes.value
    .filter((n) => n.deleted)
    .sort((a, b) => (b.deleted_at ?? 0) - (a.deleted_at ?? 0))
)

/** 便签卡片预览：任务便签显示首个未完成任务 + 进度 */
function preview(n: Note): string {
  if (n.kind === 'todo') {
    const first = n.items.find((i) => !i.done)
    return first?.text || (n.items.length ? '全部完成 ✓' : '(空任务清单)')
  }
  return n.text.split('\n')[0] || '(空白便签)'
}

function meta(n: Note): string {
  if (n.kind === 'todo') {
    const done = n.items.filter((i) => i.done).length
    return `任务 ${done}/${n.items.length} · ${n.visible ? '显示中' : '已隐藏'}`
  }
  return `${n.visible ? '显示中' : '已隐藏'} · ${n.text.length} 字`
}

async function refresh() {
  try {
    notes.value = await noteList()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function createNote(kind: NoteKind) {
  try {
    await noteCreate(kind)
    await refresh()
    showToast(kind === 'todo' ? '任务便签已创建' : '便签已创建', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function toggleVisible(note: Note) {
  try {
    await noteToggleVisible(note.id)
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 切换便签皮肤（经典/横线/方格/极简/终端/霓虹） */
async function changeSkin(note: Note, e: Event) {
  const skin = (e.target as HTMLSelectElement).value
  try {
    await noteSetSkin(note.id, skin)
    note.skin = skin
  } catch (err) {
    showToast(String(err), 'error')
  }
}

/** 删除 → 两次点击确认；空白便签真删除，有内容的移入历史（可恢复） */
async function removeNote(note: Note) {
  if (confirmId.value !== note.id) {
    confirmId.value = note.id
    window.clearTimeout(confirmTimer)
    confirmTimer = window.setTimeout(() => {
      confirmId.value = ''
    }, 3000)
    return
  }
  window.clearTimeout(confirmTimer)
  confirmId.value = ''
  try {
    const purged = await noteDelete(note.id)
    await refresh()
    showToast(purged ? '空白便签已删除' : '已移入历史，可在下方恢复', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function restoreNote(note: Note) {
  try {
    await noteRestore(note.id)
    await refresh()
    showToast('便签已恢复', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 彻底删除：两次点击确认，不可恢复 */
async function purgeNote(note: Note) {
  if (purgeId.value !== note.id) {
    purgeId.value = note.id
    window.clearTimeout(confirmTimer)
    confirmTimer = window.setTimeout(() => {
      purgeId.value = ''
    }, 3000)
    return
  }
  window.clearTimeout(confirmTimer)
  purgeId.value = ''
  try {
    await notePurge(note.id)
    await refresh()
    showToast('已彻底删除', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 清空历史：两次点击确认 */
async function clearHistory() {
  if (!clearArmed.value) {
    clearArmed.value = true
    window.clearTimeout(confirmTimer)
    confirmTimer = window.setTimeout(() => {
      clearArmed.value = false
    }, 3000)
    return
  }
  window.clearTimeout(confirmTimer)
  clearArmed.value = false
  try {
    await noteClearHistory()
    await refresh()
    showToast('历史已清空', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

onMounted(async () => {
  unlistenLifecycle = onPluginLifecycle((active) => {
    if (active) void refresh()
  })
  unlistenChanges = await pluginOnEvent<void>('notes.changed', () => {
    void refresh()
  })
})
onBeforeUnmount(() => {
  window.clearTimeout(confirmTimer)
  unlistenChanges?.()
  unlistenChanges = null
  unlistenLifecycle?.()
  unlistenLifecycle = null
})
</script>

<template>
  <div class="notes-view">
    <div class="toolbar">
      <Button variant="default" size="sm" class="btn primary" @click="createNote('text')">
        <Icon name="plus" :size="14" /> 新建便签
      </Button>
      <Button variant="outline" size="sm" class="btn" @click="createNote('todo')">
        <Icon name="check" :size="14" /> 新建任务便签
      </Button>
      <span class="spacer" />
      <span class="hint">便签在桌面上可直接编辑、拖拽、缩放；有内容的删除进历史，空白便签直接删除</span>
    </div>

    <div class="scroll">
      <div class="grid">
        <div v-for="n in active" :key="n.id" class="note-card" :style="{ borderColor: n.color }">
          <div class="dot" :style="{ background: n.color }" />
          <div class="main">
            <div class="preview">
              <span v-if="n.kind === 'todo'" class="tag">任务</span>{{ preview(n) }}
            </div>
            <div class="meta">{{ meta(n) }}</div>
          </div>
          <div class="actions">
            <select
              class="skin-sel"
              :value="n.skin || 'classic'"
              title="便签皮肤"
              @change="changeSkin(n, $event)"
            >
              <option v-for="s in NOTE_SKINS" :key="s.id" :value="s.id">{{ s.label }}</option>
            </select>
            <Button variant="outline" size="sm" class="btn small" @click="toggleVisible(n)">
              {{ n.visible ? '隐藏' : '显示' }}
            </Button>
            <Button variant="destructive" size="sm"
              class="icon-btn danger"
              :class="{ armed: confirmId === n.id }"
              :title="confirmId === n.id ? '再点一次确认（移入历史）' : '删除'"
              @click="removeNote(n)"
            >
              <Icon name="trash" :size="14" />
            </Button>
          </div>
        </div>
        <div v-if="active.length === 0" class="empty">
          还没有便签 · 点击上方按钮添加第一张
        </div>
      </div>

      <template v-if="history.length">
        <div class="sec-head">
          <span class="sec-title">历史便签（{{ history.length }}）</span>
          <span class="spacer" />
          <Button variant="outline" size="sm" class="btn small danger-btn" :class="{ armed: clearArmed }" @click="clearHistory">
            {{ clearArmed ? '再点一次确认清空' : '清空历史' }}
          </Button>
        </div>
        <div class="grid">
          <div v-for="n in history" :key="n.id" class="note-card history">
            <div class="dot" :style="{ background: n.color }" />
            <div class="main">
              <div class="preview">
                <span v-if="n.kind === 'todo'" class="tag">任务</span>{{ preview(n) }}
              </div>
              <div class="meta">{{ meta(n) }}</div>
            </div>
            <div class="actions">
              <Button variant="outline" size="sm" class="btn small" @click="restoreNote(n)">恢复</Button>
              <Button variant="destructive" size="sm"
                class="icon-btn danger"
                :class="{ armed: purgeId === n.id }"
                :title="purgeId === n.id ? '再点一次彻底删除' : '彻底删除'"
                @click="purgeNote(n)"
              >
                <Icon name="trash" :size="14" />
              </Button>
            </div>
          </div>
        </div>
      </template>
    </div>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.notes-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.spacer {
  flex: 1;
}
.hint {
  font-size: 11.5px;
  color: var(--text-muted);
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
.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.btn.small {
  padding: 5px 10px;
  font-size: 11.5px;
}
.danger-btn {
  color: var(--danger);
}
.danger-btn.armed {
  background: var(--danger);
  border-color: var(--danger);
  color: #fff;
}
.scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.sec-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
}
.sec-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-muted);
}
.grid {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.note-card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-left-width: 4px;
  border-radius: 10px;
  background: var(--surface-subtle);
}
.note-card.history {
  opacity: 0.62;
}
.dot {
  flex: none;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 1px solid rgba(0, 0, 0, 0.15);
}
.main {
  flex: 1;
  min-width: 0;
}
.preview {
  font-size: 12.5px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag {
  display: inline-block;
  margin-right: 6px;
  padding: 0 6px;
  border-radius: 6px;
  font-size: 10.5px;
  line-height: 17px;
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--accent);
  vertical-align: 1px;
}
.meta {
  margin-top: 3px;
  font-size: 11px;
  color: var(--text-muted);
}
.actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 6px;
}
.skin-sel {
  padding: 4px 6px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 11.5px;
  outline: none;
}
.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.icon-btn:hover {
  background: var(--surface-container);
  color: var(--danger);
}
.icon-btn.armed {
  background: var(--danger);
  color: #fff;
}
.empty {
  padding: 40px 0;
  text-align: center;
  font-size: 12.5px;
  color: var(--text-muted);
}
</style>
