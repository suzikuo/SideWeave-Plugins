<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  addHistoryTag,
  clearHistory,
  clearThumbCache,
  copyFile,
  deleteHistoryItem,
  openHistoryFile,
  pinFile,
  removeHistoryTag,
  type HistoryItem,
} from '../api'
import {
  cachedHistoryItems,
  cachedHistoryThumbs,
  clearHistoryMemoryCache,
  ensureThumbnails,
  evictHistoryCacheItem,
  historyCacheReady,
  isImagePath,
  loadHistoryCached,
} from '../historyCache'

import ContextMenu, { type ContextMenuItem } from './ContextMenu.vue'

const emit = defineEmits<{
  toast: [message: string]
  sendToEditor: [path: string]
}>()

const items = cachedHistoryItems
const thumbs = cachedHistoryThumbs
const loading = ref(false)
const deletingPath = ref<string | null>(null)
const kindFilter = ref<string>('all')
const tagFilter = ref<string>('')
const query = ref('')
const tagInputFor = ref<string | null>(null)
const tagDraft = ref('')
const ctxMenu = ref<{ visible: boolean; x: number; y: number; item: HistoryItem | null }>({
  visible: false,
  x: 0,
  y: 0,
  item: null,
})

const ctxMenuItems = computed<ContextMenuItem[]>(() => {
  const item = ctxMenu.value.item
  if (!item) return []
  const img = isImagePath(item.path)
  const list: ContextMenuItem[] = []
  if (img) {
    list.push(
      { id: 'copy', label: '复制到剪贴板' },
      { id: 'pin', label: '置顶贴图' },
      { id: 'canvas', label: '发送到自由画布' },
      { id: 'sep1', divider: true },
    )
  }
  list.push(
    { id: 'open', label: '打开文件' },
    { id: 'reveal', label: '在资源管理器中定位' },
    { id: 'tag', label: '添加自定义标签…' },
    { id: 'sep2', divider: true },
    { id: 'delete', label: '删除并移入回收站', danger: true },
  )
  return list
})

function onCardContextMenu(e: MouseEvent, item: HistoryItem) {
  ctxMenu.value = { visible: true, x: e.clientX, y: e.clientY, item }
}

function onCtxMenuSelect(id: string) {
  const item = ctxMenu.value.item
  if (!item) return
  if (id === 'copy') void onCopy(item)
  else if (id === 'pin') void onPin(item)
  else if (id === 'canvas') emit('sendToEditor', item.path)
  else if (id === 'open') void onOpen(item, false)
  else if (id === 'reveal') void onOpen(item, true)
  else if (id === 'tag') {
    tagInputFor.value = item.path
    tagDraft.value = ''
  } else if (id === 'delete') void onDeleteToRecycleBin(item)
}

const allTags = computed(() => {
  const set = new Set<string>()
  for (const item of items.value) {
    for (const t of item.tags) set.add(t)
  }
  return Array.from(set)
})

const filteredItems = computed(() => {
  const q = query.value.trim().toLowerCase()
  return items.value.filter((it) => {
    if (kindFilter.value !== 'all' && it.kind !== kindFilter.value) return false
    if (tagFilter.value && !it.tags.includes(tagFilter.value)) return false
    if (q) {
      const name = fileName(it.path).toLowerCase()
      const hitTag = it.tags.some((t) => t.toLowerCase().includes(q))
      if (!name.includes(q) && !hitTag) return false
    }
    return true
  })
})

function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

