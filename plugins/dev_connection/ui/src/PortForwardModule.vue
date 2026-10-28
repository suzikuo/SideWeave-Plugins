<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { onPluginLifecycle } from '../../../ui-sdk/runtime'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import {
  gwDelete,
  gwListeners,
  gwList,
  gwConnectionLogs,
  gwClearConnectionLogs,
  gwSave,
  gwStopListener,
  gwSetRunning,
  type GwRuleInput,
  type GwRuleView,
  type ConnectionLogView,
  type ListenerInfo
} from './port-forward-api'

type ActiveView = 'rules' | 'connections' | 'listeners'

const activeView = ref<ActiveView>('rules')
const rules = ref<GwRuleView[]>([])
const listeners = ref<ListenerInfo[]>([])
const connectionLogs = ref<ConnectionLogView[]>([])
const listenHost = '0.0.0.0'
const portQuery = ref<number | ''>('')
const queriedPort = ref<number | ''>('')
const listenerRefreshing = ref(false)
const terminateTarget = ref<ListenerInfo | null>(null)
const terminatingPid = ref<number | null>(null)

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

const showDialog = ref(false)
const editingId = ref<string | null>(null)
const showErrors = ref(false)

/** 端口输入清空时 v-model.number 会得到 ''，类型上如实声明 */
interface RuleForm {
  name: string
  listen_port: number | ''
  target_host: string
  target_port: number | ''
  enabled: boolean
  in_preview: boolean
}

function defaultForm(): RuleForm {
  return { name: '', listen_port: '', target_host: '', target_port: '', enabled: false, in_preview: false }
}
const form = ref<RuleForm>(defaultForm())

/** 删除二次确认：记录目标规则 id，3 秒未确认自动恢复 */
const confirmId = ref<string | null>(null)
let confirmTimer = 0

let pollTimer = 0
let pluginActive = false
let unlistenLifecycle: (() => void) | null = null
const refreshing = ref(false)

const runningCount = computed(() => rules.value.filter((r) => r.running).length)
const filterDescription = computed(() =>
  queriedPort.value === '' ? '全部监听端口' : `端口 ${queriedPort.value}`
)

// ---------- 校验 ----------

function isValidPort(v: unknown): boolean {
  const n = Number(v)
  return Number.isInteger(n) && n >= 1 && n <= 65535
}

const nameError = computed(() => (showErrors.value && !form.value.name.trim() ? '请输入名称' : ''))
const listenError = computed(() =>
  showErrors.value && !isValidPort(form.value.listen_port) ? '监听端口需为 1-65535 的整数' : ''
)
const hostError = computed(() =>
  showErrors.value && !form.value.target_host.trim() ? '请输入目标主机' : ''
)
const portError = computed(() =>
  showErrors.value && !isValidPort(form.value.target_port) ? '目标端口需为 1-65535 的整数' : ''
)

// ---------- Toast（与 AppLauncherView 同款写法） ----------

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

// ---------- 数据 ----------

async function refresh(silent = false) {
  if (refreshing.value) return
  refreshing.value = true
  try {
    rules.value = await gwList()
  } catch (e) {
    if (!silent) showToast(String(e), 'error')
  } finally {
    refreshing.value = false
  }
}

function refreshWhenVisible() {
  if (!pluginActive || document.hidden || showDialog.value || terminateTarget.value) return
  if (activeView.value === 'listeners') void refreshListeners(true)
  else if (activeView.value === 'connections') void refreshConnectionLogs(true)
  else void refresh(true)
}

function setPluginActive(active: boolean) {
  pluginActive = active
  window.clearInterval(pollTimer)
  pollTimer = 0
  if (!active) return
  if (activeView.value === 'listeners') void refreshListeners()
  else if (activeView.value === 'connections') void refreshConnectionLogs()
  else void refresh()
  pollTimer = window.setInterval(refreshWhenVisible, 5000)
}

