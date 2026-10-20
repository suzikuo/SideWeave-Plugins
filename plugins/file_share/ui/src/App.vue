<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { onPluginLifecycle } from '../../../ui-sdk/runtime'
import {
  addUser,
  copyText,
  deleteFileShare,
  deleteUser,
  clearAccessLogs,
  getFileShares,
  openUrl,
  pickDirectory,
  rotateToken,
  saveFileShare,
  setUserPassword,
  startFileShare,
  stopFileShare,
  suggestPort,
  type FileShareAccessLog,
  type FileShareConfig,
  type FileShareDetail,
  type FileShareView
} from './api'

const detail = ref<FileShareDetail | null>(null)
const loading = ref(false)
const saving = ref(false)
const actionBusy = ref(false)
const errorText = ref('')
const toast = ref('')
const expanded = ref<string[]>([])
const editorOpen = ref(false)
const editingId = ref<string | null>(null)
const accountDialog = ref(false)
const accountMode = ref<'add' | 'password'>('add')
const accountForm = ref({ username: '', password: '' })
const logShare = ref<FileShareView | null>(null)
const form = ref(defaultForm())
let refreshTimer = 0
let unlistenLifecycle: (() => void) | null = null

function defaultForm() {
  return {
    id: '', name: '', view_directory: '', receive_directory: '', port: 12026, tunnel_hostname: '',
    lan_enabled: false, allowed_ips: '', allow_view: true, allow_upload: false,
    require_login: false, media_preview_limit_kbps: 512, max_upload_mb: 100,
    max_total_upload_mb: 2048
  }
}

const runningCount = computed(() => detail.value?.file_shares.filter((share) => share.local_running).length ?? 0)
const users = computed(() => detail.value?.users ?? [])

function notify(message: string, isError = false) {
  errorText.value = isError ? message : ''
  toast.value = message
  window.setTimeout(() => { if (toast.value === message) toast.value = '' }, 3200)
}

async function loadData(silent = false) {
  if (loading.value) return
  loading.value = true
  try {
    detail.value = await getFileShares()
    errorText.value = ''
  } catch (error: any) {
    if (!silent) notify(String(error?.message || error), true)
  } finally {
    loading.value = false
  }
}

function setActive(active: boolean) {
  window.clearInterval(refreshTimer)
  refreshTimer = 0
  if (active) {
    void loadData(true)
    refreshTimer = window.setInterval(() => void loadData(true), 2500)
  }
}

onMounted(() => { unlistenLifecycle = onPluginLifecycle(setActive) })
onUnmounted(() => { window.clearInterval(refreshTimer); unlistenLifecycle?.() })

function toggleDetails(id: string) {
  expanded.value = expanded.value.includes(id) ? expanded.value.filter((item) => item !== id) : [...expanded.value, id]
}

function openNew() {
  editingId.value = null
  form.value = defaultForm()
  editorOpen.value = true
  void suggestPort().then((port) => { if (!editingId.value) form.value.port = port }).catch(() => {})
}

function openEdit(share: FileShareView) {
  editingId.value = share.id
  form.value = {
    id: share.id, name: share.name, view_directory: share.view_directory,
    receive_directory: share.receive_directory, port: share.port, tunnel_hostname: share.tunnel_hostname,
    lan_enabled: share.lan_enabled, allowed_ips: share.allowed_ips.join('\n'),
    allow_view: share.allow_view, allow_upload: share.allow_upload,
    require_login: share.require_login,
    media_preview_limit_kbps: share.media_preview_limit_kbps,
    max_upload_mb: share.max_upload_mb, max_total_upload_mb: share.max_total_upload_mb
  }
  editorOpen.value = true
}

async function chooseDirectory(kind: 'view' | 'receive') {
  try {
    const path = await pickDirectory(kind)
    if (!path) return
    if (kind === 'view') form.value.view_directory = path
    else form.value.receive_directory = path
  } catch (error: any) { notify(String(error?.message || error), true) }
}

