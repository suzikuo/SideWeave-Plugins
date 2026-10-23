<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
import { computed, onMounted, ref } from 'vue'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import {
  alAdd,
  alChromePreset,
  alIcon,
  alLaunch,
  alList,
  alRemove,
  alTogglePreview,
  alUpdate,
  pickExecutable
} from './launcher-api'
import type { AppEntry, AppEntryInput } from './launcher-api'

const apps = ref<AppEntry[]>([])
const search = ref('')
const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

/** exe_path → PNG data URL（图标列展示） */
const icons = ref<Record<string, string>>({})
/** 按 exe_path 去重的在途图标请求 */
const iconInflight = new Map<string, Promise<string | null>>()

const showDialog = ref(false)
const editingId = ref<string | null>(null)
const form = ref(defaultForm())
const showErrors = ref(false)

const nameError = computed(() =>
  showErrors.value && !form.value.name.trim() ? 'Name is required' : ''
)
const pathError = computed(() =>
  showErrors.value && !form.value.exe_path.trim() ? 'Exe path is required' : ''
)

function defaultForm() {
  return {
    app_type: 'application' as 'application' | 'chrome',
    name: '',
    exe_path: '',
    arguments: '',
    use_ahk: false,
    ahk_path: '',
    window_x: 200,
    window_y: 200,
    window_width: 160,
    window_height: 400,
    always_on_top: true,
    in_preview: false
  }
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
    apps.value = await alList()
    loadIcons()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 为列表条目拉取图标；相同 exe_path 只发一次请求 */
function loadIcons() {
  const paths = [...new Set(apps.value.map((a) => a.exe_path).filter(Boolean))]
  for (const p of paths) void requestIcon(p)
}

function requestIcon(path: string): Promise<string | null> {
  if (!path) return Promise.resolve(null)
  const cached = icons.value[path]
  if (cached) return Promise.resolve(cached)
  const pending = iconInflight.get(path)
  if (pending) return pending
  const task = alIcon(path)
    .then((b64) => {
      iconInflight.delete(path)
      if (!b64) return null
      const url = b64.startsWith('data:') ? b64 : `data:image/png;base64,${b64}`
      icons.value = { ...icons.value, [path]: url }
      return url
    })
    .catch(() => {
      iconInflight.delete(path)
      return null
    })
  iconInflight.set(path, task)
  return task
}

onMounted(refresh)

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return apps.value
  return apps.value.filter((a) =>
    [a.name, a.exe_path, a.arguments, a.app_type].some((f) =>
      f.toLowerCase().includes(q)
    )
  )
})

function targetOf(app: AppEntry): string {
  if (app.app_type === 'chrome') {
    const m = /--app=(\S+)/.exec(app.arguments)
    if (m) return m[1]
  }
  return app.exe_path
}