onMounted(() => {
  document.addEventListener('visibilitychange', refreshWhenVisible)
  unlistenLifecycle = onPluginLifecycle(setPluginActive)
})

onUnmounted(() => {
  window.clearInterval(pollTimer)
  window.clearTimeout(confirmTimer)
  document.removeEventListener('visibilitychange', refreshWhenVisible)
  unlistenLifecycle?.()
})

// ---------- 操作 ----------

async function toggleRun(rule: GwRuleView) {
  try {
    const updated = await gwSetRunning(rule.id, !rule.running)
    const i = rules.value.findIndex((r) => r.id === rule.id)
    if (i >= 0) rules.value[i] = updated
    if (!rule.running) showToast(`「${rule.name}」已开始转发`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
    await refresh(true)
  }
}

async function refreshListeners(silent = false) {
  if (listenerRefreshing.value) return
  const port = portQuery.value === '' ? undefined : Number(portQuery.value)
  if (port !== undefined && (!Number.isInteger(port) || port < 1 || port > 65535)) {
    if (!silent) showToast('请输入 1-65535 之间的端口号', 'error')
    return
  }
  listenerRefreshing.value = true
  try {
    listeners.value = await gwListeners(port)
    queriedPort.value = port ?? ''
  } catch (e) {
    if (!silent) showToast(String(e), 'error')
  } finally {
    listenerRefreshing.value = false
  }
}

async function refreshConnectionLogs(silent = false) {
  try {
    connectionLogs.value = await gwConnectionLogs()
  } catch (e) {
    if (!silent) showToast(String(e), 'error')
  }
}

async function clearConnectionLogs() {
  try {
    await gwClearConnectionLogs()
    connectionLogs.value = []
    showToast('已清空临时请求记录', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function formatLogTime(timestamp: number): string {
  return new Date(timestamp).toLocaleString('zh-CN', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit'
  })
}

function formatDuration(milliseconds: number): string {
  if (milliseconds < 1000) return `${milliseconds} ms`
  return `${(milliseconds / 1000).toFixed(1)} s`
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function selectView(view: ActiveView) {
  activeView.value = view
  if (!pluginActive) return
  if (view === 'listeners') void refreshListeners()
  else if (view === 'connections') void refreshConnectionLogs()
  else void refresh(true)
}

function openTerminate(listener: ListenerInfo) {
  if (!listener.can_terminate || !listener.path) return
  terminateTarget.value = listener
}

async function terminateProcess() {
  const target = terminateTarget.value
  if (!target || !target.can_terminate || terminatingPid.value !== null) return
  terminatingPid.value = target.pid
  try {
    await gwStopListener(target)
    terminateTarget.value = null
    showToast(`已结束 ${target.name}（PID ${target.pid}）`, 'success')
    await refreshListeners()
  } catch (e) {
    terminateTarget.value = null
    showToast(String(e), 'error')
    await refreshListeners(true)
  } finally {
    terminatingPid.value = null
  }
}

function askDelete(rule: GwRuleView) {
  window.clearTimeout(confirmTimer)
  confirmId.value = rule.id
  confirmTimer = window.setTimeout(() => {
    confirmId.value = null
  }, 3000)
}

function resetConfirm() {
  window.clearTimeout(confirmTimer)
  confirmId.value = null
}

async function doDelete(rule: GwRuleView) {
  resetConfirm()
  try {
    await gwDelete(rule.id)
    showToast(`已删除「${rule.name}」`, 'success')
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 新建 / 编辑 ----------

function openAdd() {
  editingId.value = null
  form.value = defaultForm()
  showErrors.value = false
  showDialog.value = true
}

function openEdit(rule: GwRuleView) {
  editingId.value = rule.id
  form.value = {
    name: rule.name,
    listen_port: rule.listen_port,
    target_host: rule.target_host,
    target_port: rule.target_port,
    enabled: rule.enabled,
    in_preview: rule.in_preview ?? false
  }
  showErrors.value = false
  showDialog.value = true
}

function closeDialog() {
  showDialog.value = false
}

async function save() {
  showErrors.value = true
  const f = form.value
  if (!f.name.trim() || !isValidPort(f.listen_port) || !f.target_host.trim() || !isValidPort(f.target_port)) {
    showToast('请检查表单填写', 'error')
    return
  }
  const input: GwRuleInput = {
    id: editingId.value ?? '',
    name: f.name.trim(),
    listen_port: Number(f.listen_port),
    target_host: f.target_host.trim(),
    target_port: Number(f.target_port),
    enabled: f.enabled,
    in_preview: f.in_preview
  }
  try {
    await gwSave(input)
    showToast(editingId.value ? '已保存' : '已创建', 'success')
    closeDialog()
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}
</script>

<template>
  <div class="gw">
    <div class="view-switch" role="tablist" aria-label="端口工具">
      <button
        type="button"
        role="tab"
        :aria-selected="activeView === 'rules'"
        :class="{ active: activeView === 'rules' }"
        @click="selectView('rules')"
      >
        转发规则
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="activeView === 'connections'"
        :class="{ active: activeView === 'connections' }"
        @click="selectView('connections')"
      >
        请求记录
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="activeView === 'listeners'"
        :class="{ active: activeView === 'listeners' }"
        @click="selectView('listeners')"
      >
        端口占用
      </button>
    </div>

    <template v-if="activeView === 'rules'">
      <div class="toolbar">
        <Button variant="default" size="sm" class="btn primary" @click="openAdd"><Icon name="plus" :size="14" /> 新建规则</Button>
        <div class="chip stats" :class="{ live: runningCount > 0 }">
          <span class="dot-mini" /> 运行中 {{ runningCount }}
        </div>
      </div>

      <div class="list">
        <div v-for="r in rules" :key="r.id" class="card">
          <span class="dot" :class="{ on: r.running }" :title="r.running ? '运行中' : '已停止'" />
          <div class="info">
            <div class="line1">
              <span class="name" :title="r.name">{{ r.name }}</span>
              <span v-if="r.enabled" class="chip auto" title="应用启动时自动开启转发">自启</span>
              <span v-if="r.running" class="chip conns">{{ r.conns }} 连接</span>
            </div>
            <div class="route mono" :title="`${listenHost}:${r.listen_port} → ${r.target_host}:${r.target_port}`">
              <span>{{ listenHost }}:{{ r.listen_port }}</span>
              <span class="arrow">→</span>
              <span>{{ r.target_host }}:{{ r.target_port }}</span>
            </div>
            <div v-if="r.last_error" class="rule-error" role="alert">{{ r.last_error }}</div>
          </div>
          <div class="ops">
            <Button variant="default" size="sm" v-if="!r.running" class="btn primary sm" @click="toggleRun(r)">
              <Icon name="play" :size="12" /> 启动
            </Button>
            <Button variant="outline" size="sm" v-else class="btn stop sm" @click="toggleRun(r)">
              <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                <rect x="5" y="5" width="14" height="14" rx="2" />
              </svg>
              停止
            </Button>
            <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click="openEdit(r)">
              <Icon name="pencil" :size="14" />
            </Button>
            <Button variant="destructive" size="sm"
              v-if="confirmId !== r.id"
              class="icon-btn danger"
              title="删除"
              @click="askDelete(r)"
            >
              <Icon name="trash" :size="14" />
            </Button>
            <Button variant="outline" size="sm" v-else class="btn confirm-del sm" title="再点一次确认删除" @click="doDelete(r)">
              再点一次确认删除
            </Button>
          </div>
        </div>

        <div v-if="rules.length === 0" class="empty">
          <span class="empty-icon"><Icon name="arrow-left-right" :size="34" /></span>
          <div class="empty-title">还没有转发规则</div>
          <div class="empty-sub">在本机所有 IPv4 地址上监听端口，并将流量转发到任意目标主机。</div>
        </div>
      </div>

      <Teleport to="body">
        <div v-if="showDialog" class="mask" @click.self="closeDialog">
          <div class="dialog">
            <h2>{{ editingId ? '编辑转发规则' : '新建转发规则' }}</h2>

          <label class="field">
            <span>名称 *</span>
            <input
              v-model="form.name"
              type="text"
              placeholder="如：本机 Web 调试"
              :class="{ invalid: !!nameError }"
            />
            <span v-if="nameError" class="field-err">{{ nameError }}</span>
          </label>

          <div class="grid2">
            <label class="field">
              <span>监听端口 *（所有本机 IPv4 地址）</span>
              <input
                v-model.number="form.listen_port"
                type="number"
                min="1"
                max="65535"
                placeholder="8080"
                :class="{ invalid: !!listenError }"
              />
              <span v-if="listenError" class="field-err">{{ listenError }}</span>
            </label>
            <label class="field">
              <span>目标端口 *</span>
              <input
                v-model.number="form.target_port"
                type="number"
                min="1"
                max="65535"
                placeholder="80"
                :class="{ invalid: !!portError }"
              />
              <span v-if="portError" class="field-err">{{ portError }}</span>
            </label>
          </div>

          <label class="field">
            <span>目标主机 *</span>
            <input
              v-model="form.target_host"
              type="text"
              placeholder="IP 或域名，如 192.168.1.10"
              :class="{ invalid: !!hostError }"
            />
            <span v-if="hostError" class="field-err">{{ hostError }}</span>
          </label>

          <div class="switch-row">
            <div class="switch-text">
              <span>开机自动启动</span>
              <span class="switch-sub">应用启动时自动开启该规则转发</span>
            </div>
            <button
              type="button"
              class="switch"
              :class="{ on: form.enabled }"
              role="switch"
              :aria-checked="form.enabled"
              @click="form.enabled = !form.enabled"
            >
              <span class="knob" />
            </button>
          </div>

          <div class="switch-row">
            <div class="switch-text">
              <span>显示在预览页</span>
              <span class="switch-sub">仅勾选的规则会出现在悬浮预览</span>
            </div>
            <button
              type="button"
              class="switch"
              :class="{ on: form.in_preview }"
              role="switch"
              :aria-checked="form.in_preview"
              @click="form.in_preview = !form.in_preview"
            >
              <span class="knob" />
            </button>
          </div>

            <footer class="dialog-foot">
              <Button variant="outline" size="sm" class="btn" @click="closeDialog">取消</Button>
              <Button variant="default" size="sm" class="btn primary" @click="save">保存</Button>
            </footer>
          </div>
        </div>
      </Teleport>
    </template>

    <section v-else-if="activeView === 'listeners'" class="listener-view">
      <div class="listener-toolbar">
        <label class="port-search">
          <span>端口号</span>
          <input
            v-model.number="portQuery"
            type="number"
            min="1"
            max="65535"
            placeholder="留空显示全部"
            @keyup.enter="refreshListeners()"
          />
        </label>
        <Button variant="default" size="sm" class="btn primary" :disabled="listenerRefreshing" @click="refreshListeners()">
          {{ listenerRefreshing ? '查询中…' : '检查端口' }}
        </Button>
        <span class="listener-summary">{{ filterDescription }} · {{ listeners.length }} 项</span>
      </div>

      <div class="listener-list">
        <article
          v-for="listener in listeners"
          :key="`${listener.protocol}:${listener.address}:${listener.port}:${listener.pid}`"
          class="listener-card"
        >
          <span class="protocol-chip" :class="listener.protocol.toLowerCase()">{{ listener.protocol }}</span>
          <div class="listener-endpoint mono">
            <span>{{ listener.address }}:{{ listener.port }}</span>
            <small>本地监听地址</small>
          </div>
          <div class="listener-process">
            <div class="listener-name" :title="listener.name">{{ listener.name }}</div>
            <div class="listener-meta mono">PID {{ listener.pid }}</div>
            <div v-if="listener.path" class="listener-path" :title="listener.path">{{ listener.path }}</div>
            <div v-else class="listener-path muted">进程路径不可读取</div>
          </div>
          <Button variant="outline" size="sm"
            v-if="listener.can_terminate"
            class="btn terminate sm"
            :disabled="terminatingPid === listener.pid"
            @click="openTerminate(listener)"
          >
            {{ terminatingPid === listener.pid ? '处理中…' : '结束进程' }}
          </Button>
          <span v-else class="blocked-action" :title="listener.blocked_reason || ''">不可结束</span>
        </article>

        <div v-if="listeners.length === 0 && !listenerRefreshing" class="empty">
          <span class="empty-icon"><Icon name="search" :size="32" /></span>
          <div class="empty-title">
            {{ queriedPort === '' ? '没有发现监听端口' : `端口 ${queriedPort} 当前没有监听项` }}
          </div>
          <div class="empty-sub">同时检查 TCP、UDP 以及 IPv4、IPv6 监听项。</div>
        </div>
      </div>
    </section>

    <section v-else class="connections-view">
      <div class="connection-toolbar">
        <div class="connection-retention">
          仅内存 · 最近 15 分钟 · 最多 200 条 · 停用插件后清空
        </div>
        <Button
          variant="outline"
          size="sm"
          class="btn sm"
          :disabled="connectionLogs.length === 0"
          @click="clearConnectionLogs"
        >
          清空记录
        </Button>
      </div>
      <div class="connection-note">
        每条 TCP 连接记录一次；明文 HTTP 显示首次识别到的方法和 URL。TLS 只显示可读取的 SNI 主机名；其他 TCP 仅显示流量统计。请求正文和非 Host 请求头不会保存。
      </div>
      <div class="connection-list">
        <article v-for="log in connectionLogs" :key="log.id" class="connection-card">
          <header class="connection-card-head">
            <div class="connection-badges">
              <span class="protocol-chip">{{ log.protocol }}</span>
              <span
                class="connection-status"
                :class="{
                  active: !log.finished,
                  failed: log.status.includes('失败') || log.status.includes('异常')
                }"
              >{{ log.status }}</span>
              <span class="connection-rule">{{ log.rule_name }}</span>
            </div>
            <time>{{ formatLogTime(log.started_at_ms) }} · {{ formatDuration(log.elapsed_ms) }}</time>
          </header>
          <div class="connection-route mono">
            <div class="connection-hop">
              <small>来源</small>
              <span :title="log.source_addr">{{ log.source_addr }}</span>
            </div>
            <span class="connection-arrow">→</span>
            <div class="connection-hop">
              <small>接入</small>
              <span :title="log.listen_addr">{{ log.listen_addr }}</span>
            </div>
            <span class="connection-arrow">→</span>
            <div class="connection-hop target-hop">
              <small>目标</small>
              <span :title="log.actual_target_addr || log.target_endpoint">
                {{ log.actual_target_addr || log.target_endpoint }}
              </span>
            </div>
          </div>
          <div class="connection-request">
            <span v-if="log.method" class="method-chip">{{ log.method }}</span>
            <span v-if="log.request_url" class="request-url mono" :title="log.request_url">{{ log.request_url }}</span>
            <span v-else-if="log.protocol === 'TLS' && log.host" class="request-hint mono">TLS SNI：{{ log.host }}（路径加密）</span>
            <span v-else-if="log.protocol === 'TLS'" class="request-hint">TLS 加密连接，未读取到 SNI</span>
            <span v-else-if="log.protocol === 'TCP'" class="request-hint">TCP 数据流 · 不解析应用层内容</span>
            <span v-else class="request-hint">尚未识别到 HTTP 请求</span>
          </div>
          <div v-if="log.error" class="connection-error">{{ log.error }}</div>
          <footer class="connection-card-foot">
            <span>请求 {{ formatBytes(log.bytes_from_client) }}</span>
            <span>响应 {{ formatBytes(log.bytes_from_target) }}</span>
          </footer>
        </article>
        <div v-if="connectionLogs.length === 0" class="empty">
          <span class="empty-icon"><Icon name="activity" :size="32" /></span>
          <div class="empty-title">暂无请求记录</div>
          <div class="empty-sub">有设备连接转发端口后，这里会显示来源、目标和可识别的请求信息。</div>
        </div>
      </div>
    </section>

    <Teleport to="body">
      <div v-if="terminateTarget" class="mask" @click.self="terminateTarget = null">
        <div class="dialog terminate-dialog">
          <h2>强制结束占用进程？</h2>
          <p class="terminate-summary">
            这会结束 <strong>{{ terminateTarget.name }}</strong>（PID {{ terminateTarget.pid }}），
            并释放它占用的所有端口。进程中的未保存内容可能丢失。
          </p>
          <div class="terminate-path mono" :title="terminateTarget.path || ''">
            {{ terminateTarget.path }}
          </div>
          <p class="terminate-warning">如果这是由 Windows 服务管理的进程，它可能会自动重新启动。</p>
          <footer class="dialog-foot">
            <Button variant="outline" size="sm" class="btn" :disabled="terminatingPid !== null" @click="terminateTarget = null">取消</Button>
            <Button variant="outline" size="sm" class="btn terminate-confirm" :disabled="terminatingPid !== null" @click="terminateProcess">
              {{ terminatingPid !== null ? '处理中…' : '强制结束进程' }}
            </Button>
          </footer>
        </div>
      </div>
    </Teleport>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.gw {
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
  flex: none;
}

.view-switch {
  display: flex;
  align-self: flex-start;
  flex: none;
  gap: 3px;
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: 11px;
  background: color-mix(in srgb, var(--surface-subtle) 65%, transparent);
}
.view-switch button {
  border: 0;
  border-radius: 8px;
  padding: 6px 12px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
}
.view-switch button.active {
  background: var(--surface-container);
  color: var(--text);
  box-shadow: 0 1px 3px color-mix(in srgb, #000 14%, transparent);
}

/* ---------- 通用按钮 ---------- */

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
  white-space: nowrap;
}
.btn:hover {
  border-color: var(--border-strong);
}
.btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.btn.primary {
  background: var(--primary);
  border-color: var(--primary);
  color: var(--primary-foreground);
}
.btn.primary:hover {
  filter: brightness(1.1);
}
.btn.sm {
  padding: 5px 12px;
  font-size: 12px;
  border-radius: 8px;
}
.btn.stop {
  background: var(--surface-container);
  color: var(--text-muted);
}
.btn.stop:hover {
  color: var(--text);
}
.btn.confirm-del {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  border-color: color-mix(in srgb, var(--danger) 55%, transparent);
  color: var(--danger);
  font-size: 11.5px;
  padding: 5px 10px;
}
.btn.terminate,
.btn.terminate-confirm {
  background: color-mix(in srgb, var(--danger) 13%, transparent);
  border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  color: var(--danger);
}
.btn.terminate:hover,
.btn.terminate-confirm:hover {
  border-color: var(--danger);
  background: color-mix(in srgb, var(--danger) 20%, transparent);
}

.listener-view {
  display: flex;
  flex: 1;
  min-height: 0;
  flex-direction: column;
  gap: 10px;
}
.connections-view {
  display: flex;
  flex: 1;
  min-height: 0;
  flex-direction: column;
  gap: 8px;
}
.connection-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: none;
  gap: 10px;
}
.connection-retention,
.connection-note {
  color: var(--text-muted);
  font-size: 10.5px;
}
.connection-note {
  flex: none;
  line-height: 1.5;
}
.connection-list {
  display: flex;
  flex: 1;
  min-height: 0;
  flex-direction: column;
  gap: 7px;
  overflow: auto;
}
.connection-card {
  display: flex;
  flex: none;
  flex-direction: column;
  gap: 7px;
  padding: 9px 11px;
  border: 1px solid var(--border);
  border-radius: 11px;
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.connection-card-head,
.connection-card-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.connection-card-head time,
.connection-card-foot {
  color: var(--text-muted);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.connection-badges {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.connection-status,
.connection-rule,
.method-chip {
  overflow: hidden;
  max-width: 140px;
  padding: 3px 6px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--surface-container) 75%, transparent);
  color: var(--text-muted);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.connection-status.active {
  background: color-mix(in srgb, var(--success) 12%, transparent);
  color: var(--success);
}
.connection-status.failed {
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  color: var(--danger);
}
.connection-route {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  gap: 7px;
}
.connection-hop {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}
.connection-hop small {
  color: var(--text-muted);
  font-family: inherit;
  font-size: 9.5px;
}
.connection-hop span {
  overflow: hidden;
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.connection-arrow {
  color: var(--success);
  font-family: inherit;
  font-size: 12px;
}
.connection-request {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 6px 8px;
  border-radius: 7px;
  background: color-mix(in srgb, var(--surface-container) 55%, transparent);
}
.method-chip {
  flex: none;
  background: color-mix(in srgb, var(--primary) 15%, transparent);
  color: var(--primary);
  font-weight: 700;
}
.request-url,
.request-hint {
  overflow: hidden;
  color: var(--text);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.request-hint {
  color: var(--text-muted);
}
.rule-error {
  margin-top: 5px;
  color: var(--danger);
  font-size: 11px;
  line-height: 1.5;
  overflow-wrap: anywhere;
}
.connection-error {
  overflow-wrap: anywhere;
  color: var(--danger);
  font-size: 10.5px;
}
.connection-card-foot {
  justify-content: flex-start;
  gap: 14px;
}
.listener-toolbar {
  display: flex;
  align-items: flex-end;
  flex: none;
  flex-wrap: wrap;
  gap: 9px;
}
.port-search {
  display: flex;
  width: 190px;
  flex-direction: column;
  gap: 4px;
  color: var(--text-muted);
  font-size: 11px;
}
.port-search input {
  width: 100%;
  box-sizing: border-box;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--surface-subtle);
  color: var(--text);
  font: 12px ui-monospace, 'Cascadia Mono', Consolas, 'Courier New', monospace;
  outline: none;
}
.port-search input:focus {
  border-color: var(--primary);
}
.listener-summary {
  margin-left: auto;
  padding-bottom: 7px;
  color: var(--text-muted);
  font-size: 11.5px;
  white-space: nowrap;
}
.listener-list {
  display: flex;
  flex: 1;
  min-height: 0;
  flex-direction: column;
  gap: 7px;
  overflow: auto;
}
.listener-card {
  display: grid;
  flex: none;
  grid-template-columns: 48px minmax(125px, 0.9fr) minmax(150px, 1.3fr) auto;
  align-items: center;
  gap: 10px;
  padding: 10px 11px;
  border: 1px solid var(--border);
  border-radius: 11px;
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.protocol-chip {
  justify-self: start;
  padding: 3px 6px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--primary) 13%, transparent);
  color: var(--primary);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.02em;
}
.protocol-chip.udp {
  background: color-mix(in srgb, var(--warning) 15%, transparent);
  color: var(--warning);
}
.listener-endpoint,
.listener-process {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 3px;
}
.listener-endpoint > span,
.listener-name,
.listener-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.listener-endpoint > span {
  font-size: 11.5px;
}
.listener-endpoint small,
.listener-meta,
.listener-path,
.blocked-action {
  color: var(--text-muted);
  font-size: 10.5px;
}
.listener-name {
  font-size: 12px;
  font-weight: 600;
}
.listener-meta {
  font-size: 10px;
}
.listener-path {
  opacity: 0.82;
  font-size: 9.5px;
}
.listener-path.muted {
  font-style: italic;
}
.blocked-action {
  justify-self: center;
  white-space: nowrap;
}
.terminate-dialog {
  width: min(500px, calc(100vw - 48px));
}
.terminate-summary,
.terminate-warning {
  margin: 0 0 12px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}
.terminate-summary strong {
  color: var(--text);
}
.terminate-path {
  overflow: hidden;
  margin-bottom: 12px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text-muted);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.terminate-warning {
  margin-bottom: 16px;
  color: var(--warning);
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
  flex: none;
}
.icon-btn:hover {
  background: var(--surface-container);
  color: var(--primary);
}
.icon-btn.danger:hover {
  color: var(--danger);
}

/* ---------- 统计 chip ---------- */

.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 9px;
  border-radius: 999px;
  font-size: 11px;
  white-space: nowrap;
}
.chip.stats {
  border: 1px solid var(--border);
  color: var(--text-muted);
}
.chip.stats.live {
  color: var(--success);
  border-color: color-mix(in srgb, var(--success) 45%, transparent);
  background: color-mix(in srgb, var(--success) 10%, transparent);
}
.dot-mini {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  flex: none;
}

/* ---------- 规则卡片列表 ---------- */

.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 11px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
  transition: border-color 0.15s ease;
  flex: none;
}
.card:hover {
  border-color: var(--border-strong);
}

.dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--border-strong);
  opacity: 0.55;
  flex: none;
}
.dot.on {
  background: var(--success);
  opacity: 1;
  animation: breathe 2s ease-in-out infinite;
}
@keyframes breathe {
  0%,
  100% {
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--success) 40%, transparent);
  }
  50% {
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--success) 0%, transparent);
  }
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
  gap: 8px;
  min-width: 0;
}
.name {
  font-weight: 600;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chip.auto {
  background: color-mix(in srgb, var(--warning) 14%, transparent);
  color: var(--warning);
  flex: none;
}
.chip.conns {
  background: color-mix(in srgb, var(--primary) 22%, transparent);
  color: var(--text);
  flex: none;
  font-variant-numeric: tabular-nums;
}

.route {
  display: flex;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-muted);
}
.route > span {
  overflow: hidden;
  text-overflow: ellipsis;
}
.route .arrow {
  color: var(--success);
  font-weight: 700;
  flex: none;
}
.mono {
  font-family: ui-monospace, 'Cascadia Mono', Consolas, 'Courier New', monospace;
  font-variant-numeric: tabular-nums;
}

.ops {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

/* ---------- 空状态 ---------- */

.empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 40px 24px;
  text-align: center;
  color: var(--text-muted);
}
.empty-icon {
  display: flex;
  opacity: 0.45;
  margin-bottom: 8px;
}
.empty-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
}
.empty-sub {
  font-size: 12px;
  max-width: 320px;
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
  width: min(440px, calc(100vw - 48px));
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

.grid2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: 4px 0 16px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.switch-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12.5px;
}
.switch-sub {
  font-size: 11px;
  color: var(--text-muted);
}

.switch {
  position: relative;
  width: 38px;
  height: 20px;
  flex: none;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  background: var(--surface-subtle);
  padding: 0;
  transition:
    background 0.15s ease,
    border-color 0.15s ease;
}
.switch .knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--border-strong);
  transition:
    transform 0.15s ease,
    background 0.15s ease;
}
.switch.on {
  background: var(--primary);
  border-color: var(--primary);
}
.switch.on .knob {
  transform: translateX(18px);
  background: var(--primary-foreground);
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}
</style>
