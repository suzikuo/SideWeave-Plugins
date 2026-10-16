<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** SSH 会话管理：按标签分组卡片、终端连接、TCP 测速、复制命令、增删改（M3 令牌） */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import { errorMessage } from '../../../ui-sdk/runtime'
import {
  buildSshCommand,
  groupOf,
  parseTags,
  sshDelete,
  sshDeleteKey,
  sshImportKey,
  sshList,
  sshListKeys,
  sshOpenTerminal,
  sshResolveKey,
  sshSave,
  sshTest
} from './api'
import type { SshKeyInfo, SshProfile, SshProfileInput } from './api'

const profiles = ref<SshProfile[]>([])
const search = ref('')
const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

/** 测速状态：id → 按钮 DTO（结果常驻到下次点击） */
type TestState = { kind: 'loading' | 'ok' | 'err'; text: string; title?: string }
const tests = ref<Record<string, TestState>>({})

/** 二次确认删除中的会话 id（3 秒未确认自动复位） */
const confirmId = ref('')
let confirmTimer: number | undefined

const showDialog = ref(false)
const editingId = ref<string | null>(null)
const form = ref(defaultForm())
const showErrors = ref(false)

// ---------- 托管密钥（导入即备份，历史可追溯） ----------

const showKeyMenu = ref(false)
const keys = ref<SshKeyInfo[]>([])
const importingKey = ref(false)
/** 二次确认删除中的密钥名（3 秒未确认自动复位） */
const confirmKeyName = ref('')
let confirmKeyTimer: number | undefined

async function importKey() {
  if (importingKey.value) return
  importingKey.value = true
  try {
    const info = await sshImportKey()
    if (!info) return
    form.value.key_path = info.name
    keys.value = [info, ...keys.value.filter((k) => k.name !== info.name)]
    showToast(`密钥已备份到应用目录：${info.name}`, 'success')
  } catch (e) {
    showToast(errorMessage(e), 'error')
  } finally {
    importingKey.value = false
  }
}

