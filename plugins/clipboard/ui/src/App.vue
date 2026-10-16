<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 剪贴板历史详情页：自动记录（后台轮询）、搜索、一键复制、删除/清空、容量与开关设置 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import { onPluginLifecycle, pluginOnEvent, type UnlistenFn } from '../../../ui-sdk/runtime'
import {
  clipboardClear,
  clipboardPinToggle,
  clipboardCopyItem,
  clipboardDeleteItem,
  clipboardGetSettings,
  clipboardList,
  clipboardSetEnabled,
  clipboardSetMax
} from './api'
import type { ClipItem, ClipboardSettings } from './api'

const items = ref<ClipItem[]>([])
const settings = ref<ClipboardSettings>({ enabled: true, max: 100 })
const search = ref('')
const copiedId = ref('')
/** 二次确认清空 */
const confirmClear = ref(false)
let confirmClearTimer = 0

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  const list = items.value
  if (!q) return list
  return list.filter((i) => i.text.toLowerCase().includes(q))
})

async function refresh() {
  try {
    items.value = await clipboardList()
    settings.value = await clipboardGetSettings()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function copyItem(item: ClipItem) {
  try {
    await clipboardCopyItem(item.id)
    copiedId.value = item.id
    window.setTimeout(() => {
      if (copiedId.value === item.id) copiedId.value = ''
    }, 1200)
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function pinToggle(item: ClipItem) {
  try {
    await clipboardPinToggle(item.id)
    item.pinned = !item.pinned
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function removeItem(item: ClipItem) {
  try {
    await clipboardDeleteItem(item.id)
    items.value = items.value.filter((i) => i.id !== item.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function clearAll() {
  if (!confirmClear.value) {
    confirmClear.value = true
    window.clearTimeout(confirmClearTimer)
    confirmClearTimer = window.setTimeout(() => {
      confirmClear.value = false
    }, 3000)
    return
  }
  window.clearTimeout(confirmClearTimer)
  confirmClear.value = false
  try {
    await clipboardClear()
    items.value = []
    showToast('已清空历史', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function setEnabled(enabled: boolean) {
  settings.value.enabled = enabled
  try {
    await clipboardSetEnabled(enabled)
    showToast(enabled ? '已开始记录剪贴板' : '已暂停记录', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function setMax(max: number) {
  const safe = Math.min(1000, Math.max(10, Math.floor(max) || 100))
  settings.value.max = safe
  try {
    await clipboardSetMax(safe)
    await refresh()
    showToast(`历史上限 ${safe} 条`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function preview(text: string): string {
  const first = text.split('\n')[0]
  return first.length > 160 ? `${first.slice(0, 160)}…` : first || '(空白)'
}

function fmtTime(iso: string): string {
  return iso ? iso.replace('T', ' ').slice(5, 16) : ''
}

let unlistenChanges: UnlistenFn | null = null
let unlistenLifecycle: (() => void) | null = null

onMounted(async () => {
  unlistenLifecycle = onPluginLifecycle((active) => {
    if (active) void refresh()
  })
  unlistenChanges = await pluginOnEvent<void>('clipboard.changed', () => {
    void refresh()
  })
})

onBeforeUnmount(() => {
  unlistenChanges?.()
  unlistenLifecycle?.()
})
</script>

<template>
  <div class="clip-view">
    <div class="toolbar">
      <label class="switch" title="开启后自动记录复制的文本">
        <input type="checkbox" :checked="settings.enabled" @change="setEnabled(($event.target as HTMLInputElement).checked)" />
        <span class="slider" />
      </label>
      <span class="state">{{ settings.enabled ? '记录中' : '已暂停' }}</span>
      <span class="spacer" />
      <label class="max-wrap" title="历史条数上限">
        <span>上限</span>
        <input
          class="max-input"
          type="number"
          min="10"
          max="1000"
          :value="settings.max"
          @change="setMax(Number(($event.target as HTMLInputElement).value))"
        />
        <span>条</span>
      </label>
      <Button variant="destructive" size="sm"
        class="btn danger"
        :class="{ armed: confirmClear }"
        :disabled="items.length === 0"
        @click="clearAll"
      >
        {{ confirmClear ? '确认清空' : '清空历史' }}
      </Button>
    </div>

    <div class="search-row">
      <input v-model="search" type="text" placeholder="搜索历史内容…" spellcheck="false" />
      <span class="count">{{ filtered.length }} 条</span>
    </div>

    <div class="list">
      <div v-for="item in filtered" :key="item.id" class="item" :class="{ pinned: item.pinned }" @click="copyItem(item)">
        <div class="text">
          <Icon v-if="item.pinned" name="bookmark" :size="12" class="pin-mark" />
          {{ preview(item.text) }}
        </div>
        <div class="meta">
          <span>{{ fmtTime(item.ts) }}</span>
          <span>{{ item.text.length }} 字符</span>
          <span v-if="copiedId === item.id" class="copied">已复制 ✓</span>
          <Button variant="outline" size="sm"
            class="icon-btn"
            :class="{ on: item.pinned }"
            :title="item.pinned ? '取消收藏' : '收藏（置顶且不淘汰）'"
            @click.stop="pinToggle(item)"
          >
            <Icon name="bookmark" :size="13" />
          </Button>
          <Button variant="outline" size="sm"
            class="icon-btn"
            title="删除该条"
            @click.stop="removeItem(item)"
          >
            <Icon name="trash" :size="13" />
          </Button>
        </div>
      </div>
      <div v-if="filtered.length === 0" class="empty">
        {{ settings.enabled ? '还没有记录 · 复制任意文本后会自动出现在这里' : '记录已暂停 · 打开上方开关开始记录' }}
      </div>
    </div>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.clip-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 10px;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
}
.state {
  font-size: 12.5px;
  color: var(--text-muted);
}
.spacer {
  flex: 1;
}
.max-wrap {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
}
.max-input {
  width: 64px;
  padding: 5px 8px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
}

.btn {
  padding: 7px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 12.5px;
}
.btn.danger {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
}
.btn.danger.armed {
  background: var(--danger);
  color: #fff;
}
.btn:disabled {
  opacity: 0.5;
}

.search-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.search-row input {
  flex: 1;
  padding: 8px 12px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  outline: none;
}
.search-row input:focus {
  border-color: var(--primary);
}
.count {
  font-size: 12px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.item {
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: 10px;
  background: var(--surface-subtle);
  cursor: pointer;
}
.item:hover {
  border-color: var(--primary);
}
.item.pinned {
  border-color: color-mix(in srgb, var(--primary) 45%, transparent);
}
.pin-mark {
  color: var(--primary);
  margin-right: 6px;
  vertical-align: -2px;
}
.icon-btn.on {
  color: var(--primary);
}
.text {
  font-size: 12.5px;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-all;
}
.meta {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 6px;
  font-size: 11px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
.copied {
  color: var(--success, #4ade80);
}
.icon-btn {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}
.icon-btn:hover {
  background: var(--surface-container);
  color: var(--danger);
}
.empty {
  padding: 40px 0;
  text-align: center;
  font-size: 12.5px;
  color: var(--text-muted);
}

/* 开关 */
.switch {
  position: relative;
  display: inline-block;
  width: 34px;
  height: 19px;
  cursor: pointer;
}
.switch input {
  display: none;
}
.switch .slider {
  position: absolute;
  inset: 0;
  border-radius: 999px;
  background: var(--border);
  transition: background 0.18s;
}
.switch .slider::before {
  content: '';
  position: absolute;
  top: 2.5px;
  left: 3px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.18s;
}
.switch input:checked + .slider {
  background: var(--primary);
}
.switch input:checked + .slider::before {
  transform: translateX(14px);
  background: var(--primary-foreground);
}
</style>