async function launch(app: AppEntry) {
  try {
    await alLaunch(app.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function openAdd() {
  editingId.value = null
  form.value = defaultForm()
  showErrors.value = false
  showDialog.value = true
}

function openAddChrome() {
  editingId.value = null
  form.value = { ...defaultForm(), app_type: 'chrome' }
  showErrors.value = false
  showDialog.value = true
}

async function applyChromePreset() {
  try {
    const preset = await alChromePreset()
    if (!preset) {
      showToast('未检测到 Chrome', 'error')
      return
    }
    form.value = { ...form.value, ...preset, app_type: 'chrome' }
    showToast('已填入 Chrome 配置', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function openEdit(app: AppEntry) {
  editingId.value = app.id
  form.value = { ...app, in_preview: Boolean(app.in_preview) }
  showErrors.value = false
  showDialog.value = true
}

function formInput(): AppEntryInput {
  const current = form.value
  return {
    app_type: current.app_type,
    name: String(current.name),
    exe_path: String(current.exe_path),
    arguments: String(current.arguments),
    use_ahk: Boolean(current.use_ahk),
    ahk_path: String(current.ahk_path),
    window_x: Number(current.window_x),
    window_y: Number(current.window_y),
    window_width: Number(current.window_width),
    window_height: Number(current.window_height),
    always_on_top: Boolean(current.always_on_top),
    in_preview: Boolean(current.in_preview)
  }
}

async function save() {
  const f = form.value
  showErrors.value = true
  if (!f.name.trim() || !f.exe_path.trim()) {
    showToast('Name 与 Exe Path 不能为空', 'error')
    return
  }
  const input = formInput()
  try {
    if (editingId.value) {
      await alUpdate(editingId.value, input)
      showToast('已更新', 'success')
    } else {
      await alAdd(input)
      showToast('已添加', 'success')
    }
    showDialog.value = false
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function remove(app: AppEntry) {
  try {
    await alRemove(app.id)
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function browse() {
  const path = await pickExecutable()
  if (path) {
    form.value.exe_path = path
    if (!form.value.name.trim()) {
      const base = path.split(/[\\/]/).pop() ?? ''
      form.value.name = base.replace(/\.exe$/i, '').replace(/^\w/, (c) => c.toUpperCase())
    }
  }
}

async function togglePreview(app: AppEntry) {
  try {
    const res = await alTogglePreview(app.id)
    app.in_preview = res.in_preview
    showToast(
      res.in_preview ? `已将「${app.name}」加入悬浮预览` : `已从悬浮预览移除「${app.name}」`,
      'success'
    )
  } catch (e) {
    showToast(String(e), 'error')
  }
}
</script>

<template>
  <div class="launcher">
    <div class="toolbar">
      <div class="actions">
        <Button variant="default" size="sm" class="btn primary" @click="openAdd"><Icon name="plus" :size="14" /> Add App</Button>
        <Button variant="outline" size="sm" class="btn" @click="openAddChrome"><Icon name="globe" :size="14" /> Add Chrome</Button>
      </div>
      <div class="search">
        <Icon name="search" :size="14" />
        <input
          v-model="search"
          type="text"
          placeholder="Search name, path, or arguments"
        />
      </div>
    </div>

    <div class="table" role="table">
      <div class="tr head" role="row">
        <span class="c-icon">Icon</span>
        <span class="c-name">Name</span>
        <span class="c-type">Type</span>
        <span class="c-target">Target</span>
        <span class="c-args">Arguments</span>
        <span class="c-preview">悬浮展示</span>
        <span class="c-actions">Actions</span>
      </div>
      <div
        v-for="app in filtered"
        :key="app.id"
        class="tr"
        role="row"
        @dblclick="launch(app)"
      >
        <span class="c-icon">
          <img
            v-if="icons[app.exe_path]"
            :src="icons[app.exe_path]"
            alt=""
            draggable="false"
          />
          <span v-else class="icon-dot" />
        </span>
        <span class="c-name strong" :title="app.name">{{ app.name }}</span>
        <span class="c-type">{{ app.app_type }}</span>
        <span class="c-target" :title="targetOf(app)">{{ targetOf(app) }}</span>
        <span class="c-args" :title="app.arguments">{{ app.arguments }}</span>
        <span class="c-preview">
          <label class="switch" :title="app.in_preview ? '已加入悬浮展示，点击取消' : '点击在侧边栏悬浮卡片展示'">
            <input
              type="checkbox"
              :checked="app.in_preview"
              @click.stop
              @change="togglePreview(app)"
            />
            <span class="slider" />
          </label>
        </span>
        <span class="c-actions">
          <Button variant="outline" size="sm" class="icon-btn" title="启动" @click.stop="launch(app)">
            <Icon name="play" :size="14" />
          </Button>
          <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click.stop="openEdit(app)">
            <Icon name="pencil" :size="14" />
          </Button>
          <Button variant="destructive" size="sm" class="icon-btn danger" title="删除" @click.stop="remove(app)">
            <Icon name="trash" :size="14" />
          </Button>
        </span>
      </div>
      <div v-if="filtered.length === 0" class="empty-row">
        {{ apps.length === 0 ? 'No applications' : 'No matching applications' }}
      </div>
    </div>

    <Teleport to="body">
      <div v-if="showDialog" class="mask" @click.self="showDialog = false">
        <div class="dialog">
          <h2>{{ editingId ? 'Edit Application' : 'Add Application' }}</h2>

          <div class="field">
            <span>Type</span>
            <select v-model="form.app_type">
              <option value="application">application</option>
              <option value="chrome">chrome</option>
            </select>
            <Button variant="outline" size="sm" v-if="form.app_type === 'chrome'" class="btn" type="button" @click="applyChromePreset">
              Detect Chrome
            </Button>
          </div>

          <label class="field">
            <span>Name *</span>
            <input
              v-model="form.name"
              type="text"
              placeholder="显示名称"
              :class="{ invalid: !!nameError }"
            />
            <span v-if="nameError" class="field-err">{{ nameError }}</span>
          </label>

          <label class="field">
            <span>Exe Path *</span>
            <div class="row">
              <input
                v-model="form.exe_path"
                type="text"
                placeholder="C:\\path\\to\\app.exe"
                :class="{ invalid: !!pathError }"
              />
              <Button variant="outline" size="sm" class="btn" @click="browse"><Icon name="folder" :size="14" /> …</Button>
            </div>
            <span v-if="pathError" class="field-err">{{ pathError }}</span>
          </label>

          <label class="field">
            <span>Arguments</span>
            <textarea v-model="form.arguments" rows="3" placeholder="--flag value" />
          </label>

          <template v-if="form.app_type === 'chrome'">
            <label class="check">
              <input v-model="form.use_ahk" type="checkbox" />
              <span>Use AutoHotkey window adjustment</span>
            </label>

            <template v-if="form.use_ahk">
              <label class="field">
                <span>AHK Path（留空自动探测）</span>
                <input v-model="form.ahk_path" type="text" placeholder="AutoHotkey.exe" />
              </label>

              <div class="grid4">
                <label class="field"><span>X</span><input v-model.number="form.window_x" type="number" min="0" max="9999" /></label>
                <label class="field"><span>Y</span><input v-model.number="form.window_y" type="number" min="0" max="9999" /></label>
                <label class="field"><span>W</span><input v-model.number="form.window_width" type="number" min="1" /></label>
                <label class="field"><span>H</span><input v-model.number="form.window_height" type="number" min="1" /></label>
              </div>

              <label class="check">
                <input v-model="form.always_on_top" type="checkbox" />
                <span>Always on top</span>
              </label>
            </template>
          </template>

          <label class="check">
            <input v-model="form.in_preview" type="checkbox" />
            <span>在侧边栏悬浮卡片中展示 (Show in hover preview)</span>
          </label>

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
.launcher {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 12px;
}

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.actions {
  display: flex;
  gap: 8px;
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
  background: var(--primary);
  border-color: var(--primary);
  color: var(--primary-foreground);
}
.btn.primary:hover {
  filter: brightness(1.1);
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 200px;
  max-width: 360px;
  padding: 7px 12px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text-muted);
}
.search input {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text);
}

.table {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}

.tr {
  display: grid;
  grid-template-columns: 36px minmax(88px, 1.1fr) 76px minmax(0, 1.5fr) minmax(0, 1.2fr) 76px 96px;
  align-items: center;
  min-height: 44px;
  padding: 0 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 45%, transparent);
  font-size: 12.5px;
}
.tr:last-child {
  border-bottom: none;
}
.tr:not(.head):hover {
  background: color-mix(in srgb, var(--primary) 8%, transparent);
}

.tr.head {
  position: sticky;
  top: 0;
  min-height: 36px;
  background: var(--surface-container);
  color: var(--text-muted);
  font-weight: 600;
  z-index: 1;
}

.c-name,
.c-type,
.c-target,
.c-args {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding-right: 10px;
}
.strong {
  font-weight: 600;
}

.c-icon {
  display: flex;
  align-items: center;
  justify-content: center;
}
.c-icon img {
  width: 20px;
  height: 20px;
  object-fit: contain;
  border-radius: 5px;
}
.c-icon .icon-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--border-strong);
  opacity: 0.6;
}

.c-preview {
  display: flex;
  align-items: center;
  justify-content: center;
}

.switch {
  position: relative;
  display: inline-block;
  width: 32px;
  height: 18px;
  cursor: pointer;
}
.switch input {
  opacity: 0;
  width: 0;
  height: 0;
}
.slider {
  position: absolute;
  cursor: pointer;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background-color: color-mix(in srgb, var(--border-strong) 80%, transparent);
  transition: 0.2s;
  border-radius: 18px;
}
.slider:before {
  position: absolute;
  content: "";
  height: 14px;
  width: 14px;
  left: 2px;
  bottom: 2px;
  background-color: white;
  transition: 0.2s;
  border-radius: 50%;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
}
.switch input:checked + .slider {
  background-color: var(--primary);
}
.switch input:checked + .slider:before {
  transform: translateX(14px);
  background-color: var(--primary-foreground);
}

.c-actions {
  display: flex;
  justify-content: flex-end;
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
  color: var(--primary);
}
.icon-btn.danger:hover {
  color: var(--danger);
}

.empty-row {
  padding: 32px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: 12.5px;
}

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
  width: min(480px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow: auto;
  border-radius: 16px;
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
.field input,
.field select,
.field textarea {
  padding: 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  outline: none;
  resize: vertical;
}
.field input:focus,
.field select:focus,
.field textarea:focus {
  border-color: var(--primary);
}
.field input.invalid {
  border-color: var(--danger);
}
.field-err {
  color: var(--danger);
  font-size: 11.5px;
}

.row {
  display: flex;
  gap: 8px;
}
.row input {
  flex: 1;
}

.grid4 {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}

.check {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  font-size: 12.5px;
  color: var(--text);
}
.check input {
  accent-color: var(--primary);
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}
</style>