async function toggleKeyMenu() {
  showKeyMenu.value = !showKeyMenu.value
  if (!showKeyMenu.value) return
  try {
    keys.value = await sshListKeys()
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

function pickKey(k: SshKeyInfo) {
  form.value.key_path = k.name
  showKeyMenu.value = false
}

async function removeKey(k: SshKeyInfo) {
  if (confirmKeyName.value !== k.name) {
    confirmKeyName.value = k.name
    window.clearTimeout(confirmKeyTimer)
    confirmKeyTimer = window.setTimeout(() => {
      confirmKeyName.value = ''
    }, 3000)
    return
  }
  window.clearTimeout(confirmKeyTimer)
  confirmKeyName.value = ''
  try {
    await sshDeleteKey(k.name)
    keys.value = keys.value.filter((x) => x.name !== k.name)
    if (form.value.key_path === k.name) form.value.key_path = ''
    showToast('已删除托管密钥', 'success')
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

function formatKeyTime(iso: string): string {
  return iso ? iso.replace('T', ' ').slice(0, 16) : ''
}

function keySizeText(size: number): string {
  return size < 1024 ? `${size} B` : `${Math.round(size / 1024)} KB`
}

function defaultForm() {
  return {
    id: '',
    name: '',
    host: '',
    port: 22,
    user: '',
    auth: 'password' as SshProfileInput['auth'],
    key_path: '',
    tagsText: ''
  }
}

const nameError = computed(() =>
  showErrors.value && !form.value.name.trim() ? '名称不能为空' : ''
)
const hostError = computed(() =>
  showErrors.value && !form.value.host.trim() ? '主机不能为空' : ''
)
const userError = computed(() =>
  showErrors.value && !form.value.user.trim() ? '用户不能为空' : ''
)
const portError = computed(() => {
  if (!showErrors.value) return ''
  const port = form.value.port
  return !Number.isInteger(port) || port < 1 || port > 65535 ? '端口需为 1-65535 的整数' : ''
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
    profiles.value = await sshList()
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

onMounted(refresh)

/** 搜索：名称 / 主机 / 用户 */
const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return profiles.value
  return profiles.value.filter((p) =>
    [p.name, p.host, p.user].some((f) => f.toLowerCase().includes(q))
  )
})

/** 按 tags 第一项分组（无标签 → 默认组），保持首次出现顺序 */
const groups = computed(() => {
  const list: { tag: string; items: SshProfile[] }[] = []
  const index = new Map<string, number>()
  for (const p of filtered.value) {
    const tag = groupOf(p)
    let i = index.get(tag)
    if (i === undefined) {
      i = list.length
      index.set(tag, i)
      list.push({ tag, items: [] })
    }
    list[i].items.push(p)
  }
  return list
})

const isEmpty = computed(() => profiles.value.length === 0)

function initialOf(p: SshProfile): string {
  return p.name.trim().charAt(0).toUpperCase()
}

function testClass(id: string): string {
  const t = tests.value[id]
  return t?.kind === 'ok' ? 'ok' : t?.kind === 'err' ? 'err' : ''
}

// ---------- 会话操作 ----------

async function connect(p: SshProfile) {
  try {
    await sshOpenTerminal(p.id)
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

async function runTest(p: SshProfile) {
  tests.value = { ...tests.value, [p.id]: { kind: 'loading', text: '测试中…' } }
  try {
    const ms = await sshTest(p.id)
    tests.value = { ...tests.value, [p.id]: { kind: 'ok', text: `${ms} ms` } }
  } catch (e) {
    const msg = errorMessage(e)
    // 取末段短文案进按钮（如「超时」），完整错误挂 title
    const short = msg.includes('：') ? (msg.split('：').pop() ?? '失败') : '失败'
    tests.value = { ...tests.value, [p.id]: { kind: 'err', text: short, title: msg } }
  }
}

async function copyCommand(p: SshProfile) {
  try {
    // 密钥解析为实际路径：托管相对名 → 应用数据目录里的副本（与终端打开语义一致）
    let keyPath = p.key_path.trim()
    if (p.auth === 'key' && keyPath) {
      keyPath = (await sshResolveKey(keyPath)) ?? keyPath
    }
    await navigator.clipboard.writeText(buildSshCommand({ ...p, key_path: keyPath }))
    showToast('已复制', 'success')
  } catch {
    showToast('复制失败', 'error')
  }
}

async function remove(p: SshProfile) {
  if (confirmId.value !== p.id) {
    confirmId.value = p.id
    window.clearTimeout(confirmTimer)
    confirmTimer = window.setTimeout(() => {
      confirmId.value = ''
    }, 3000)
    return
  }
  window.clearTimeout(confirmTimer)
  confirmId.value = ''
  try {
    await sshDelete(p.id)
    showToast(`已删除 ${p.name}`, 'success')
    await refresh()
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

// ---------- 新增 / 编辑 ----------

function openAdd() {
  editingId.value = null
  form.value = defaultForm()
  showErrors.value = false
  showDialog.value = true
}

function openEdit(p: SshProfile) {
  editingId.value = p.id
  form.value = {
    id: p.id,
    name: p.name,
    host: p.host,
    port: p.port,
    user: p.user,
    auth: p.auth === 'key' ? 'key' : 'password',
    key_path: p.key_path,
    tagsText: p.tags.join(', ')
  }
  showErrors.value = false
  showDialog.value = true
}

async function save() {
  const f = form.value
  showErrors.value = true
  if (nameError.value || hostError.value || userError.value || portError.value) return
  const input: SshProfileInput = {
    id: editingId.value ?? '',
    name: f.name.trim(),
    host: f.host.trim(),
    port: f.port,
    user: f.user.trim(),
    auth: f.auth,
    key_path: f.key_path.trim(),
    tags: parseTags(f.tagsText)
  }
  const editing = !!editingId.value
  try {
    await sshSave(input)
    showDialog.value = false
    showToast(editing ? '已更新' : '已添加', 'success')
    await refresh()
  } catch (e) {
    showToast(errorMessage(e), 'error')
  }
}

onBeforeUnmount(() => {
  window.clearTimeout(confirmTimer)
  window.clearTimeout(confirmKeyTimer)
})

watch(showDialog, (open) => {
  if (!open) {
    showKeyMenu.value = false
    confirmKeyName.value = ''
  }
})
</script>

<template>
  <div class="ssh">
    <div class="toolbar">
      <Button variant="default" size="sm" class="btn primary" @click="openAdd">
        <Icon name="plus" :size="14" /> 新增会话
      </Button>
      <div class="search">
        <Icon name="search" :size="14" />
        <input v-model="search" type="text" placeholder="搜索名称、主机或用户" />
      </div>
    </div>

    <div class="list">
      <section v-for="g in groups" :key="g.tag" class="group">
        <div class="group-head">
          <span>{{ g.tag }}</span>
          <span class="count">{{ g.items.length }}</span>
        </div>

        <div
          v-for="p in g.items"
          :key="p.id"
          class="card"
          :title="`双击连接 ${p.name}`"
          @dblclick="connect(p)"
        >
          <div class="avatar">
            <span v-if="initialOf(p)" class="initial">{{ initialOf(p) }}</span>
            <Icon v-else name="terminal" :size="16" />
          </div>

          <div class="info">
            <div class="line1">
              <span class="name">{{ p.name }}</span>
              <span class="chip auth">{{ p.auth === 'key' ? '密钥' : '密码' }}</span>
              <span v-for="t in p.tags" :key="t" class="chip tag">{{ t }}</span>
            </div>
            <div class="meta">{{ p.user }}@{{ p.host }}:{{ p.port }}</div>
          </div>

          <div class="actions">
            <Button variant="default" size="sm" class="btn primary connect" @click="connect(p)">连接</Button>
            <Button variant="outline" size="sm"
              class="btn test"
              :class="testClass(p.id)"
              :title="tests[p.id]?.title ?? 'TCP 连通性测试'"
              :disabled="tests[p.id]?.kind === 'loading'"
              @click="runTest(p)"
            >
              {{ tests[p.id]?.text ?? '测试' }}
            </Button>
            <Button variant="outline" size="sm" class="icon-btn" title="复制命令" @click="copyCommand(p)">
              <!-- 自绘 copy 图标（Icon.vue 无该图标） -->
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
                <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
              </svg>
            </Button>
            <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click="openEdit(p)">
              <Icon name="pencil" :size="14" />
            </Button>
            <Button variant="destructive" size="sm"
              class="icon-btn danger"
              :class="{ confirming: confirmId === p.id }"
              :title="confirmId === p.id ? '再次点击确认删除' : '删除'"
              @click="remove(p)"
            >
              <Icon name="trash" :size="14" />
            </Button>
          </div>
        </div>
      </section>

      <div v-if="groups.length === 0" class="empty">
        <Icon name="terminal" :size="28" />
        <p>{{ isEmpty ? '还没有 SSH 会话，点击右上角「新增会话」添加' : '没有匹配的会话' }}</p>
      </div>
    </div>

    <Teleport to="body">
      <div v-if="showDialog" class="mask" @click.self="showDialog = false">
        <div class="dialog">
          <h2>{{ editingId ? '编辑会话' : '新增会话' }}</h2>

          <label class="field">
            <span>名称 *</span>
            <input
              v-model="form.name"
              type="text"
              placeholder="例如：生产环境 Web"
              :class="{ invalid: !!nameError }"
            />
            <span v-if="nameError" class="field-err">{{ nameError }}</span>
          </label>

          <div class="grid2">
            <label class="field">
              <span>主机 *</span>
              <input
                v-model="form.host"
                type="text"
                placeholder="example.com 或 IP"
                :class="{ invalid: !!hostError }"
              />
              <span v-if="hostError" class="field-err">{{ hostError }}</span>
            </label>
            <label class="field">
              <span>端口</span>
              <input
                v-model.number="form.port"
                type="number"
                min="1"
                max="65535"
                :class="{ invalid: !!portError }"
              />
              <span v-if="portError" class="field-err">{{ portError }}</span>
            </label>
          </div>

          <label class="field">
            <span>用户 *</span>
            <input
              v-model="form.user"
              type="text"
              placeholder="root"
              :class="{ invalid: !!userError }"
            />
            <span v-if="userError" class="field-err">{{ userError }}</span>
          </label>

          <div class="field">
            <span>认证方式</span>
            <div class="segmented">
              <button
                type="button"
                :class="{ active: form.auth === 'password' }"
                @click="form.auth = 'password'"
              >
                密码
              </button>
              <button
                type="button"
                :class="{ active: form.auth === 'key' }"
                @click="form.auth = 'key'"
              >
                密钥
              </button>
            </div>
          </div>

          <label v-if="form.auth === 'key'" class="field">
            <span>密钥（导入后备份到应用数据目录，源文件丢失不影响连接）</span>
            <div class="key-row">
              <input
                v-model="form.key_path"
                type="text"
                placeholder="id_ed25519（托管名）或 C:\Users\you\.ssh\id_ed25519"
              />
              <Button variant="outline" size="sm" type="button" class="btn" :disabled="importingKey" @click="importKey">
                {{ importingKey ? '导入中…' : '导入' }}
              </Button>
              <div class="keys-menu-wrap">
                <Button variant="outline" size="sm" type="button" class="btn" @click="toggleKeyMenu">历史</Button>
                <div v-if="showKeyMenu" class="keys-menu">
                  <p v-if="keys.length === 0" class="keys-empty">还没有托管密钥，先点"导入"</p>
                  <div v-for="k in keys" :key="k.name" class="keys-item" @click="pickKey(k)">
                    <span class="keys-name" :title="k.path">{{ k.name }}</span>
                    <span class="keys-meta">
                      {{ formatKeyTime(k.imported_at) }} · {{ keySizeText(k.size) }}
                    </span>
                    <button
                      type="button"
                      class="keys-del"
                      :class="{ confirming: confirmKeyName === k.name }"
                      title="删除托管副本（不影响源文件）"
                      @click.stop="removeKey(k)"
                    >
                      {{ confirmKeyName === k.name ? '确认' : '删除' }}
                    </button>
                  </div>
                </div>
              </div>
            </div>
            <span class="hint">留空则用默认 ~/.ssh；可填托管文件名（推荐）或绝对路径</span>
          </label>

          <label class="field">
            <span>标签</span>
            <input v-model="form.tagsText" type="text" placeholder="生产, 日常（逗号分隔）" />
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
.ssh {
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

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
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
.btn:disabled {
  cursor: default;
  opacity: 0.75;
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

/* ---------- 分组列表 ---------- */

.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding-bottom: 2px;
}

.group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.group-head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-muted);
  padding: 0 2px;
}
.group-head .count {
  min-width: 18px;
  padding: 0 5px;
  border-radius: 999px;
  text-align: center;
  font-size: 10.5px;
  line-height: 16px;
  background: var(--surface-container);
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
}

.card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: var(--radius, 12px);
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  background: color-mix(in srgb, var(--surface-subtle) 70%, transparent);
  transition: background 0.15s ease, border-color 0.15s ease;
}
.card:hover {
  background: color-mix(in srgb, var(--primary) 7%, var(--surface-subtle));
  border-color: color-mix(in srgb, var(--primary) 40%, var(--border));
}

.avatar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--primary) 26%, #101416);
  color: #dff5f7;
  font-size: 14px;
  font-weight: 600;
  user-select: none;
}

.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.line1 {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.name {
  font-weight: 600;
  font-size: 13px;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip {
  flex-shrink: 0;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 10.5px;
  line-height: 16px;
  white-space: nowrap;
}
.chip.auth {
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  border: 1px solid color-mix(in srgb, var(--primary) 36%, transparent);
}
.chip.tag {
  color: var(--text-muted);
  background: var(--surface-container);
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
}

.meta {
  font-family: 'Cascadia Mono', 'Consolas', ui-monospace, monospace;
  font-size: 11.5px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---------- 卡片操作 ---------- */

.actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}

.btn.connect {
  padding: 6px 14px;
}

.btn.test {
  min-width: 66px;
  padding: 6px 10px;
  font-variant-numeric: tabular-nums;
}
.btn.test.ok {
  color: var(--success);
  border-color: color-mix(in srgb, var(--success) 55%, transparent);
}
.btn.test.err {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 55%, transparent);
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
.icon-btn.confirming {
  color: #fff;
  background: var(--danger);
}
.icon-btn.confirming:hover {
  color: #fff;
  filter: brightness(1.08);
}

/* ---------- 空状态 ---------- */

.empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 32px 0;
  color: var(--text-muted);
  font-size: 12.5px;
  text-align: center;
}
.empty p {
  margin: 0;
  max-width: 260px;
  line-height: 1.6;
}

/* ---------- 对话框 ---------- */

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
  width: min(460px, calc(100vw - 48px));
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
.field input {
  padding: 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  outline: none;
}
.field input:focus {
  border-color: var(--primary);
}
.field input.invalid {
  border-color: var(--danger);
}
.field-err {
  color: var(--danger);
  font-size: 11.5px;
}
.hint {
  color: var(--text-muted);
  font-size: 11.5px;
  opacity: 0.85;
}

.grid2 {
  display: grid;
  grid-template-columns: 1fr 110px;
  gap: 10px;
}

.segmented {
  display: inline-flex;
  padding: 3px;
  gap: 3px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  width: fit-content;
}
.segmented button {
  padding: 5px 16px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
}
.segmented button.active {
  background: var(--primary);
  color: var(--primary-foreground);
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}

/* ---------- 密钥托管 ---------- */
.key-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.key-row input {
  flex: 1;
  min-width: 0;
}
.keys-menu-wrap {
  position: relative;
  flex: none;
}
.keys-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 30;
  width: 280px;
  max-height: 260px;
  overflow: auto;
  padding: 4px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface);
  box-shadow: var(--shadow);
}
.keys-empty {
  margin: 0;
  padding: 10px;
  text-align: center;
  font-size: 12px;
  color: var(--text-muted);
}
.keys-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  border-radius: 7px;
  cursor: pointer;
  font-size: 12px;
}
.keys-item:hover {
  background: color-mix(in srgb, var(--primary) 14%, transparent);
}
.keys-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
}
.keys-meta {
  flex: none;
  font-size: 10.5px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
.keys-del {
  flex: none;
  padding: 2px 7px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  font-size: 10.5px;
  cursor: pointer;
}
.keys-del:hover {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
}
.keys-del.confirming {
  background: var(--danger);
  color: #fff;
}
</style>
