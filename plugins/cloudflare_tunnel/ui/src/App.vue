<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
import { onMounted, onUnmounted, ref } from 'vue'
import { onPluginLifecycle } from '../../../ui-sdk/runtime'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import {
  cfClearLogs,
  cfCopyText,
  cfGetLogs,
  cfGetTunnel,
  cfOpenUrl,
  cfResetTunnel,
  cfRestartTunnel,
  cfSaveFileShare,
  cfSaveServices,
  cfScanCredentials,
  cfSetupTunnel,
  cfStartFileShare,
  cfStopFileShare,
  cfStartTunnel,
  cfStopTunnel,
  cfToggleService,
  type CredentialScanItem,
  type ServiceRule,
  type TunnelDetailView
} from './api'

// ---------- 状态 ----------
const detail = ref<TunnelDetailView | null>(null)
const configured = ref(false)
const actionLoading = ref(false)
const fileShareSaving = ref(false)
const fileShareToggleLoading = ref(false)
const fileShareDirty = ref(false)
const fileShareForm = ref({
  directory: '',
  hostname: '',
  port: 12026,
  lan_enabled: false,
  allowed_ips: '',
  upload_enabled: false,
  upload_directory: '',
  max_upload_mb: 100,
  max_total_upload_mb: 2048
})

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

// ---------- 数据刷新 ----------
let refreshTimer = 0
let pluginActive = false
let setupPrompted = false
let unlistenLifecycle: (() => void) | null = null

async function loadData(silent = false) {
  try {
    const res = await cfGetTunnel()
    if (res) {
      detail.value = res
      configured.value = true
      if (!fileShareDirty.value) {
        const share = res.tunnel.file_share
        fileShareForm.value = {
          directory: share?.directory ?? '',
          hostname: share?.hostname ?? '',
          port: share?.port || 12026,
          lan_enabled: share?.lan_enabled ?? false,
          allowed_ips: share?.allowed_ips?.join('\n') ?? '',
          upload_enabled: share?.upload_enabled ?? false,
          upload_directory: share?.upload_directory ?? '',
          max_upload_mb: share?.max_upload_mb ?? 100,
          max_total_upload_mb: share?.max_total_upload_mb ?? 2048
        }
      }
    } else {
      detail.value = null
      configured.value = false
      fileShareDirty.value = false
    }
  } catch (e: any) {
    if (!silent) showToast(String(e?.message || e), 'error')
  }
}

// ---------- 启停与控制 ----------
async function startTunnel() {
  if (actionLoading.value) return
  actionLoading.value = true
  try {
    await cfStartTunnel()
    showToast('已启动 Tunnel', 'success')
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  } finally {
    actionLoading.value = false
  }
}

async function stopTunnel() {
  if (actionLoading.value) return
  actionLoading.value = true
  try {
    await cfStopTunnel()
    showToast('已停止 Tunnel', 'info')
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  } finally {
    actionLoading.value = false
  }
}

async function restartTunnel() {
  if (actionLoading.value) return
  actionLoading.value = true
  try {
    await cfRestartTunnel()
    showToast('已重启生效', 'success')
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  } finally {
    actionLoading.value = false
  }
}