async function save() {
  if (saving.value) return
  const value = form.value
  if (!value.name.trim() || (value.allow_view && !value.view_directory.trim()) || (value.allow_upload && !value.receive_directory.trim() && !value.allow_view) || (!value.allow_view && !value.allow_upload)) {
    notify('请检查实例名称、目录和访问权限。', true)
    return
  }
  saving.value = true
  const config: FileShareConfig = {
    id: value.id, name: value.name.trim(), enabled: false,
    view_directory: value.view_directory.trim(), receive_directory: value.receive_directory.trim(),
    port: value.port, tunnel_hostname: value.tunnel_hostname.trim(), lan_enabled: value.lan_enabled,
    allowed_ips: value.allowed_ips.split(/[\n,]/).map((item) => item.trim()).filter(Boolean),
    allow_view: value.allow_view, allow_upload: value.allow_upload,
    require_login: value.require_login, media_preview_limit_kbps: value.media_preview_limit_kbps,
    max_upload_mb: value.max_upload_mb, max_total_upload_mb: value.max_total_upload_mb
  }
  try {
    await saveFileShare(config)
    editorOpen.value = false
    notify('实例已保存')
    await loadData(true)
  } catch (error: any) { notify(String(error?.message || error), true) }
  finally { saving.value = false }
}

async function toggle(share: FileShareView) {
  if (actionBusy.value) return
  actionBusy.value = true
  try {
    if (share.local_running) await stopFileShare(share.id)
    else await startFileShare(share.id)
    await loadData(true)
  } catch (error: any) { notify(String(error?.message || error), true); await loadData(true) }
  finally { actionBusy.value = false }
}

async function removeShare(share: FileShareView) {
  if (!window.confirm(`确定删除“${share.name}”吗？`)) return
  try {
    await deleteFileShare(share.id)
    await loadData(true)
  }
  catch (error: any) { notify(String(error?.message || error), true) }
}

async function rotate(share: FileShareView) {
  if (!window.confirm(`重置“${share.name}”的令牌？旧链接将立即失效。`)) return
  try { await rotateToken(share.id); await loadData(true) }
  catch (error: any) { notify(String(error?.message || error), true) }
}

async function copy(value: string, label: string) {
  try { await copyText(value); notify(`${label}已复制`) }
  catch (error: any) { notify(String(error?.message || error), true) }
}

function openAccount(mode: 'add' | 'password', username = '') {
  accountMode.value = mode
  accountForm.value = { username, password: '' }
  accountDialog.value = true
}

async function saveAccount() {
  try {
    if (accountMode.value === 'add') await addUser(accountForm.value.username.trim(), accountForm.value.password)
    else await setUserPassword(accountForm.value.username, accountForm.value.password)
    accountDialog.value = false
    notify('账号已保存')
    await loadData(true)
  } catch (error: any) { notify(String(error?.message || error), true) }
}

async function removeUser(username: string) {
  if (!window.confirm(`确定删除账号“${username}”吗？`)) return
  try { await deleteUser(username); await loadData(true) }
  catch (error: any) { notify(String(error?.message || error), true) }
}