function formatSize(bytes: number): string {
  if (!bytes) return ''
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`
}

function formatTime(ms: number): string {
  if (!ms) return ''
  const d = new Date(ms)
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function kindLabel(kind: string): string {
  switch (kind) {
    case 'scroll':
      return '长截图'
    case 'recording':
      return '录屏'
    case 'edit':
      return '编辑导出'
    default:
      return '截图'
  }
}

async function refresh(force = true) {
  const showSpinner = !historyCacheReady.value || !items.value.length
  if (showSpinner) loading.value = true
  try {
    const list = await loadHistoryCached(force)
    void ensureThumbnails(list, 60)
  } catch (err) {
    emit('toast', `加载历史失败：${String(err)}`)
  } finally {
    loading.value = false
  }
}

async function onCopy(item: HistoryItem) {
  try {
    await copyFile(item.path)
    emit('toast', '图片已复制到剪贴板')
  } catch (err) {
    emit('toast', `复制失败：${String(err)}`)
  }
}

async function onPin(item: HistoryItem) {
  try {
    await pinFile(item.path)
    emit('toast', '已置顶贴图')
  } catch (err) {
    emit('toast', `贴图失败：${String(err)}`)
  }
}

async function onOpen(item: HistoryItem, reveal = false) {
  try {
    await openHistoryFile(item.path, reveal)
  } catch (err) {
    emit('toast', String(err))
  }
}

async function onAddTag(item: HistoryItem) {
  const t = tagDraft.value.trim()
  if (!t) return
  try {
    const tags = await addHistoryTag(item.id, item.path, t)
    item.tags = tags
    tagDraft.value = ''
    tagInputFor.value = null
  } catch (err) {
    emit('toast', String(err))
  }
}

async function onRemoveTag(item: HistoryItem, tag: string) {
  try {
    const tags = await removeHistoryTag(item.id, item.path, tag)
    item.tags = tags
  } catch (err) {
    emit('toast', String(err))
  }
}

async function onDeleteToRecycleBin(item: HistoryItem) {
  if (deletingPath.value === item.path) return
  deletingPath.value = item.path
  try {
    await deleteHistoryItem(item.id, item.path, true)
    evictHistoryCacheItem(item.path)
    emit('toast', '已删除并移入系统回收站')
  } catch (err) {
    emit('toast', `删除失败：${String(err)}`)
  } finally {
    deletingPath.value = null
  }
}

async function onClearCache() {
  try {
    await clearThumbCache()
    clearHistoryMemoryCache()
    await refresh(true)
    emit('toast', '缩略图缓存已清理并重新加载')
  } catch (err) {
    emit('toast', String(err))
  }
}

async function onClearAll() {
  try {
    await clearHistory()
    await refresh(true)
    emit('toast', '已清空历史索引（保留磁盘文件）')
  } catch (err) {
    emit('toast', String(err))
  }
}

onMounted(() => {
  // Show cached items immediately (0ms), then quietly sync new captures in background
  if (historyCacheReady.value && items.value.length) {
    void ensureThumbnails(items.value, 60)
  }
  void refresh(true)
})
</script>

<template>
  <div class="sn-panel">
    <div class="sn-section-head">
      <div>
        <h2 class="sn-title">历史记录</h2>
        <p class="sn-subtitle">自动索引截图、长图、录屏与文档导出，支持自定义标签与一键贴图 / 拼图</p>
      </div>
      <div class="hist-head-btns">
        <button type="button" class="sn-ghost-btn" @click="refresh(true)">刷新</button>
        <button type="button" class="sn-ghost-btn" @click="onClearCache">清理缓存</button>
        <button type="button" class="sn-ghost-btn" @click="onClearAll">清空索引</button>
      </div>
    </div>

    <div class="hist-toolbar">
      <div class="sn-seg">
        <button
          v-for="k in [
            { id: 'all', label: '全部' },
            { id: 'screenshot', label: '截图' },
            { id: 'scroll', label: '长图' },
            { id: 'recording', label: '录屏' },
            { id: 'edit', label: '拼图/文档' },
          ]"
          :key="k.id"
          type="button"
          class="sn-seg-btn"
          :class="{ active: kindFilter === k.id }"
          @click="kindFilter = k.id"
        >
          {{ k.label }}
        </button>
      </div>
      <input v-model="query" class="sn-input hist-search" placeholder="搜索文件名或标签…" />
    </div>

    <div v-if="allTags.length" class="tag-filter-bar">
      <button
        type="button"
        class="tag-pill"
        :class="{ active: tagFilter === '' }"
        @click="tagFilter = ''"
      >
        全部标签
      </button>
      <button
        v-for="tag in allTags"
        :key="tag"
        type="button"
        class="tag-pill"
        :class="{ active: tagFilter === tag }"
        @click="tagFilter = tagFilter === tag ? '' : tag"
      >
        # {{ tag }}
      </button>
    </div>

    <div v-if="loading && !items.length" class="hist-empty">正在加载历史记录…</div>
    <div v-else-if="!filteredItems.length" class="hist-empty">
      暂无符合条件的记录，完成截图或拼图导出后将自动出现在这里
    </div>

    <div v-else class="hist-grid">
      <div
        v-for="item in filteredItems"
        :key="item.path"
        class="hist-card"
        @contextmenu.prevent="onCardContextMenu($event, item)"
      >
        <div class="hist-thumb" @click="onOpen(item, false)">
          <img v-if="thumbs[item.path]" :src="thumbs[item.path]" :alt="fileName(item.path)" />
          <div v-else class="hist-thumb-fallback">
            <span>{{ fileName(item.path).split('.').pop()?.toUpperCase() || 'FILE' }}</span>
          </div>
          <span class="hist-kind">{{ kindLabel(item.kind) }}</span>
        </div>

        <div class="hist-meta">
          <div class="hist-name" :title="item.path">{{ fileName(item.path) }}</div>
          <div class="hist-sub">
            <span v-if="item.width && item.height">{{ item.width }}×{{ item.height }}</span>
            <span v-if="item.fileSize">{{ formatSize(item.fileSize) }}</span>
            <span>{{ formatTime(item.createdAtMs) }}</span>
          </div>

          <div class="hist-tags">
            <span v-for="tag in item.tags" :key="tag" class="mini-tag">
              {{ tag }}
              <button type="button" @click.stop="onRemoveTag(item, tag)">×</button>
            </span>
            <button
              type="button"
              class="mini-tag add"
              @click="tagInputFor = tagInputFor === item.path ? null : item.path; tagDraft = ''"
            >
              + 标签
            </button>
          </div>

          <div v-if="tagInputFor === item.path" class="tag-editor">
            <input
              v-model="tagDraft"
              class="sn-input"
              placeholder="输入标签回车"
              @keydown.enter.prevent="onAddTag(item)"
            />
            <button type="button" class="sn-primary-btn sm" @click="onAddTag(item)">添加</button>
          </div>

          <div class="hist-actions">
            <button
              v-if="isImagePath(item.path)"
              type="button"
              class="hist-act"
              title="复制到剪贴板"
              @click="onCopy(item)"
            >
              复制
            </button>
            <button
              v-if="isImagePath(item.path)"
              type="button"
              class="hist-act"
              title="置顶贴图"
              @click="onPin(item)"
            >
              贴图
            </button>
            <button
              v-if="isImagePath(item.path)"
              type="button"
              class="hist-act"
              title="发送到自由画布"
              @click="emit('sendToEditor', item.path)"
            >
              画布
            </button>
            <button
              type="button"
              class="hist-act"
              title="在资源管理器中定位"
              @click="onOpen(item, true)"
            >
              定位
            </button>
            <button
              type="button"
              class="hist-act danger"
              :disabled="deletingPath === item.path"
              title="删除文件并移入系统回收站"
              @click="onDeleteToRecycleBin(item)"
            >
              {{ deletingPath === item.path ? '删除中…' : '删除' }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <ContextMenu
      :visible="ctxMenu.visible"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :items="ctxMenuItems"
      @select="onCtxMenuSelect"
      @close="ctxMenu.visible = false"
    />
  </div>
</template>

<style scoped>
.hist-head-btns {
  display: flex;
  gap: 8px;
}
.hist-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 12px;
}
.hist-search {
  max-width: 220px;
}
.tag-filter-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 12px;
}
.tag-pill {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-muted);
  font-size: 11.5px;
  cursor: pointer;
}
.tag-pill.active {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
  background: color-mix(in srgb, var(--sn-accent) 14%, transparent);
}
.hist-empty {
  padding: 48px 16px;
  text-align: center;
  color: var(--sn-muted);
  font-size: 13px;
  border-radius: 12px;
  border: 1px dashed var(--sn-border);
}
.hist-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(215px, 1fr));
  gap: 12px;
}
.hist-card {
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  transition: border-color 0.15s ease;
}
.hist-card:hover {
  border-color: var(--sn-accent);
}
.hist-thumb {
  position: relative;
  height: 128px;
  background: var(--sn-card);
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  cursor: pointer;
}
.hist-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.hist-thumb-fallback {
  font-size: 16px;
  font-weight: 700;
  color: var(--sn-muted);
  letter-spacing: 0.08em;
}
.hist-kind {
  position: absolute;
  top: 7px;
  left: 7px;
  padding: 2px 7px;
  border-radius: 6px;
  background: rgba(20, 20, 22, 0.78);
  color: #f4f4f6;
  font-size: 10.5px;
  font-weight: 600;
}
.hist-meta {
  padding: 10px 11px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.hist-name {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--sn-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hist-sub {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  font-size: 11px;
  color: var(--sn-muted);
}
.hist-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.mini-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border-radius: 6px;
  background: var(--sn-card);
  border: 1px solid var(--sn-border);
  color: var(--sn-text);
  font-size: 10.5px;
}
.mini-tag button {
  border: none;
  background: transparent;
  color: var(--sn-muted);
  cursor: pointer;
  padding: 0;
  font-size: 12px;
}
.mini-tag.add {
  color: var(--sn-muted);
  cursor: pointer;
}
.mini-tag.add:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}
.tag-editor {
  display: flex;
  gap: 6px;
}
.hist-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 2px;
}
.hist-act {
  flex: 1;
  min-width: 38px;
  padding: 4px 5px;
  border-radius: 6px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-text);
  font-size: 11px;
  cursor: pointer;
}
.hist-act:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}
.hist-act.danger:hover {
  border-color: #ef4444;
  color: #ef4444;
}
</style>