// ---------- 暂停 / 启用 子域名 ----------
async function toggleService(serviceId: string, currentEnabled: boolean) {
  const next = !currentEnabled
  try {
    await cfToggleService(serviceId, next)
    showToast(next ? '已启用子域名' : '已暂停子域名 (访问返回 404)', 'info')
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

// ---------- 添加 / 编辑服务 ----------
const showServiceDialog = ref(false)
const editingServiceId = ref<string | null>(null)
const serviceForm = ref({
  name: '',
  hostname: '',
  service: '',
  enabled: true
})

function openAddService() {
  editingServiceId.value = null
  serviceForm.value = {
    name: '',
    hostname: '',
    service: '127.0.0.1:',
    enabled: true
  }
  showServiceDialog.value = true
}

function openEditService(s: ServiceRule) {
  editingServiceId.value = s.id
  serviceForm.value = {
    name: s.name,
    hostname: s.hostname,
    service: s.service,
    enabled: s.enabled
  }
  showServiceDialog.value = true
}

async function submitService() {
  if (!detail.value) return
  const name = serviceForm.value.name.trim() || serviceForm.value.hostname.trim()
  const hostname = serviceForm.value.hostname.trim().replace(/^\.+|\.+$/g, '')
  const service = serviceForm.value.service.trim()

  if (!hostname) {
    showToast('请输入子域名前缀', 'error')
    return
  }
  if (!service) {
    showToast('请输入本地端口或地址', 'error')
    return
  }

  const services = [...detail.value.tunnel.services]
  if (editingServiceId.value) {
    const idx = services.findIndex((x) => x.id === editingServiceId.value)
    if (idx !== -1) {
      services[idx] = {
        id: editingServiceId.value,
        name,
        hostname,
        service,
        enabled: serviceForm.value.enabled
      }
    }
  } else {
    services.push({
      id: '',
      name,
      hostname,
      service,
      enabled: true
    })
  }

  try {
    await cfSaveServices(services)
    showToast('服务已保存', 'success')
    showServiceDialog.value = false
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

// 删除确认
const confirmDelId = ref<string | null>(null)
let delTimer = 0

async function deleteService(id: string) {
  if (confirmDelId.value !== id) {
    confirmDelId.value = id
    window.clearTimeout(delTimer)
    delTimer = window.setTimeout(() => {
      confirmDelId.value = null
    }, 2500)
    return
  }
  confirmDelId.value = null

  if (!detail.value) return
  const services = detail.value.tunnel.services.filter((x) => x.id !== id)
  try {
    await cfSaveServices(services)
    showToast('已删除服务', 'success')
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

async function saveFileShare(rotateToken = false): Promise<boolean> {
  if (fileShareSaving.value) return false
  fileShareSaving.value = true
  try {
    await cfSaveFileShare(fileShareForm.value, rotateToken)
    fileShareDirty.value = false
    const message = rotateToken
      ? '分享链接已重置，旧链接已失效'
      : '文件共享设置已保存'
    showToast(message, 'success')
    await loadData(true)
    return true
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
    return false
  } finally {
    fileShareSaving.value = false
  }
}

async function toggleFileShare() {
  if (fileShareToggleLoading.value || fileShareSaving.value) return
  fileShareToggleLoading.value = true
  try {
    const running = detail.value?.file_share?.local_running ?? false
    if (running) {
      await cfStopFileShare()
      showToast('文件服务和关联转发已关闭', 'success')
    } else {
      if (fileShareDirty.value && !(await saveFileShare())) return
      await cfStartFileShare()
      showToast(
        fileShareForm.value.hostname.trim()
          ? '文件服务已启动，Cloudflare 转发规则已启用'
          : '文件服务已启动',
        'success'
      )
    }
    await loadData(true)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
    await loadData(true)
  } finally {
    fileShareToggleLoading.value = false
  }
}

const confirmRotateShare = ref(false)
let rotateShareTimer = 0

function rotateShareLink() {
  if (!confirmRotateShare.value) {
    confirmRotateShare.value = true
    window.clearTimeout(rotateShareTimer)
    rotateShareTimer = window.setTimeout(() => {
      confirmRotateShare.value = false
    }, 3000)
    return
  }
  confirmRotateShare.value = false
  void saveFileShare(true)
}

// ---------- 快捷操作 ----------
async function openUrl(url: string) {
  try {
    await cfOpenUrl(url)
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

async function copyText(text: string, label = '内容') {
  try {
    await cfCopyText(text)
    showToast(`已复制${label}`, 'success')
  } catch (e: any) {
    showToast(`复制失败：${String(e?.message || e)}`, 'error')
  }
}

function copyShareLink() {
  const url = detail.value?.file_share?.local_url
  if (url) void copyText(url, '分享链接')
}

function openShareLink() {
  const url = detail.value?.file_share?.local_url
  if (url) void openUrl(url)
}

// ---------- 配置向导 ----------
const setupForm = ref({
  name: 'local',
  domain: 'zikuosu.ccwu.cc',
  tunnel_id: '',
  credentials_path: ''
})
const scannedCredentials = ref<CredentialScanItem[]>([])

async function openSetup() {
  setupForm.value = {
    name: detail.value?.tunnel.name || 'local',
    domain: detail.value?.tunnel.domain || 'zikuosu.ccwu.cc',
    tunnel_id: detail.value?.tunnel.tunnel_id || '',
    credentials_path: ''
  }
  try {
    scannedCredentials.value = await cfScanCredentials()
    if (scannedCredentials.value.length > 0 && !setupForm.value.tunnel_id) {
      setupForm.value.tunnel_id = scannedCredentials.value[0].tunnel_id
      setupForm.value.credentials_path = scannedCredentials.value[0].path
    }
  } catch {
    scannedCredentials.value = []
  }
}

function selectScanned(c: CredentialScanItem) {
  setupForm.value.tunnel_id = c.tunnel_id
  setupForm.value.credentials_path = c.path
}

async function submitSetup() {
  const name = setupForm.value.name.trim()
  const domain = setupForm.value.domain.trim()
  const tid = setupForm.value.tunnel_id.trim()

  if (!name || !domain || !tid) {
    showToast('请完整填写名称、根域名与 Tunnel ID', 'error')
    return
  }

  actionLoading.value = true
  try {
    await cfSetupTunnel(name, domain, tid, setupForm.value.credentials_path || undefined)
    showToast('配置成功', 'success')
    await loadData()
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  } finally {
    actionLoading.value = false
  }
}

// 重置 Tunnel
const confirmReset = ref(false)
let resetTimer = 0

async function handleReset() {
  if (!confirmReset.value) {
    confirmReset.value = true
    window.clearTimeout(resetTimer)
    resetTimer = window.setTimeout(() => {
      confirmReset.value = false
    }, 2500)
    return
  }
  confirmReset.value = false
  try {
    await cfResetTunnel()
    showToast('已重置配置', 'info')
    await loadData()
    await openSetup()
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

// ---------- 实时控制台 ----------
const showLogDrawer = ref(false)
const logLines = ref<string[]>([])
let logTimer = 0

async function fetchLogs() {
  try {
    logLines.value = await cfGetLogs(50)
  } catch {
    // 静默
  }
}

function openLogs() {
  showLogDrawer.value = true
  syncPollingTimers()
}

function closeLogs() {
  showLogDrawer.value = false
  syncPollingTimers()
}

function syncPollingTimers() {
  window.clearInterval(refreshTimer)
  window.clearInterval(logTimer)
  refreshTimer = 0
  logTimer = 0
  if (!pluginActive) return
  if (showLogDrawer.value) {
    void fetchLogs()
    logTimer = window.setInterval(fetchLogs, 1000)
    return
  }
  refreshTimer = window.setInterval(() => {
    if (!document.hidden && !showServiceDialog.value) void loadData(true)
  }, 4000)
}

async function activatePlugin() {
  const firstActivation = !setupPrompted
  setupPrompted = true
  await loadData(!firstActivation)
  if (firstActivation && pluginActive && !configured.value) {
    await openSetup()
  }
}

function setPluginActive(active: boolean) {
  pluginActive = active
  syncPollingTimers()
  if (active) void activatePlugin()
}

async function clearLogs() {
  try {
    await cfClearLogs()
    logLines.value = []
    showToast('已清空控制台', 'info')
  } catch (e: any) {
    showToast(String(e?.message || e), 'error')
  }
}

// ---------- 生命周期 ----------
onMounted(() => {
  unlistenLifecycle = onPluginLifecycle(setPluginActive)
})

onUnmounted(() => {
  window.clearInterval(refreshTimer)
  window.clearInterval(logTimer)
  window.clearTimeout(rotateShareTimer)
  unlistenLifecycle?.()
})
</script>

<template>
  <div class="cf">
    <!-- 顶部工具栏 -->
    <header class="toolbar">
      <div class="tb-left">
        <span class="dot" :class="{ on: detail?.status === 'running' }" />
        <span class="tb-title">Cloudflare Tunnel</span>
        <span v-if="configured && detail" class="chip status" :class="{ live: detail.status === 'running' }">
          {{ detail.status === 'running' ? `运行中 (${detail.conns})` : '已停止' }}
        </span>
      </div>

      <div class="tb-right">
        <template v-if="configured && detail">
          <Button variant="outline" size="sm" class="btn sm" title="实时日志监控" @click="openLogs">
            <Icon name="terminal" :size="12" /> 控制台
          </Button>

          <Button variant="outline" size="sm"
            v-if="detail.status === 'running'"
            class="btn sm stop"
            :disabled="actionLoading"
            @click="stopTunnel"
          >
            <Icon name="square" :size="11" /> 停止
          </Button>
          <Button variant="default" size="sm"
            v-else
            class="btn sm primary"
            :disabled="actionLoading"
            @click="startTunnel"
          >
            <Icon name="play" :size="11" /> 启动
          </Button>

          <Button variant="outline" size="sm"
            v-if="detail.status === 'running'"
            class="icon-btn"
            title="重启以重载配置"
            :disabled="actionLoading"
            @click="restartTunnel"
          >
            <Icon name="refresh" :size="13" />
          </Button>
        </template>
      </div>
    </header>

    <!-- 主体区域 -->
    <main class="body">
      <!-- 引导配置 -->
      <div v-if="!configured || !detail" class="setup-wrap">
        <div class="setup-panel">
          <div class="setup-head">
            <Icon name="cloud" :size="28" class="setup-icon" />
            <div class="setup-h2">配置本机 Tunnel</div>
            <div class="setup-p">
              配置一次即可。由本地 <code>config.yaml</code> 自主管理路由与启停。
            </div>
          </div>

          <div class="form">
            <div v-if="scannedCredentials.length > 0" class="field">
              <span class="label">发现的本机凭证</span>
              <div class="scanned-group">
                <div
                  v-for="c in scannedCredentials"
                  :key="c.tunnel_id"
                  class="scanned-row"
                  :class="{ active: setupForm.tunnel_id === c.tunnel_id }"
                  @click="selectScanned(c)"
                >
                  <Icon name="check" :size="12" class="chk" />
                  <span class="mono">{{ c.tunnel_id }}</span>
                </div>
              </div>
            </div>

            <div class="grid2">
              <label class="field">
                <span class="label">Tunnel 名称</span>
                <input v-model="setupForm.name" type="text" placeholder="local" />
              </label>
              <label class="field">
                <span class="label">根域名</span>
                <input v-model="setupForm.domain" type="text" placeholder="zikuosu.ccwu.cc" />
              </label>
            </div>

            <label class="field">
              <span class="label">Tunnel ID (UUID)</span>
              <input v-model="setupForm.tunnel_id" type="text" class="mono" placeholder="5cc43274-..." />
            </label>

            <Button variant="default" size="sm" class="btn primary full" :disabled="actionLoading" @click="submitSetup">
              {{ actionLoading ? '正在配置...' : '保存并进入' }}
            </Button>
          </div>
        </div>
      </div>

      <!-- 服务列表主视图 -->
      <template v-else>
        <!-- 基础域名与 DNS 提示横条 -->
        <div class="meta-bar">
          <div class="meta-left">
            <span class="domain-tag">{{ detail.tunnel.domain }}</span>
            <span class="dns-hint">
              CNAME 目标: <code class="mono">{{ detail.tunnel.tunnel_id }}.cfargotunnel.com</code>
            </span>
          </div>
          <div class="meta-right">
            <Button variant="outline" size="sm"
              class="tiny-btn"
              @click="copyText(`${detail.tunnel.tunnel_id}.cfargotunnel.com`, 'CNAME 目标')"
            >
              复制目标
            </Button>
            <Button variant="destructive" size="sm"
              class="tiny-btn danger"
              :class="{ active: confirmReset }"
              @click="handleReset"
            >
              {{ confirmReset ? '确认重置' : '重设' }}
            </Button>
          </div>
        </div>

        <section class="share-panel share-dashboard">
          <div class="share-hero">
            <div class="share-hero-copy">
              <span class="share-eyebrow">LOCAL FILE SHARING</span>
              <div class="share-header">
                <div>
                  <div class="sb-title">本机文件共享</div>
                  <div class="share-description">浏览和下载本机文件，也可按需开放上传或远程访问。</div>
                </div>
                <span class="chip" :class="detail.file_share?.local_running ? 'live' : 'paused'">
                  <span class="dot-status" :class="detail.file_share?.local_running ? 'online' : 'paused'" />
                  {{ detail.file_share?.local_running ? '运行中' : '已停止' }}
                </span>
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              :disabled="fileShareToggleLoading || fileShareSaving"
              @click="toggleFileShare"
            >
              {{ fileShareToggleLoading ? '正在处理…' : (detail.file_share?.local_running ? '停止服务' : '启动服务') }}
            </Button>
          </div>

          <div v-if="detail.file_share" class="share-overview">
            <div class="share-stat">
              <span>本机端口</span>
              <strong>{{ detail.file_share.port }}</strong>
              <small>{{ detail.file_share.local_running ? (detail.file_share.lan_enabled ? '监听 0.0.0.0' : '仅监听本机') : '服务停止时不监听' }}</small>
            </div>
            <div class="share-stat">
              <span>局域网访问</span>
              <strong>{{ fileShareForm.lan_enabled ? '已允许' : '仅本机' }}</strong>
              <small>{{ fileShareForm.lan_enabled ? (detail.file_share.lan_host || '可手动使用本机 IPv4') : '开启后使用局域网 IP 分享' }}</small>
            </div>
            <div class="share-stat">
              <span>共享内容</span>
              <strong>{{ fileShareForm.upload_enabled ? '浏览 + 上传' : '浏览 + 下载' }}</strong>
              <small>{{ fileShareForm.upload_enabled ? '同一链接提供两种操作' : '上传功能已关闭' }}</small>
            </div>
          </div>

          <div class="share-section">
            <div class="share-section-heading">
              <span class="section-index">01</span>
              <div><strong>服务设置</strong><small>选择目录、端口和外部访问域名</small></div>
            </div>
            <div class="share-form">
              <label class="field field-wide">
                <span class="label">共享文件夹路径</span>
                <input
                  v-model="fileShareForm.directory"
                  type="text"
                  class="mono"
                  placeholder="C:\Users\you\Public"
                  @input="fileShareDirty = true"
                />
              </label>
              <label class="field">
                <span class="label">本机与局域网共用端口</span>
                <input
                  v-model.number="fileShareForm.port"
                  type="number"
                  min="1024"
                  max="65535"
                  @input="fileShareDirty = true"
                />
                <span class="field-hint">端口占用时，可改为 1024–65535 之间的其他端口。</span>
              </label>
              <label class="field">
                <span class="label">Cloudflare 转发域名 <span class="optional-tag">可选</span></span>
                <input
                  v-model="fileShareForm.hostname"
                  type="text"
                  class="mono"
                  placeholder="file.example.com"
                  @input="fileShareDirty = true"
                />
                <span class="field-hint">仅填写当前 Tunnel 根域名下的完整域名。</span>
              </label>
            </div>
            <div class="route-preview">
              <span class="route-led" />
              <template v-if="fileShareForm.hostname.trim()">
                启动时关联 <code>{{ fileShareForm.hostname }}</code> → <code>127.0.0.1:{{ fileShareForm.port }}</code>；关闭文件服务时同步停用转发。
              </template>
              <template v-else>未配置 Tunnel 域名；文件服务仍可通过本机或局域网访问。</template>
            </div>
          </div>

          <div class="share-section share-access-section">
            <div class="share-section-heading">
              <span class="section-index">02</span>
              <div><strong>访问控制</strong><small>局域网监听与来源地址限制</small></div>
            </div>
            <label class="option-card">
              <input v-model="fileShareForm.lan_enabled" type="checkbox" @change="fileShareDirty = true" />
              <span class="option-check" />
              <span class="option-copy"><strong>允许局域网访问</strong><small>监听 0.0.0.0，并使用本机局域网 IPv4 地址分享</small></span>
              <code v-if="fileShareForm.lan_enabled" class="option-port">:{{ fileShareForm.port }}</code>
            </label>
            <div v-if="fileShareForm.lan_enabled" class="lan-address-note">
              <span class="lan-address-icon">↗</span>
              <span v-if="detail.file_share?.lan_host">当前局域网地址：<code>http://{{ detail.file_share.lan_host }}:{{ detail.file_share.port }}</code></span>
              <span v-else>尚未自动检测到局域网 IP；服务仍会监听所有网卡，可手动使用本机 IPv4 和端口 {{ detail.file_share?.port }}。</span>
            </div>
            <label class="field allowlist-field">
              <span class="label">IP 白名单 <span class="optional-tag">每行一个 IP 或 CIDR，留空不限</span></span>
              <textarea
                v-model="fileShareForm.allowed_ips"
                rows="3"
                class="mono allowlist-input"
                placeholder="192.168.1.20&#10;192.168.1.0/24&#10;2001:db8::/48"
                @input="fileShareDirty = true"
              />
              <span class="field-hint">Tunnel 请求按访客 IP 校验；局域网请求按连接来源 IP 校验。</span>
            </label>
          </div>

          <div class="share-section">
            <div class="share-section-heading">
              <span class="section-index">03</span>
              <div><strong>上传选项</strong><small>上传仅写入单独的收件目录</small></div>
            </div>
            <label class="option-card">
              <input v-model="fileShareForm.upload_enabled" type="checkbox" @change="fileShareDirty = true" />
              <span class="option-check" />
              <span class="option-copy"><strong>允许通过共享页上传文件</strong><small>浏览列表和上传操作使用同一个分享链接</small></span>
              <span class="option-state">{{ fileShareForm.upload_enabled ? '开启' : '关闭' }}</span>
            </label>
            <div v-if="fileShareForm.upload_enabled" class="share-form upload-form upload-settings">
              <label class="field field-wide">
                <span class="label">收件目录</span>
                <input
                  v-model="fileShareForm.upload_directory"
                  type="text"
                  class="mono"
                  placeholder="留空则自动创建共享目录旁的同级上传目录"
                  @input="fileShareDirty = true"
                />
                <span class="field-hint">必须与共享目录分开；上传文件不会出现在只读文件列表。</span>
              </label>
              <label class="field">
                <span class="label">单文件上限（MB）</span>
                <input v-model.number="fileShareForm.max_upload_mb" type="number" min="1" max="1024" @input="fileShareDirty = true" />
              </label>
              <label class="field">
                <span class="label">收件目录总量（MB）</span>
                <input v-model.number="fileShareForm.max_total_upload_mb" type="number" min="1" max="8192" @input="fileShareDirty = true" />
              </label>
            </div>
            <p v-if="fileShareForm.upload_enabled" class="inline-note">通过 Tunnel 上传还受 Cloudflare 区域的单次请求大小限制。</p>
          </div>

          <div class="share-section links-section">
            <div class="share-section-heading">
              <span class="section-index">04</span>
              <div><strong>分享链接</strong><small>{{ fileShareForm.upload_enabled ? '同一个链接可浏览、下载和上传' : '此链接可浏览和下载共享文件' }}</small></div>
            </div>
            <div v-if="detail.file_share?.local_url || detail.file_share?.lan_share_url || detail.file_share?.cloudflare_share_url" class="share-links-grid">
              <article v-if="detail.file_share?.local_url" class="share-link-card">
                <div class="link-card-heading"><span class="link-icon local-icon">⌂</span><div><strong>本机</strong><small>仅此电脑可访问</small></div></div>
                <code class="mono">{{ detail.file_share.local_url }}</code>
                <div class="share-link-actions">
                  <Button v-if="detail.file_share.local_running" variant="outline" size="sm" @click="openShareLink">打开</Button>
                  <Button variant="outline" size="sm" @click="copyShareLink">复制链接</Button>
                </div>
              </article>
              <article v-if="detail.file_share?.lan_share_url" class="share-link-card">
                <div class="link-card-heading"><span class="link-icon lan-icon">⌁</span><div><strong>局域网</strong><small>{{ detail.file_share.lan_host }}</small></div></div>
                <code class="mono">{{ detail.file_share.lan_share_url }}</code>
                <div class="share-link-actions"><Button variant="outline" size="sm" @click="copyText(detail.file_share.lan_share_url, '局域网链接')">复制链接</Button></div>
              </article>
              <article v-if="detail.file_share?.cloudflare_share_url" class="share-link-card">
                <div class="link-card-heading"><span class="link-icon tunnel-icon">↗</span><div><strong>Cloudflare Tunnel</strong><small>公网域名</small></div></div>
                <code class="mono">{{ detail.file_share.cloudflare_share_url }}</code>
                <div class="share-link-actions"><Button variant="outline" size="sm" @click="copyText(detail.file_share.cloudflare_share_url, 'Cloudflare 链接')">复制链接</Button></div>
              </article>
            </div>
            <div v-else class="links-empty"><span>↗</span><div><strong>启动文件服务后生成分享链接</strong><small>配置了局域网或 Tunnel 域名时，会在这里显示对应链接。</small></div></div>
            <div class="link-security-row">
              <span>分享链接包含访问令牌，请仅发给可信任的人。</span>
              <Button variant="outline" size="sm" @click="rotateShareLink">
                {{ confirmRotateShare ? '再次点击确认重置' : '重置所有分享链接' }}
              </Button>
            </div>
          </div>

          <div class="share-footer">
            <p class="share-warning">文件服务可独立启停；停止时会同步停用关联转发。Tunnel 启停不会影响本机和局域网文件服务。</p>
            <Button variant="default" size="sm" :disabled="fileShareSaving || fileShareToggleLoading" @click="saveFileShare()">
              {{ fileShareSaving ? '正在保存…' : (fileShareDirty ? '保存设置' : '设置已保存') }}
            </Button>
          </div>
        </section>

        <div class="services-box">
          <div class="services-bar">
            <div class="sb-title">
              <span>本地服务</span>
              <span class="chip count">
                {{ detail.services.filter(s => s.enabled).length }} / {{ detail.services.length }}
              </span>
            </div>
            <Button variant="default" size="sm" class="btn sm primary" @click="openAddService">
              <Icon name="plus" :size="12" /> 添加服务
            </Button>
          </div>

          <!-- 空提示 -->
          <div v-if="detail.services.length === 0" class="empty">
            <div class="empty-txt">暂无服务路由</div>
            <div class="empty-sub">点击右上角“添加服务”，即可将本机端口暴露为子域名。</div>
          </div>

          <!-- 服务卡片列表 -->
          <div v-else class="list">
            <div
              v-for="s in detail.services"
              :key="s.id"
              class="card"
              :class="{ off: !s.enabled }"
            >
              <!-- 状态呼吸灯：绿色=本地在线且启用，灰色=停用或未启动 -->
              <span
                class="dot-status"
                :class="{
                  online: s.enabled && s.local_online,
                  paused: !s.enabled,
                  offline: s.enabled && !s.local_online
                }"
                :title="!s.enabled ? '已暂停路由' : (s.local_online ? '正常运行中' : '本地端口未启动')"
              />

              <div class="info">
                <div class="row1">
                  <span class="name">{{ s.name }}</span>
                  <span class="subdomain-tag mono">{{ s.hostname }}</span>
                  <span class="port-tag mono">→ {{ s.service }}</span>
                  <span v-if="!s.enabled" class="chip paused">已暂停</span>
                  <span v-else-if="!s.local_online" class="chip offline">本地未启动</span>
                </div>
                <div class="row2">
                  <a
                    href="javascript:void(0)"
                    class="url mono"
                    :class="{ muted: !s.enabled }"
                    @click="openUrl(s.full_url)"
                  >
                    {{ s.full_url }}
                  </a>
                </div>
              </div>

              <!-- 右侧紧凑操作区 -->
              <div class="actions">
                <Button variant="outline" size="sm"
                  class="action-btn"
                  :class="s.enabled ? 'btn-pause' : 'btn-resume'"
                  :title="s.enabled ? '暂停此子域名（流量返回 404）' : '启用此子域名'"
                  @click="toggleService(s.id, s.enabled)"
                >
                  {{ s.enabled ? '暂停' : '启用' }}
                </Button>

                <Button variant="outline" size="sm"
                  class="action-btn"
                  title="在系统浏览器中打开"
                  @click="openUrl(s.full_url)"
                >
                  打开
                </Button>

                <Button variant="outline" size="sm"
                  class="action-btn"
                  title="复制公网链接"
                  @click="copyText(s.full_url, '公网链接')"
                >
                  复制
                </Button>

                <Button variant="outline" size="sm"
                  class="icon-btn"
                  title="编辑"
                  @click="openEditService(s)"
                >
                  <Icon name="pencil" :size="13" />
                </Button>

                <Button variant="destructive" size="sm"
                  v-if="confirmDelId !== s.id"
                  class="icon-btn danger"
                  title="删除"
                  @click="deleteService(s.id)"
                >
                  <Icon name="trash" :size="13" />
                </Button>
                <Button variant="outline" size="sm"
                  v-else
                  class="action-btn del-confirm"
                  @click="deleteService(s.id)"
                >
                  确认删除
                </Button>
              </div>
            </div>
          </div>
        </div>
      </template>
    </main>

    <!-- 模态框：添加 / 编辑服务 -->
    <Teleport to="body">
      <div v-if="showServiceDialog" class="mask" @click.self="showServiceDialog = false">
        <div class="dialog">
          <div class="dialog-head">
            <h3>{{ editingServiceId ? '编辑服务' : '添加本地服务' }}</h3>
            <Button variant="outline" size="sm" class="icon-btn" @click="showServiceDialog = false">
              <Icon name="x" :size="14" />
            </Button>
          </div>

          <div class="dialog-body">
            <label class="field">
              <span class="label">服务名称</span>
              <input v-model="serviceForm.name" type="text" placeholder="例如：API / Web" />
            </label>

            <label class="field">
              <span class="label">Hostname 子域名前缀 *</span>
              <div class="suffix-input">
                <input v-model="serviceForm.hostname" type="text" placeholder="tunnel6694" />
                <span class="suffix">.{{ detail?.tunnel.domain || '' }}</span>
              </div>
              <span class="field-tip">
                访问地址：https://{{ serviceForm.hostname || 'xxx' }}.{{ detail?.tunnel.domain || '' }}
              </span>
            </label>

            <label class="field">
              <span class="label">本地地址 / 端口 *</span>
              <input v-model="serviceForm.service" type="text" class="mono" placeholder="6694 或 127.0.0.1:6694" />
            </label>
          </div>

          <div class="dialog-foot">
            <Button variant="outline" size="sm" class="btn sm" @click="showServiceDialog = false">取消</Button>
            <Button variant="default" size="sm" class="btn sm primary" @click="submitService">保存生效</Button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 模态框：实时控制台 -->
    <Teleport to="body">
      <div v-if="showLogDrawer" class="mask" @click.self="closeLogs">
        <div class="dialog log-dialog">
          <div class="dialog-head">
            <div class="log-title">
              <span class="dot-mini live" />
              <h3>实时控制台 (Live)</h3>
              <span class="sub-tip">即时查看输出，退出即停止</span>
            </div>
            <div class="head-ops">
              <Button variant="outline" size="sm" class="btn sm" @click="clearLogs">清空</Button>
              <Button variant="outline" size="sm" class="icon-btn" @click="closeLogs">
                <Icon name="x" :size="14" />
              </Button>
            </div>
          </div>

          <div class="log-box">
            <div v-if="logLines.length === 0" class="log-none">
              正在监听输出或暂无新日志...
            </div>
            <div v-for="(line, idx) in logLines" :key="idx" class="log-row">
              {{ line }}
            </div>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Toast 提示 -->
    <Toast v-if="toast" :message="toast" :kind="toastKind" />
  </div>
</template>

<style scoped>
:global(html[data-sideweave_embedded] body) {
  background: transparent;
}

.cf {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: color-mix(in srgb, var(--bg) calc(var(--detail-panel-opacity, 1) * 100%), transparent);
  color: var(--text);
  font-family: var(--font-ui, system-ui, -apple-system, sans-serif);
  user-select: none;
}

/* 顶部工具栏 */
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  border-bottom: 1px solid var(--border);
  flex: none;
}

.tb-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tb-title {
  font-weight: 600;
  font-size: 13.5px;
}

.tb-right {
  display: flex;
  align-items: center;
  gap: 6px;
}

/* 状态指示点 */
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--border-strong);
  flex: none;
}

.dot.on {
  background: var(--success, #10b981);
  box-shadow: 0 0 6px color-mix(in srgb, var(--success, #10b981) 45%, transparent);
}

.dot-mini {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
  flex: none;
}

.dot-mini.live {
  background: var(--success, #10b981);
}

/* Chip 徽章 */
.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: 999px;
  font-size: 11px;
  white-space: nowrap;
}

.chip.status {
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 11px;
}

.chip.status.live {
  color: var(--success, #10b981);
  border-color: color-mix(in srgb, var(--success, #10b981) 40%, transparent);
  background: color-mix(in srgb, var(--success, #10b981) 10%, transparent);
}

.chip.count {
  border: 1px solid var(--border);
  color: var(--text-muted);
}

.chip.paused {
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text-muted);
  border: 1px solid var(--border);
}

.chip.offline {
  background: color-mix(in srgb, var(--warning, #f59e0b) 15%, transparent);
  color: var(--warning, #f59e0b);
}

/* 容器布局 */
.body {
  flex: 1;
  padding: 12px 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* Meta 信息横条 */
.meta-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  border-radius: 8px;
  font-size: 11.5px;
  gap: 10px;
}

.meta-left {
  display: flex;
  align-items: center;
  gap: 8px;
  overflow: hidden;
}

.domain-tag {
  font-weight: 600;
  color: var(--text);
  font-family: monospace;
}

.dns-hint {
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dns-hint code {
  color: var(--primary);
}

.meta-right {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.tiny-btn {
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 10.5px;
  padding: 2px 7px;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s;
}

.tiny-btn:hover {
  color: var(--text);
  border-color: var(--border-strong);
}

.tiny-btn.danger.active {
  background: color-mix(in srgb, var(--danger) calc(var(--component-opacity, 1) * 100%), transparent);
  color: #fff;
  border-color: var(--danger);
}

/* 服务列表 */
.share-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
}

.share-header,
.share-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
}

.share-description {
  margin-top: 3px;
  color: var(--text-muted);
  font-size: 11px;
}

.local-port {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 20px;
  color: var(--text-muted);
  font-size: 11px;
}

.local-port code {
  color: var(--text-primary, var(--text));
  font-family: var(--font-mono, monospace);
}

.local-port-note {
  opacity: 0.75;
}

.share-header .chip.live {
  color: var(--success, #10b981);
  border: 1px solid color-mix(in srgb, var(--success, #10b981) 40%, transparent);
  background: color-mix(in srgb, var(--success, #10b981) 10%, transparent);
}

.share-enable {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  width: fit-content;
  font-size: 12px;
  cursor: pointer;
}

.share-enable input {
  width: 14px;
  height: 14px;
  margin: 0;
  padding: 0;
  accent-color: var(--primary);
}

.share-form {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: 10px;
}

.upload-form {
  align-items: start;
}

.allowlist-input {
  width: 100%;
  box-sizing: border-box;
  resize: vertical;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  font-size: 11.5px;
}

.allowlist-input:focus {
  outline: none;
  border-color: var(--primary);
}

.field-hint {
  color: var(--text-muted);
  font-size: 10.5px;
  line-height: 1.4;
}

.share-link {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.share-link code {
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--primary);
  font-size: 11px;
  overflow-wrap: anywhere;
  user-select: all;
}

.share-link-actions {
  display: flex;
  gap: 6px;
}

.share-warning {
  margin: 0;
  color: var(--warning, #f59e0b);
  font-size: 10.5px;
  line-height: 1.5;
}

.services-box {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.services-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.sb-title {
  font-size: 13px;
  font-weight: 600;
  display: flex;
  align-items: center;
  gap: 6px;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.card {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  transition: border-color 0.15s ease;
}

.card:hover {
  border-color: var(--border-strong, var(--primary));
}

.card.off {
  opacity: 0.65;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
}

.dot-status {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: none;
  background: var(--border-strong);
}

.dot-status.online {
  background: var(--success, #10b981);
}

.dot-status.offline {
  background: var(--warning, #f59e0b);
}

.dot-status.paused {
  background: var(--text-muted);
  opacity: 0.5;
}

.info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.row1 {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.name {
  font-weight: 600;
  font-size: 12.5px;
  white-space: nowrap;
}

.subdomain-tag {
  font-size: 11px;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--primary);
  padding: 1px 5px;
  border-radius: 3px;
}

.port-tag {
  font-size: 11px;
  color: var(--text-muted);
}

.row2 {
  display: flex;
}

.url {
  font-size: 11.5px;
  color: var(--text-muted);
  text-decoration: none;
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.url:hover {
  color: var(--primary);
  text-decoration: underline;
}

.url.muted {
  text-decoration: line-through;
  opacity: 0.7;
}

/* 操作区域 */
.actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

.action-btn {
  padding: 3px 8px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 5px;
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  cursor: pointer;
  transition: all 0.15s ease;
  white-space: nowrap;
}

.action-btn:hover {
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border-color: var(--border-strong);
}

.btn-pause {
  color: var(--warning, #f59e0b);
  border-color: color-mix(in srgb, var(--warning, #f59e0b) 40%, transparent);
}

.btn-pause:hover {
  background: color-mix(in srgb, var(--warning, #f59e0b) 12%, transparent);
}

.btn-resume {
  color: var(--success, #10b981);
  border-color: color-mix(in srgb, var(--success, #10b981) 40%, transparent);
}

.btn-resume:hover {
  background: color-mix(in srgb, var(--success, #10b981) 12%, transparent);
}

.del-confirm {
  color: var(--danger);
  border-color: var(--danger);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  flex: none;
  transition: all 0.15s;
}

.icon-btn:hover {
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
}

.icon-btn.danger:hover {
  color: var(--danger);
}

/* 空列表 */
.empty {
  padding: 30px 16px;
  text-align: center;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px dashed var(--border);
  border-radius: 8px;
  color: var(--text-muted);
}

.empty-txt {
  font-size: 12.5px;
  font-weight: 500;
}

.empty-sub {
  font-size: 11.5px;
  margin-top: 4px;
  opacity: 0.8;
}

/* 通用按钮 */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  font-size: 12px;
  font-weight: 500;
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  cursor: pointer;
  outline: none;
  transition: all 0.15s ease;
  white-space: nowrap;
}

.btn:hover:not(:disabled) {
  border-color: var(--border-strong);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn.primary {
  background: color-mix(in srgb, var(--primary) calc(var(--component-opacity, 1) * 100%), transparent);
  border-color: var(--primary);
  color: var(--primary-foreground);
}

.btn.primary:hover:not(:disabled) {
  filter: brightness(1.08);
}

.btn.stop {
  background: color-mix(in srgb, var(--danger, #ef4444) 12%, transparent);
  border-color: color-mix(in srgb, var(--danger, #ef4444) 40%, transparent);
  color: var(--danger, #ef4444);
}

.btn.sm {
  padding: 3px 9px;
  font-size: 11.5px;
}

.btn.full {
  width: 100%;
  padding: 8px 12px;
  font-size: 13px;
}

/* 引导面板 */
.setup-wrap {
  margin: auto;
  max-width: 420px;
  width: 100%;
  padding: 16px 0;
}

.setup-panel {
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.setup-head {
  text-align: center;
}

.setup-icon {
  color: var(--primary);
  margin-bottom: 6px;
}

.setup-h2 {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 4px;
}

.setup-p {
  font-size: 11.5px;
  color: var(--text-muted);
  line-height: 1.5;
}

.setup-p code {
  font-family: monospace;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.label {
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text-muted);
}

input {
  padding: 6px 9px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  font-size: 12.5px;
  outline: none;
}

input:focus {
  border-color: var(--primary);
}

.grid2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.scanned-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 90px;
  overflow-y: auto;
}

.scanned-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  border-radius: 5px;
  font-size: 11px;
  cursor: pointer;
}

.scanned-row .chk {
  opacity: 0;
  color: var(--primary);
}

.scanned-row.active {
  border-color: var(--primary);
  color: var(--primary);
}

.scanned-row.active .chk {
  opacity: 1;
}

/* 弹窗 Dialog */
.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
}

.dialog {
  width: 420px;
  max-width: 90vw;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.dialog-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
}

.dialog-head h3 {
  font-size: 13.5px;
  font-weight: 600;
  margin: 0;
}

.dialog-body {
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.suffix-input {
  display: flex;
  align-items: center;
}

.suffix-input input {
  flex: 1;
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
}

.suffix {
  padding: 6px 8px;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid var(--border);
  border-left: none;
  border-top-right-radius: 6px;
  border-bottom-right-radius: 6px;
  font-size: 11.5px;
  font-family: monospace;
  color: var(--text-muted);
}

.field-tip {
  font-size: 10.5px;
  color: var(--text-muted);
}

.dialog-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
}

/* 控制台弹窗 */
.log-dialog {
  width: 600px;
  height: 420px;
}

.log-title {
  display: flex;
  align-items: center;
  gap: 6px;
}

.log-title .sub-tip {
  font-size: 11px;
  color: var(--text-muted);
  font-weight: normal;
}

.head-ops {
  display: flex;
  align-items: center;
  gap: 6px;
}

.log-box {
  flex: 1;
  padding: 10px 12px;
  background: color-mix(in srgb, var(--surface-container) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text-muted);
  font-family: 'Consolas', 'Fira Code', monospace;
  font-size: 11.5px;
  line-height: 1.5;
  overflow-y: auto;
  user-select: text;
}

.log-none {
  text-align: center;
  margin-top: 60px;
  color: var(--text-muted);
}

.log-row {
  white-space: pre-wrap;
  word-break: break-all;
}

.mono {
  font-family: monospace;
}

.share-dashboard {
  flex-shrink: 0;
  gap: 0;
  padding: 0;
  overflow: hidden;
  border-radius: 14px;
  box-shadow: 0 8px 24px color-mix(in srgb, var(--text) 5%, transparent);
}

.share-hero {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px;
  background:
    radial-gradient(circle at 85% 10%, color-mix(in srgb, var(--primary) 13%, transparent), transparent 40%),
    color-mix(in srgb, var(--surface-subtle) 68%, var(--surface));
  border-bottom: 1px solid var(--border);
}

.share-hero-copy { min-width: 0; }
.share-eyebrow { display: block; margin-bottom: 5px; color: var(--primary); font-size: 9px; font-weight: 750; letter-spacing: .16em; }
.share-header { display: flex; align-items: center; gap: 14px; }
.share-header .sb-title { font-size: 15px; }
.share-description { margin-top: 4px; font-size: 11px; }
.share-header .chip { display: inline-flex; align-items: center; gap: 6px; padding: 5px 9px; border-radius: 999px; font-size: 10px; white-space: nowrap; }
.share-header .dot-status { width: 6px; height: 6px; }

.share-overview {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 9px;
  padding: 13px 18px 2px;
}

.share-stat {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: color-mix(in srgb, var(--surface-container) 55%, transparent);
}

.share-stat > span { color: var(--text-muted); font-size: 10px; }
.share-stat strong { overflow: hidden; color: var(--text); font-size: 13px; text-overflow: ellipsis; white-space: nowrap; }
.share-stat small { overflow: hidden; color: var(--text-muted); font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }

.share-section { display: flex; flex-direction: column; gap: 12px; padding: 16px 18px; border-top: 1px solid var(--border); }
.share-section-heading { display: flex; align-items: center; gap: 10px; }
.share-section-heading > div { display: flex; flex-direction: column; gap: 2px; }
.share-section-heading strong { color: var(--text); font-size: 12px; }
.share-section-heading small { color: var(--text-muted); font-size: 9.5px; }
.section-index { display: grid; width: 27px; height: 27px; place-items: center; flex: none; border: 1px solid color-mix(in srgb, var(--primary) 25%, var(--border)); border-radius: 8px; background: color-mix(in srgb, var(--primary) 9%, transparent); color: var(--primary); font-family: var(--font-mono, monospace); font-size: 10px; font-weight: 700; }
.field-wide { grid-column: 1 / -1; }
.field .label { display: flex; align-items: center; gap: 7px; }
.optional-tag { color: var(--text-muted); font-size: 9px; font-weight: 400; }
.route-preview { display: flex; align-items: flex-start; gap: 8px; padding: 9px 11px; border: 1px solid color-mix(in srgb, var(--primary) 16%, var(--border)); border-radius: 8px; background: color-mix(in srgb, var(--primary) 4%, transparent); color: var(--text-muted); font-size: 10px; line-height: 1.55; overflow-wrap: anywhere; }
.route-preview code, .lan-address-note code { color: var(--primary); font-family: var(--font-mono, monospace); }
.route-led { width: 6px; height: 6px; margin-top: 4px; border-radius: 50%; flex: none; background: var(--primary); }

.option-card { display: flex; align-items: center; gap: 10px; min-height: 48px; padding: 9px 12px; border: 1px solid var(--border); border-radius: 9px; background: color-mix(in srgb, var(--surface-container) 50%, transparent); cursor: pointer; transition: border-color .15s, background .15s; }
.option-card:hover { border-color: color-mix(in srgb, var(--primary) 50%, var(--border)); }
.option-card input { position: absolute; width: 1px; height: 1px; opacity: 0; }
.option-check { display: grid; width: 17px; height: 17px; place-items: center; flex: none; border: 1px solid var(--border-strong); border-radius: 5px; background: var(--surface); }
.option-card input:checked + .option-check { border-color: var(--primary); background: var(--primary); }
.option-card input:checked + .option-check::after { width: 7px; height: 4px; border-bottom: 2px solid white; border-left: 2px solid white; content: ''; transform: rotate(-45deg) translateY(-1px); }
.option-card input:focus-visible + .option-check { outline: 2px solid var(--primary); outline-offset: 2px; }
.option-copy { display: flex; min-width: 0; flex-direction: column; gap: 3px; flex: 1; }
.option-copy strong { color: var(--text); font-size: 11px; }
.option-copy small { color: var(--text-muted); font-size: 9.5px; }
.option-port, .option-state { padding: 3px 7px; border-radius: 6px; background: color-mix(in srgb, var(--primary) 9%, transparent); color: var(--primary); font-size: 10px; }
.lan-address-note { display: flex; align-items: flex-start; gap: 7px; padding: 2px 2px 0; color: var(--text-muted); font-size: 10px; line-height: 1.5; overflow-wrap: anywhere; }
.lan-address-icon { color: var(--primary); }
.allowlist-field { margin-top: 2px; }
.upload-settings { margin-top: 1px; }
.inline-note { margin: -3px 0 0; color: var(--text-muted); font-size: 9.5px; }

.share-links-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 9px; }
.share-link-card { display: flex; min-width: 0; flex-direction: column; gap: 10px; padding: 11px; border: 1px solid var(--border); border-radius: 9px; background: color-mix(in srgb, var(--surface-container) 48%, transparent); }
.link-card-heading { display: flex; align-items: center; gap: 8px; }
.link-card-heading > div { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.link-card-heading strong { color: var(--text); font-size: 10.5px; }
.link-card-heading small { overflow: hidden; color: var(--text-muted); font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
.link-icon { display: grid; width: 26px; height: 26px; place-items: center; flex: none; border-radius: 8px; font-size: 15px; }
.local-icon { background: color-mix(in srgb, var(--success, #10b981) 13%, transparent); color: var(--success, #10b981); }
.lan-icon { background: color-mix(in srgb, #0ea5e9 13%, transparent); color: #0ea5e9; }
.tunnel-icon { background: color-mix(in srgb, var(--primary) 12%, transparent); color: var(--primary); }
.share-link-card code { min-height: 30px; padding: 7px 8px; border: 1px solid var(--border); border-radius: 6px; background: color-mix(in srgb, var(--surface) 65%, transparent); color: var(--primary); font-size: 9px; line-height: 1.4; overflow-wrap: anywhere; user-select: all; }
.share-link-card .share-link-actions { margin-top: auto; }
.links-empty { display: flex; align-items: center; gap: 10px; padding: 14px; border: 1px dashed var(--border-strong); border-radius: 9px; color: var(--text-muted); }
.links-empty > span { color: var(--primary); font-size: 19px; }
.links-empty > div { display: flex; flex-direction: column; gap: 4px; }
.links-empty strong { color: var(--text); font-size: 10.5px; }
.links-empty small { font-size: 9px; }
.link-security-row { display: flex; align-items: center; justify-content: space-between; gap: 10px; color: var(--text-muted); font-size: 9px; }
.share-footer { align-items: center; padding: 12px 18px; border-top: 1px solid var(--border); background: color-mix(in srgb, var(--surface-subtle) 55%, transparent); }
.share-footer .share-warning { max-width: 520px; color: var(--text-muted); font-size: 9.5px; }

@media (max-width: 680px) {
  .share-overview { grid-template-columns: 1fr; }
  .share-stat { display: grid; grid-template-columns: 1fr auto; align-items: center; }
  .share-stat small { grid-column: 1 / -1; }
  .share-links-grid { grid-template-columns: 1fr; }
  .share-hero { align-items: flex-start; }
  .share-footer { align-items: flex-start; flex-direction: column; }
  .link-security-row { align-items: flex-start; flex-direction: column; }
}
</style>
