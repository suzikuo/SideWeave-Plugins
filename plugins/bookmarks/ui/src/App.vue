<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** Bookmarks 书签插件详情页：标签过滤 + 搜索 + 置顶分组 + 双击直达（Pulse 深色胶囊风格） */
import { computed, onMounted, ref } from 'vue'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import { bmAdd, bmGetView, bmList, bmOpen, bmRemove, bmSetView, bmTogglePreview, bmUpdate } from './api'
import type { Bookmark, BookmarkInput, BookmarkViewStyle } from './api'

// ---------- 状态 ----------

const items = ref<Bookmark[]>([])
const search = ref('')
/** 当前选中的过滤标签；null = 全部 */
const activeTag = ref<string | null>(null)
/** 列表视图样式：卡片（默认）/ 单条 / 表格（跨会话记忆） */
const view = ref<BookmarkViewStyle>('card')

async function setView(v: BookmarkViewStyle) {
  view.value = v
  try {
    await bmSetView(v)
  } catch {
    // 持久化失败不影响本次预览
  }
}

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

const showDialog = ref(false)
const editingId = ref<string | null>(null)
const form = ref(defaultForm())
const showErrors = ref(false)

/** 待二次确认删除的书签 id（3 秒未确认自动复位） */
const confirmId = ref<string | null>(null)
let confirmTimer = 0

function defaultForm() {
  return { name: '', url: '', tags: '', pinned: false, in_preview: false }
}

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