function formatBytes(bytes: number | null) {
  if (bytes === null) return '—'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

const visibleLogs = computed<FileShareAccessLog[]>(() => logShare.value?.access_logs ?? [])
const selectedLog = ref<FileShareAccessLog | null>(null)

async function clearLogs() {
  const share = logShare.value
  if (!share || !window.confirm(`清空“${share.name}”的访问日志？`)) return
  try {
    await clearAccessLogs(share.id)
    await loadData(true)
    logShare.value = detail.value?.file_shares.find((item) => item.id === share.id) ?? null
    selectedLog.value = null
    notify('访问日志已清空')
  } catch (error: any) { notify(String(error?.message || error), true) }
}
</script>

<template>
  <main class="page">
    <header class="toolbar">
      <div class="summary"><span class="status-dot" :class="runningCount ? 'online' : ''" /><strong>{{ runningCount }} / {{ detail?.file_shares.length ?? 0 }} 运行</strong></div>
      <div class="toolbar-actions"><button class="button subtle" @click="loadData()">刷新</button><button class="button primary" @click="openNew">＋ 新建实例</button></div>
    </header>

    <p v-if="errorText" class="notice error">{{ errorText }}</p>

    <section class="instance-list">
      <article v-for="share in detail?.file_shares ?? []" :key="share.id" class="instance">
        <header class="instance-head">
          <div class="instance-name">
            <span class="status-dot" :class="share.local_running ? 'online' : ''" />
            <strong>{{ share.name }}</strong><span class="tag">{{ share.local_running ? '运行中' : '已停止' }}</span><span class="tag mono">{{ share.port }}</span>
            <button class="button detail-button" :aria-expanded="expanded.includes(share.id)" @click="toggleDetails(share.id)">{{ expanded.includes(share.id) ? '收起详情' : '查看详情' }}</button>
          </div>
          <div class="row-actions">
            <button class="button subtle" @click="logShare = share">日志 {{ share.access_logs.length }}</button>
            <button class="button subtle" @click="openEdit(share)">编辑</button>
            <button class="button subtle" :disabled="actionBusy" @click="toggle(share)">{{ share.local_running ? '停止' : '启动' }}</button>
            <button class="button danger" @click="removeShare(share)">删除</button>
          </div>
        </header>
        <div v-if="expanded.includes(share.id)" class="instance-detail">
          <div class="chips">
            <span class="tag">{{ share.allow_view ? '允许查看 / 下载' : '禁止查看' }}</span>
            <span class="tag">{{ share.allow_upload ? '允许上传' : '禁止上传' }}</span>
            <span class="tag">{{ share.require_login ? '共享账号登录' : '令牌后缀认证' }}</span>
            <span class="tag">{{ share.lan_enabled ? '局域网监听' : '仅本机监听' }}</span>
            <span v-if="share.allow_view" class="tag">媒体预览 {{ share.media_preview_limit_kbps }} KB/s</span>
          </div>
          <div class="paths">
            <div v-if="share.allow_view"><small>查看目录</small><code>{{ share.view_directory }}</code></div>
            <div v-if="share.allow_upload"><small>收件目录</small><code>{{ share.receive_directory }}</code></div>
          </div>
          <div v-if="share.local_url || share.lan_share_url" class="links">
            <div v-if="share.local_url" class="link-row"><b>本机</b><code>{{ share.local_url }}</code><span v-if="!share.local_running" class="tag">未启动</span><button v-if="share.local_running" class="button subtle" @click="openUrl(share.local_url)">打开</button><button class="button subtle" @click="copy(share.local_url, '本机链接')">复制</button></div>
            <div v-if="share.lan_share_url" class="link-row"><b>局域网</b><code>{{ share.lan_share_url }}</code><span v-if="!share.lan_running" class="tag">未启动</span><button class="button subtle" @click="copy(share.lan_share_url, '局域网链接')">复制</button></div>
            <div v-if="share.cloudflare_url" class="link-row"><b>Cloudflare</b><code>{{ share.cloudflare_url }}</code><button class="button subtle" @click="copy(share.cloudflare_url, 'Cloudflare 链接')">复制</button></div>
            <div v-if="share.token" class="link-row token-row"><b>访问令牌</b><code>{{ share.token }}</code><button class="button subtle" @click="copy(share.token, '访问令牌')">复制令牌</button></div>
          </div>
          <footer class="detail-foot">
            <span v-if="share.lan_enabled" class="warning">局域网使用 HTTPS 加密传输。证书为自签名，浏览器会提示连接不安全，确认继续即可。</span>
            <span v-else>{{ share.require_login ? '停用服务或修改账号密码会使现有会话失效。' : '分享链接包含访问令牌，请只发送给可信对象。' }}</span>
            <button v-if="!share.require_login" class="button subtle" @click="rotate(share)">重置令牌</button>
          </footer>
          <p class="manual-note">填写 Cloudflare 域名只用于生成分享链接；请在 Cloudflare Tunnel 插件中手动添加该域名到本机端口的转发规则。上传实例会自动启用“关闭分块编码”。登录模式不带令牌，令牌模式的公网链接会包含访问令牌。</p>
        </div>
      </article>
      <div v-if="detail && !detail.file_shares.length" class="empty"><strong>还没有文件服务实例</strong><span>新建实例后，可以独立设置目录、端口、访问权限和认证方式。</span></div>
      <div v-if="!detail && !errorText" class="empty">正在读取文件服务…</div>
    </section>

    <section class="accounts">
      <header class="section-head"><div><strong>共享登录账号</strong><span class="tag">{{ users.length }}</span></div><button class="button subtle" @click="openAccount('add')">添加账号</button></header>
      <p class="muted">所有要求登录的实例共用这些账号，不开放访客注册，也不按实例区分账号权限。</p>
      <div v-if="users.length" class="user-list"><div v-for="username in users" :key="username" class="user-row"><strong>{{ username }}</strong><div><button class="button subtle" @click="openAccount('password', username)">修改密码</button><button class="button danger" @click="removeUser(username)">删除</button></div></div></div>
      <div v-else class="muted">还没有共享账号。要求登录的实例需要至少一个账号才能启动。</div>
    </section>

    <div v-if="editorOpen" class="overlay" @click.self="editorOpen = false">
      <form class="dialog" @submit.prevent="save">
        <header class="dialog-head"><div><strong>{{ editingId ? '编辑实例' : '新建文件服务' }}</strong><small>每个实例使用独立端口；Cloudflare 转发规则需在 Tunnel 插件中手动维护</small></div><button type="button" class="icon-button" @click="editorOpen = false">×</button></header>
        <div class="dialog-body">
          <label class="field"><span>实例名称</span><input v-model="form.name" required maxlength="64" placeholder="例如：公开只读目录" /></label>
          <label class="field"><span>服务端口</span><input v-model.number="form.port" type="number" min="1024" max="65535" required /></label>
          <label class="field"><span>Cloudflare 域名 <small>可选，仅用于生成分享链接；转发规则请在 Tunnel 插件中手动添加</small></span><input v-model="form.tunnel_hostname" type="text" placeholder="files.example.com" autocomplete="off" /></label>
          <label class="option"><input v-model="form.allow_view" type="checkbox" /><span><b>允许查看和下载</b><small>关闭后只显示上传页面，不暴露目录内容</small></span></label>
          <label v-if="form.allow_view" class="field"><span>查看目录</span><div class="picker"><input v-model="form.view_directory" readonly placeholder="选择查看根目录" /><button type="button" class="button subtle" @click="chooseDirectory('view')">选择</button></div></label>
          <label v-if="form.allow_view" class="field"><span>媒体预览限速（KB/s）</span><input v-model.number="form.media_preview_limit_kbps" type="number" min="64" max="8192" step="64" /></label>
          <label class="option"><input v-model="form.allow_upload" type="checkbox" /><span><b>允许上传</b><small>上传内容写入独立收件目录，不覆盖同名文件</small></span></label>
          <div v-if="form.allow_upload" class="security-note">上传由原生插件写入所选目录。插件权限页会将文件写入权限设为必需；原生插件不是操作系统沙箱，请只选择可信目录。</div>
          <label v-if="form.allow_upload" class="field"><span>收件目录</span><div class="picker"><input v-model="form.receive_directory" readonly placeholder="留空时在查看目录旁创建收件目录" /><button type="button" class="button subtle" @click="chooseDirectory('receive')">选择</button></div><small>可与查看目录相同、不同或嵌套；落在查看范围内的文件会显示。</small></label>
          <div v-if="form.allow_upload" class="grid"><label class="field"><span>单文件上限（MB）</span><input v-model.number="form.max_upload_mb" type="number" min="1" max="1024" /></label><label class="field"><span>收件目录上限（MB）</span><input v-model.number="form.max_total_upload_mb" type="number" min="1" max="8192" /></label></div>
          <div class="grid"><label class="option"><input v-model="form.lan_enabled" type="checkbox" /><span><b>允许局域网访问</b><small>监听本机所有网络接口</small></span></label><label class="option"><input v-model="form.require_login" type="checkbox" /><span><b>要求共享账号登录</b><small>登录模式不使用令牌后缀</small></span></label></div>
          <label class="field"><span>IP 白名单 <small>每行一个 IP 或 CIDR，可留空</small></span><textarea v-model="form.allowed_ips" rows="3" class="mono" placeholder="192.168.1.20&#10;192.168.1.0/24" /></label>
          <div v-if="form.lan_enabled" class="security-note warning">局域网使用 HTTPS 加密传输。证书为自签名，浏览器会提示连接不安全，确认继续即可。</div>
        </div>
        <footer class="dialog-foot"><button type="button" class="button subtle" @click="editorOpen = false">取消</button><button type="submit" class="button primary" :disabled="saving">{{ saving ? '保存中…' : '保存实例' }}</button></footer>
      </form>
    </div>

    <div v-if="accountDialog" class="overlay" @click.self="accountDialog = false">
      <form class="dialog account-dialog" @submit.prevent="saveAccount">
        <header class="dialog-head"><div><strong>{{ accountMode === 'add' ? '添加共享账号' : `修改 ${accountForm.username} 的密码` }}</strong><small>密码长度不限，至少填写 1 个字符</small></div><button type="button" class="icon-button" @click="accountDialog = false">×</button></header>
        <div class="dialog-body"><label v-if="accountMode === 'add'" class="field"><span>用户名</span><input v-model="accountForm.username" required minlength="3" maxlength="48" autocomplete="username" /></label><label class="field"><span>{{ accountMode === 'add' ? '密码' : '新密码' }}</span><input v-model="accountForm.password" type="password" required autocomplete="new-password" /></label></div>
        <footer class="dialog-foot"><button type="button" class="button subtle" @click="accountDialog = false">取消</button><button type="submit" class="button primary">保存</button></footer>
      </form>
    </div>

    <div v-if="logShare" class="overlay" @click.self="logShare = null">
      <section class="dialog logs-dialog">
        <header class="dialog-head"><div><strong>{{ logShare.name }} · 访问日志</strong><small>仅保存在内存，最多保留 100 条</small></div><div class="row-actions"><button class="button danger" :disabled="!visibleLogs.length" @click="clearLogs">清空日志</button><button class="icon-button" @click="logShare = null">×</button></div></header>
        <div class="logs-body"><div class="logs-table-wrap"><table class="logs-table"><thead><tr><th>时间</th><th>客户端</th><th>请求</th><th>状态</th><th>流量</th><th>耗时</th><th></th></tr></thead><tbody>
          <tr v-for="(entry, index) in visibleLogs" :key="entry.request_id || `${entry.timestamp}-${index}`" @click="selectedLog = entry">
            <td>{{ entry.timestamp }}</td><td>{{ entry.client_ip }}</td><td class="request-cell"><b>{{ entry.method || '未知' }}</b><code>{{ entry.path || '(无法解析请求)' }}</code></td><td :class="entry.status_code >= 400 ? 'bad' : 'good'">{{ entry.status_code || '中断' }} {{ entry.status_text }}</td><td>收 {{ formatBytes(entry.received_bytes) }}<br />发 {{ formatBytes(entry.sent_bytes) }}</td><td>{{ entry.duration_ms }} ms</td><td><button class="button subtle" @click.stop="selectedLog = entry">详情</button></td>
          </tr>
          <tr v-if="!visibleLogs.length"><td colspan="7" class="logs-empty">收到访问请求后会在这里显示。</td></tr>
        </tbody></table></div></div>
      </section>
    </div>

    <div v-if="selectedLog" class="overlay log-detail-overlay" @click.self="selectedLog = null">
      <section class="dialog log-detail-dialog"><header class="dialog-head"><div><strong>请求详情</strong><small>#{{ selectedLog.request_id || '—' }}</small></div><button class="icon-button" @click="selectedLog = null">×</button></header>
        <div class="log-detail-body"><dl>
          <dt>请求</dt><dd><b>{{ selectedLog.method || '未知' }}</b> <code>{{ selectedLog.path || '(无法解析请求)' }}</code></dd>
          <dt>状态</dt><dd :class="selectedLog.status_code >= 400 ? 'bad' : 'good'">{{ selectedLog.status_code || '中断' }} {{ selectedLog.status_text }}</dd>
          <dt>时间 / 客户端</dt><dd>{{ selectedLog.timestamp }} · {{ selectedLog.client_ip }} · {{ selectedLog.duration_ms }} ms</dd>
          <dt>协议 / Host</dt><dd>{{ selectedLog.protocol || '—' }} · {{ selectedLog.host || '—' }}</dd>
          <dt>请求类型 / 大小</dt><dd>{{ selectedLog.content_type || '—' }} · {{ formatBytes(selectedLog.request_content_length) }}</dd>
          <dt>上传进度</dt><dd>{{ formatBytes(selectedLog.received_bytes) }}<template v-if="selectedLog.expected_bytes !== null"> / {{ formatBytes(selectedLog.expected_bytes) }}</template></dd>
          <dt>响应类型 / 大小</dt><dd>{{ selectedLog.response_content_type || '—' }} · {{ formatBytes(selectedLog.sent_bytes) }}</dd>
          <dt>User-Agent</dt><dd>{{ selectedLog.user_agent || '—' }}</dd>
          <dt>Referer</dt><dd>{{ selectedLog.referer || '—' }}</dd>
          <template v-if="selectedLog.error"><dt>错误</dt><dd class="bad">{{ selectedLog.error }}</dd></template>
        </dl></div>
      </section>
    </div>

    <div v-if="toast" class="toast" :class="{ error: !!errorText }">{{ toast }}</div>
  </main>
</template>

<style>
:root { font-family: var(--font-ui); color: var(--text); background: transparent; }
* { box-sizing: border-box; }
body { margin: 0; color: var(--text); background: transparent; }
button, input, textarea { font: inherit; color: inherit; }
.page { min-height: 100vh; padding: 14px; display: flex; flex-direction: column; gap: 12px; }
.toolbar, .section-head, .instance-head, .dialog-head, .dialog-foot, .row-actions, .toolbar-actions, .user-row, .log-title, .log-meta { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.toolbar { min-height: 38px; }
.summary, .instance-name, .chips, .paths, .links, .detail-foot, .user-list { display: flex; align-items: center; gap: 8px; }
.summary { font-size: 12px; color: var(--text-muted); }
.summary strong { color: var(--text); font-weight: 600; }
.status-dot { width: 8px; height: 8px; flex: none; border-radius: 50%; background: #777; }
.status-dot.online { background: var(--success); box-shadow: 0 0 8px color-mix(in srgb, var(--success), transparent 65%); }
.button, .icon-button { border: 1px solid var(--border); background: var(--surface); border-radius: 5px; cursor: pointer; transition: background .15s, border-color .15s; }
.button { min-height: 29px; padding: 5px 9px; font-size: 11px; white-space: nowrap; }
.button:hover, .icon-button:hover { background: var(--surface-subtle); border-color: var(--border-strong); }
.button:disabled { opacity: .5; cursor: not-allowed; }
.button.primary { background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); font-weight: 600; }
.button.detail-button { color: var(--primary); border-color: color-mix(in srgb, var(--primary), var(--border) 45%); font-weight: 600; }
.button.detail-button[aria-expanded="true"] { background: var(--surface-subtle); }
.button.danger { color: var(--danger); }
.instance-list { display: flex; flex-direction: column; gap: 8px; }
.instance, .accounts { border: 1px solid var(--border); background: color-mix(in srgb, var(--surface), transparent 4%); border-radius: 8px; box-shadow: var(--shadow); overflow: hidden; }
.instance-head { padding: 9px 10px; flex-wrap: wrap; }
.instance-name { min-width: 170px; flex-wrap: wrap; font-size: 12px; }
.instance-name strong { font-size: 12px; font-weight: 600; }
.tag { padding: 3px 6px; border: 1px solid var(--border); border-radius: 99px; color: var(--text-muted); font-size: 10px; white-space: nowrap; }
.mono, code, textarea.mono { font-family: var(--font-mono); }
.row-actions { flex-wrap: wrap; justify-content: flex-end; }
.instance-detail { border-top: 1px solid var(--border); padding: 10px; display: grid; gap: 10px; color: var(--text-muted); font-size: 11px; }
.chips, .paths { flex-wrap: wrap; }
.paths { align-items: flex-start; }
.paths > div { min-width: min(100%, 270px); display: grid; gap: 4px; }
.paths small, .field small, .field > small, .dialog-head small { color: var(--text-muted); font-size: 10px; }
.paths code, .link-row code { overflow-wrap: anywhere; color: var(--text); }
.links { flex-direction: column; align-items: stretch; }
.link-row { min-width: 0; display: grid; grid-template-columns: 75px minmax(0, 1fr) auto auto; align-items: center; gap: 7px; }
.link-row b { color: var(--text); font-size: 11px; }
.detail-foot { flex-wrap: wrap; justify-content: space-between; border-top: 1px solid var(--border); padding-top: 8px; }
.warning, .security-note.warning { color: var(--warning); }
.manual-note { margin: 0; color: var(--text-muted); font-size: 10px; }
.empty { min-height: 90px; border: 1px dashed var(--border-strong); border-radius: 8px; display: grid; place-content: center; gap: 5px; text-align: center; color: var(--text-muted); font-size: 11px; }
.empty strong { color: var(--text); font-size: 12px; }
.accounts { padding: 10px; }
.section-head > div { display: flex; align-items: center; gap: 7px; font-size: 12px; }
.muted { color: var(--text-muted); font-size: 10px; line-height: 1.5; }
.user-list { flex-direction: column; align-items: stretch; }
.user-row { padding-top: 7px; border-top: 1px solid var(--border); font-size: 11px; }
.user-row > div { display: flex; gap: 6px; }
.notice { margin: 0; padding: 8px 10px; border-radius: 6px; font-size: 11px; }
.error, .bad { color: var(--danger); }
.good { color: var(--success); }
.overlay { position: fixed; inset: 0; z-index: 10; padding: 16px; display: grid; place-items: center; background: rgba(0,0,0,.55); backdrop-filter: blur(4px); }
.dialog { width: min(640px, 100%); max-height: min(90vh, 820px); display: flex; flex-direction: column; background: var(--surface); border: 1px solid var(--border-strong); border-radius: 10px; box-shadow: var(--shadow); overflow: hidden; }
.dialog-head { padding: 12px 14px; border-bottom: 1px solid var(--border); }
.dialog-head > div { display: grid; gap: 3px; }
.dialog-head strong { font-size: 13px; }
.icon-button { width: 28px; height: 28px; font-size: 18px; line-height: 1; }
.dialog-body { padding: 12px 14px; display: flex; flex-direction: column; gap: 10px; overflow: auto; }
.field { display: grid; gap: 5px; font-size: 11px; }
.field > span { color: var(--text-muted); }
.field input, .field textarea, .picker input { min-width: 0; width: 100%; padding: 8px 9px; color: var(--text); background: var(--bg); border: 1px solid var(--border); border-radius: 5px; outline: none; font-size: 11px; }
.field input:focus, .field textarea:focus { border-color: var(--border-strong); }
.field textarea { resize: vertical; }
.picker { display: flex; gap: 6px; }
.option { min-height: 42px; padding: 8px 9px; border: 1px solid var(--border); border-radius: 6px; display: flex; align-items: center; gap: 8px; cursor: pointer; }
.option input { accent-color: var(--primary); }
.option span { display: grid; gap: 3px; }
.option b { font-size: 11px; }
.option small { color: var(--text-muted); font-size: 10px; }
.security-note { padding: 8px 9px; border: 1px solid var(--border); border-radius: 6px; color: var(--text-muted); background: var(--surface-subtle); font-size: 10px; line-height: 1.45; }
.grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.dialog-foot { padding: 10px 14px; border-top: 1px solid var(--border); justify-content: flex-end; }
.logs-dialog { width: min(1080px, 100%); }
.logs-body { min-height: 180px; max-height: min(68vh, 680px); padding: 8px; overflow: auto; }
.logs-table-wrap { width: 100%; overflow: auto; }
.logs-table { width: 100%; border-collapse: collapse; font-size: 10px; text-align: left; }
.logs-table th, .logs-table td { padding: 8px 7px; border-bottom: 1px solid var(--border); white-space: nowrap; vertical-align: middle; }
.logs-table th { position: sticky; top: 0; z-index: 1; color: var(--text-muted); background: var(--surface); font-weight: 500; }
.logs-table tbody tr { cursor: pointer; }
.logs-table tbody tr:hover { background: var(--surface-subtle); }
.logs-table .request-cell { max-width: 330px; }
.request-cell b { display: inline-block; margin-right: 6px; }
.request-cell code { max-width: 280px; overflow: hidden; text-overflow: ellipsis; vertical-align: bottom; color: var(--text-muted); }
.logs-empty { height: 130px; text-align: center; color: var(--text-muted); }
.log-detail-overlay { z-index: 12; }
.log-detail-dialog { width: min(700px, 100%); }
.log-detail-body { padding: 12px 15px; overflow: auto; }
.log-detail-body dl { margin: 0; display: grid; grid-template-columns: 140px minmax(0, 1fr); gap: 0; font-size: 11px; }
.log-detail-body dt, .log-detail-body dd { margin: 0; padding: 8px 4px; border-bottom: 1px solid var(--border); }
.log-detail-body dt { color: var(--text-muted); }
.log-detail-body dd { min-width: 0; overflow-wrap: anywhere; }
.log-detail-body dd code { color: var(--text); }
.toast { position: fixed; z-index: 20; right: 16px; bottom: 16px; max-width: min(420px, calc(100vw - 32px)); padding: 9px 12px; border: 1px solid var(--border-strong); border-radius: 7px; color: var(--text); background: var(--surface-container); box-shadow: var(--shadow); font-size: 11px; }
.toast.error { color: var(--danger); }
@media (max-width: 620px) { .instance-head { align-items: flex-start; flex-direction: column; } .row-actions { justify-content: flex-start; } .link-row { grid-template-columns: 58px minmax(0, 1fr) auto; } .link-row .button:last-child { grid-column: 3; } .grid { grid-template-columns: 1fr; } }
</style>