async function refresh() {
  try {
    items.value = await bmList()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

onMounted(async () => {
  await refresh()
  try {
    const v = await bmGetView()
    if (v === 'card' || v === 'compact' || v === 'table') view.value = v
  } catch {
    // 读取失败保持默认卡片
  }
})

// ---------- 过滤与分组 ----------

/** 全部书签的去重标签（中文排序） */
const allTags = computed(() => {
  const set = new Set<string>()
  for (const it of items.value) {
    for (const t of it.tags) {
      const tag = t.trim()
      if (tag) set.add(tag)
    }
  }
  return [...set].sort((a, b) => a.localeCompare(b, 'zh-Hans-CN'))
})

const filtered = computed(() => {
  const tag = activeTag.value
  const q = search.value.trim().toLowerCase()
  return items.value.filter((it) => {
    if (tag && !it.tags.includes(tag)) return false
    if (!q) return true
    return [it.name, it.url, it.tags.join(' ')].some((f) => f.toLowerCase().includes(q))
  })
})

/** 置顶区 / 普通区（组内按创建时间倒序） */
const pinnedList = computed(() =>
  filtered.value.filter((it) => it.pinned).sort((a, b) => b.created_at - a.created_at)
)
const normalList = computed(() =>
  filtered.value.filter((it) => !it.pinned).sort((a, b) => b.created_at - a.created_at)
)

/** 渲染分区：仅有置顶时出现"已置顶"，普通区标题固定为"全部" */
const sections = computed<{ title: string; rows: Bookmark[] }[]>(() => {
  const list: { title: string; rows: Bookmark[] }[] = []
  if (pinnedList.value.length) list.push({ title: '已置顶', rows: pinnedList.value })
  if (normalList.value.length || !pinnedList.value.length) {
    list.push({ title: '全部', rows: normalList.value })
  }
  return list
})

// ---------- 列表操作 ----------

async function open(it: Bookmark) {
  try {
    await bmOpen(it.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function togglePin(it: Bookmark) {
  try {
    await bmUpdate(it.id, {
      name: it.name,
      url: it.url,
      tags: it.tags,
      pinned: !it.pinned,
      in_preview: it.in_preview
    })
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function togglePreview(it: Bookmark) {
  try {
    const res = await bmTogglePreview(it.id)
    it.in_preview = res.in_preview
    showToast(
      res.in_preview ? `已将「${it.name}」加入悬浮预览` : `已从悬浮预览移除「${it.name}」`,
      'success'
    )
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 两次点击确认删除：首次进入确认态（3 秒自动恢复），再次点击执行删除 */
function requestRemove(it: Bookmark) {
  if (confirmId.value === it.id) {
    clearConfirm()
    void doRemove(it.id)
    return
  }
  confirmId.value = it.id
  window.clearTimeout(confirmTimer)
  confirmTimer = window.setTimeout(clearConfirm, 3000)
}

function clearConfirm() {
  confirmId.value = null
  window.clearTimeout(confirmTimer)
}

async function doRemove(id: string) {
  try {
    await bmRemove(id)
    showToast('已删除书签', 'success')
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 新增 / 编辑 ----------

const nameError = computed(() =>
  showErrors.value && !form.value.name.trim() ? '请输入名称' : ''
)
const urlError = computed(() => {
  if (!showErrors.value) return ''
  const u = form.value.url.trim()
  if (!u) return '请输入网址'
  if (!/^https?:\/\//.test(u)) return '网址必须以 http:// 或 https:// 开头'
  return ''
})

function openAdd() {
  editingId.value = null
  form.value = defaultForm()
  showErrors.value = false
  showDialog.value = true
}

function openEdit(it: Bookmark) {
  editingId.value = it.id
  form.value = {
    name: it.name,
    url: it.url,
    tags: it.tags.join(', '),
    pinned: it.pinned,
    in_preview: it.in_preview ?? false
  }
  showErrors.value = false
  showDialog.value = true
}

async function save() {
  showErrors.value = true
  if (nameError.value || urlError.value) return
  const input: BookmarkInput = {
    name: form.value.name.trim(),
    url: form.value.url.trim(),
    tags: form.value.tags
      .split(/[,，]/)
      .map((t) => t.trim())
      .filter(Boolean),
    pinned: form.value.pinned,
    in_preview: form.value.in_preview
  }
  try {
    if (editingId.value) {
      await bmUpdate(editingId.value, input)
      showToast('已保存修改', 'success')
    } else {
      await bmAdd(input)
      showToast('已添加书签', 'success')
    }
    showDialog.value = false
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 字母头像 ----------

/** 头像底色（与深色主题协调的柔和色板，按名称哈希取色） */
const AVATAR_COLORS = [
  '#1f5f6b', // 青
  '#41568b', // 蓝
  '#5a4a86', // 紫
  '#7a4f74', // 玫紫
  '#81563f', // 陶橙
  '#6e6a35', // 橄榄
  '#3f6e52', // 绿
  '#5d5d64' // 石灰
]

function avatarBg(name: string): string {
  let h = 0
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0
  return AVATAR_COLORS[Math.abs(h) % AVATAR_COLORS.length]
}

function avatarChar(name: string): string {
  const t = name.trim()
  return t ? t.charAt(0).toUpperCase() : '?'
}
</script>

<template>
  <div class="bookmarks">
    <div class="toolbar">
      <div class="toolbar-left">
        <Button variant="outline" size="sm" class="add-btn" @click="openAdd"><Icon name="plus" :size="14" /> 新增书签</Button>
        <div class="style-switch" title="列表样式">
          <button :class="{ active: view === 'card' }" @click="setView('card')">卡片</button>
          <button :class="{ active: view === 'compact' }" @click="setView('compact')">单条</button>
          <button :class="{ active: view === 'table' }" @click="setView('table')">表格</button>
        </div>
      </div>
      <div class="search">
        <Icon name="search" :size="14" />
        <input v-model="search" type="text" placeholder="搜索名称、标签或网址" />
      </div>
    </div>

    <div v-if="allTags.length" class="tagbar">
      <button class="chip" :class="{ active: activeTag === null }" @click="activeTag = null">
        全部
      </button>
      <button
        v-for="tag in allTags"
        :key="tag"
        class="chip"
        :class="{ active: activeTag === tag }"
        @click="activeTag = activeTag === tag ? null : tag"
      >
        {{ tag }}
      </button>
    </div>

    <div class="list">
      <template v-if="sections.length">
        <section v-for="sec in sections" :key="sec.title" class="section">
          <div class="section-title">{{ sec.title }}</div>

          <!-- 卡片 / 单条：同一行结构，样式随 view 切换 -->
          <div v-if="view !== 'table'" class="rows">
            <div
              v-for="it in sec.rows"
              :key="it.id"
              class="row"
              :class="[view === 'compact' ? 'compact' : 'card', { confirming: confirmId === it.id }]"
              @dblclick="open(it)"
            >
              <span v-if="view === 'card'" class="avatar" :style="{ background: avatarBg(it.name) }">
                {{ avatarChar(it.name) }}
              </span>
              <span v-else class="mini-dot" :style="{ background: avatarBg(it.name) }" />
              <span class="meta">
                <span class="name">
                  <span class="name-text">{{ it.name }}</span>
                  <span v-if="it.pinned" class="pin-badge" title="已置顶">
                    <Icon name="bookmark" :size="9" />
                  </span>
                  <span v-if="it.in_preview" class="preview-badge" title="已加入悬浮预览">
                    <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" />
                      <circle cx="12" cy="12" r="3" />
                    </svg>
                  </span>
                </span>
                <span class="url" :title="it.url">{{ it.url }}</span>
              </span>
              <span class="actions">
                <Button variant="outline" size="sm" class="icon-btn" title="打开" @click.stop="open(it)">
                  <Icon name="play" :size="14" />
                </Button>
                <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click.stop="openEdit(it)">
                  <Icon name="pencil" :size="14" />
                </Button>
                <Button variant="outline" size="sm"
                  class="icon-btn"
                  :class="{ active: it.pinned }"
                  :title="it.pinned ? '取消置顶' : '置顶'"
                  @click.stop="togglePin(it)"
                >
                  <Icon name="bookmark" :size="14" />
                </Button>
                <Button variant="outline" size="sm"
                  class="icon-btn"
                  :class="{ active: it.in_preview }"
                  :title="it.in_preview ? '从悬浮预览移除' : '加入悬浮预览'"
                  @click.stop="togglePreview(it)"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" />
                    <circle cx="12" cy="12" r="3" />
                  </svg>
                </Button>
                <Button variant="outline" size="sm" v-if="confirmId === it.id" class="confirm-btn" @click.stop="requestRemove(it)">
                  再点一次确认删除
                </Button>
                <Button variant="destructive" size="sm" v-else class="icon-btn danger" title="删除" @click.stop="requestRemove(it)">
                  <Icon name="trash" :size="14" />
                </Button>
              </span>
            </div>
          </div>

          <!-- 表格：名称 / 网址 / 标签 / 操作 -->
          <div v-else class="bm-table">
            <div class="thead">
              <span>名称</span>
              <span>网址</span>
              <span>标签</span>
              <span class="th-ops">操作</span>
            </div>
            <div
              v-for="it in sec.rows"
              :key="it.id"
              class="tr"
              :class="{ confirming: confirmId === it.id }"
              @dblclick="open(it)"
            >
              <span class="td td-name">
                <span class="name-text">{{ it.name }}</span>
                <span v-if="it.pinned" class="pin-badge" title="已置顶">
                  <Icon name="bookmark" :size="9" />
                </span>
                <span v-if="it.in_preview" class="preview-badge" title="已加入悬浮预览">
                  <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" />
                    <circle cx="12" cy="12" r="3" />
                  </svg>
                </span>
              </span>
              <span class="td td-url" :title="it.url">{{ it.url }}</span>
              <span class="td td-tags">
                <span v-for="t in it.tags" :key="t" class="mini-tag">{{ t }}</span>
              </span>
              <span class="td td-ops">
                <span class="actions inline">
                  <Button variant="outline" size="sm" class="icon-btn" title="打开" @click.stop="open(it)">
                    <Icon name="play" :size="14" />
                  </Button>
                  <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click.stop="openEdit(it)">
                    <Icon name="pencil" :size="14" />
                  </Button>
                  <Button variant="outline" size="sm"
                    class="icon-btn"
                    :class="{ active: it.pinned }"
                    :title="it.pinned ? '取消置顶' : '置顶'"
                    @click.stop="togglePin(it)"
                  >
                    <Icon name="bookmark" :size="14" />
                  </Button>
                  <Button variant="outline" size="sm"
                    class="icon-btn"
                    :class="{ active: it.in_preview }"
                    :title="it.in_preview ? '从悬浮预览移除' : '加入悬浮预览'"
                    @click.stop="togglePreview(it)"
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" />
                      <circle cx="12" cy="12" r="3" />
                    </svg>
                  </Button>
                  <Button variant="outline" size="sm" v-if="confirmId === it.id" class="confirm-btn" @click.stop="requestRemove(it)">
                    再点一次确认删除
                  </Button>
                  <Button variant="destructive" size="sm" v-else class="icon-btn danger" title="删除" @click.stop="requestRemove(it)">
                    <Icon name="trash" :size="14" />
                  </Button>
                </span>
              </span>
            </div>
          </div>
        </section>
      </template>
      <div v-else-if="items.length === 0" class="empty">
        <div class="empty-icon"><Icon name="bookmark" :size="34" /></div>
        <div class="empty-title">还没有书签</div>
        <div class="empty-sub">点击左上角「新增书签」，把常用网址收进来，双击即可直达</div>
      </div>
      <div v-else class="no-match">没有匹配的书签</div>
    </div>

    <Teleport to="body">
      <div v-if="showDialog" class="mask" @click.self="showDialog = false">
        <div class="dialog">
          <h2>{{ editingId ? '编辑书签' : '新增书签' }}</h2>

          <label class="field">
            <span>名称 *</span>
            <input
              v-model="form.name"
              type="text"
              placeholder="例如：GitHub"
              :class="{ invalid: !!nameError }"
              @keydown.enter="save"
            />
            <span v-if="nameError" class="field-err">{{ nameError }}</span>
          </label>

          <label class="field">
            <span>网址 *</span>
            <input
              v-model="form.url"
              type="text"
              placeholder="https://example.com"
              :class="{ invalid: !!urlError }"
              @keydown.enter="save"
            />
            <span v-if="urlError" class="field-err">{{ urlError }}</span>
          </label>

          <label class="field">
            <span>标签（逗号分隔）</span>
            <input
              v-model="form.tags"
              type="text"
              placeholder="例如：开发, 工具"
              @keydown.enter="save"
            />
          </label>

          <div class="switch-row">
            <span>置顶</span>
            <button
              class="switch"
              :class="{ on: form.pinned }"
              role="switch"
              :aria-checked="form.pinned"
              @click="form.pinned = !form.pinned"
            >
              <span class="thumb" />
            </button>
          </div>

          <div class="switch-row">
            <span>悬浮预览展示</span>
            <button
              class="switch"
              :class="{ on: form.in_preview }"
              role="switch"
              :aria-checked="form.in_preview"
              @click="form.in_preview = !form.in_preview"
            >
              <span class="thumb" />
            </button>
          </div>

          <footer class="dialog-foot">
            <Button variant="outline" size="sm" class="btn" @click="showDialog = false">取消</Button>
            <Button variant="default" size="sm" class="btn primary" @click="save">保存</Button>
          </footer>
        </div>
      </div>
    </Teleport>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.bookmarks {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}

/* ---------- 顶栏 ---------- */

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.add-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 18px;
  border: none;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
  box-shadow: 0 4px 14px color-mix(in srgb, var(--accent) 35%, transparent);
}
.add-btn:hover {
  filter: brightness(1.12);
}
.add-btn:active {
  transform: translateY(1px);
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 300px;
  padding: 8px 14px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-subtle) 80%, transparent);
  color: var(--text-muted);
  transition: border-color 0.15s ease;
}
.search:focus-within {
  border-color: var(--accent);
}
.search input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 12.5px;
}
.search input::placeholder {
  color: var(--text-muted);
}

/* ---------- 标签过滤 ---------- */

.tagbar {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip {
  padding: 4px 13px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-subtle) 70%, transparent);
  color: var(--text-muted);
  font-size: 11.5px;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}
.chip:hover {
  border-color: var(--border-strong);
  color: var(--text);
}
.chip.active {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
  font-weight: 600;
}

/* ---------- 视图样式切换 ---------- */

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.style-switch {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-subtle) 70%, transparent);
}
.style-switch button {
  padding: 4px 12px;
  border: none;
  border-radius: 999px;
  background: transparent;
  color: var(--text-muted);
  font-size: 11.5px;
  cursor: pointer;
}
.style-switch button:hover {
  color: var(--text);
}
.style-switch button.active {
  background: var(--accent);
  color: #fff;
  font-weight: 600;
}

/* ---------- 列表 ---------- */

.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding-right: 2px;
}

.section + .section {
  margin-top: 10px;
}

.section-title {
  padding: 2px 8px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--text-muted);
}

.rows {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 4px;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border-radius: 14px;
  background: color-mix(in srgb, var(--surface-subtle) 75%, transparent);
  border: 1px solid color-mix(in srgb, var(--border) 40%, transparent);
  transition: background 0.15s ease, border-color 0.15s ease;
}
.row:hover,
.row.confirming {
  background: color-mix(in srgb, var(--accent) 9%, var(--surface-subtle));
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}

/* 单条视图：一行一条，点 + 名称 + 网址同行，无卡片底、更紧凑 */
.row.compact {
  padding: 5px 10px;
  border-radius: 8px;
  background: transparent;
  border-color: transparent;
}
.row.compact:hover,
.row.compact.confirming {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  border-color: transparent;
}
.mini-dot {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.row.compact .meta {
  flex-direction: row;
  align-items: baseline;
  gap: 10px;
}
.row.compact .name {
  flex: none;
  max-width: 45%;
  font-size: 12.5px;
}
.row.compact .url {
  flex: 1;
  min-width: 0;
}

/* 表格视图 */
.bm-table {
  margin-top: 4px;
  border: 1px solid color-mix(in srgb, var(--border) 45%, transparent);
  border-radius: 10px;
  overflow: hidden;
}
.thead,
.tr {
  display: grid;
  grid-template-columns: minmax(120px, 1.1fr) minmax(160px, 1.6fr) minmax(90px, 1fr) auto;
  align-items: center;
  gap: 10px;
  padding: 7px 12px;
}
.thead {
  background: var(--surface-container);
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
}
.th-ops {
  justify-self: end;
}
.tr {
  border-top: 1px solid color-mix(in srgb, var(--border) 30%, transparent);
}
.tr:hover {
  background: color-mix(in srgb, var(--accent) 7%, transparent);
}
.td-name {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
}
.td-name .name-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.td-url {
  font-size: 12px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.td-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.mini-tag {
  padding: 1px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
  font-size: 10.5px;
  white-space: nowrap;
}
.td-ops {
  justify-self: end;
}
/* 表格内操作按钮常显 */
.actions.inline {
  opacity: 1;
}
.tr.confirming .actions {
  opacity: 1;
}

.avatar {
  flex: none;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 15px;
  font-weight: 600;
}

.meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.name {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
}
.name-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pin-badge {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
}
.preview-badge {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--accent) 25%, transparent);
  color: var(--accent);
  border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
}
.url {
  font-size: 12px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 悬停才显示的操作区（确认删除态保持可见） */
.actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  transition: opacity 0.15s ease;
}
.row:hover .actions,
.row:focus-within .actions,
.row.confirming .actions {
  opacity: 1;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: 999px;
  background: transparent;
  color: var(--text-muted);
}
.icon-btn:hover {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
}
.icon-btn.active {
  color: var(--accent);
}
.icon-btn.danger:hover {
  background: color-mix(in srgb, var(--danger) 16%, transparent);
  color: var(--danger);
}

.confirm-btn {
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--danger);
  border-radius: 999px;
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
  font-size: 11px;
  white-space: nowrap;
}
.confirm-btn:hover {
  background: color-mix(in srgb, var(--danger) 32%, transparent);
}

/* ---------- 空状态 ---------- */

.empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 40px 16px;
  text-align: center;
}
.empty-icon {
  color: var(--text-muted);
  opacity: 0.55;
  margin-bottom: 4px;
}
.empty-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
}
.empty-sub {
  font-size: 12px;
  color: var(--text-muted);
  max-width: 260px;
}

.no-match {
  padding: 28px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: 12.5px;
}

/* ---------- 新增 / 编辑对话框 ---------- */

.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  width: min(440px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow: auto;
  border-radius: 20px;
  border: 1px solid var(--border);
  background: var(--surface);
  box-shadow: var(--shadow);
  padding: 20px;
}
.dialog h2 {
  margin: 0 0 16px;
  font-size: 15px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-bottom: 12px;
  font-size: 12px;
  color: var(--text-muted);
}
.field input {
  padding: 8px 12px;
  border-radius: var(--radius, 12px);
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  outline: none;
  transition: border-color 0.15s ease;
}
.field input:focus {
  border-color: var(--accent);
}
.field input.invalid {
  border-color: var(--danger);
}
.field-err {
  color: var(--danger);
  font-size: 11.5px;
}

.switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
  font-size: 12.5px;
  color: var(--text);
}

.switch {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  background: var(--surface-container);
  padding: 0;
  transition: background 0.15s ease, border-color 0.15s ease;
}
.switch .thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--border-strong);
  transition: left 0.15s ease, background 0.15s ease;
}
.switch.on {
  background: var(--accent);
  border-color: var(--accent);
}
.switch.on .thumb {
  left: 18px;
  background: #fff;
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 16px;
  border-radius: 999px;
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
  font-weight: 600;
}
.btn.primary:hover {
  filter: brightness(1.1);
}
</style>
