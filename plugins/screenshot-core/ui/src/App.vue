<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { getCurrentWindow } from '../../../ui-sdk/runtime'
import {
  DEFAULT_SETTINGS,
  getSettings,
  quickPin,
  setSettings,
  startCapture,
  startDemo,
  startRecording,
  type ScreenshotSettings,
  type TencentCredentialsUpdate,
} from './api'
import AppearancePanel from './components/AppearancePanel.vue'
import FeaturesPanel from './components/FeaturesPanel.vue'
import HistoryPanel from './components/HistoryPanel.vue'
import HotkeysPanel from './components/HotkeysPanel.vue'
import ImageEditorPanel from './components/ImageEditorPanel.vue'
import QuickTranslatePanel from './components/QuickTranslatePanel.vue'

type NavId = 'hotkeys' | 'translate' | 'appearance' | 'features' | 'history' | 'editor'

const navItems: { id: NavId; label: string }[] = [
  { id: 'hotkeys', label: '全局热键' },
  { id: 'translate', label: '快捷翻译' },
  { id: 'appearance', label: '外观设置' },
  { id: 'features', label: '功能设置' },
  { id: 'history', label: '历史记录' },
  { id: 'editor', label: '自由画布' },
]

const activeNav = ref<NavId>('hotkeys')
const settings = ref<ScreenshotSettings>({ ...DEFAULT_SETTINGS })
const tencentForm = ref<TencentCredentialsUpdate>({
  secretId: '',
  secretKey: '',
  token: '',
  clear: false,
})
const dirty = ref(false)
const saving = ref(false)
const toast = ref('')
const incomingEditorPath = ref<string | null>(null)
const isStandalone = window.parent === window

let toastTimer: number | null = null
function showToast(msg: string) {
  toast.value = msg
  if (toastTimer !== null) window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => {
    toast.value = ''
    toastTimer = null
  }, 2800)
}

const resolvedTheme = computed<'dark' | 'light'>(() => {
  if (settings.value.themeMode === 'dark') return 'dark'
  if (settings.value.themeMode === 'light') return 'light'
  if (typeof settings.value.systemPrefersLight === 'boolean') {
    return settings.value.systemPrefersLight ? 'light' : 'dark'
  }
  const hostTheme = document.documentElement.dataset.theme
  if (hostTheme === 'light' || hostTheme === 'dark') return hostTheme
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
})

async function loadSettings() {
  try {
    const loaded = await getSettings()
    settings.value = { ...DEFAULT_SETTINGS, ...loaded }
    dirty.value = false
  } catch (err) {
    showToast(`加载设置失败：${String(err)}`)
  }
}

async function saveAll() {
  if (saving.value) return
  saving.value = true
  try {
    const enteredCreds = Boolean(
      tencentForm.value.secretId.trim() ||
        tencentForm.value.secretKey.trim() ||
        tencentForm.value.token.trim(),
    )
    const credsPayload: TencentCredentialsUpdate | undefined =
      enteredCreds || tencentForm.value.clear
        ? {
            secretId: tencentForm.value.secretId,
            secretKey: tencentForm.value.secretKey,
            token: tencentForm.value.token,
            clear: tencentForm.value.clear && !enteredCreds,
          }
        : undefined
    await setSettings(settings.value, credsPayload)
    const refreshed = await getSettings()
    settings.value = { ...DEFAULT_SETTINGS, ...refreshed }
    tencentForm.value = { secretId: '', secretKey: '', token: '', clear: false }
    dirty.value = false
    showToast('设置已保存并立即生效')
  } catch (err) {
    showToast(`保存失败：${String(err)}`)
  } finally {
    saving.value = false
  }
}

async function triggerAction(kind: 'region' | 'scroll' | 'record' | 'demo' | 'quick_pin') {
  try {
    if (kind === 'region') await startCapture('region')
    else if (kind === 'scroll') await startCapture('scroll')
    else if (kind === 'record') await startRecording(settings.value.recordingFps || 15)
    else if (kind === 'demo') await startDemo()
    else if (kind === 'quick_pin') await quickPin()
  } catch (err) {
    showToast(String(err))
  }
}

function onSendToEditor(path: string) {
  incomingEditorPath.value = path
  activeNav.value = 'editor'
  showToast('已将图片发送到自由画布')
}

function onGlobalKeyDown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
    e.preventDefault()
    void saveAll()
  }
}

function onShellContextMenu(e: MouseEvent) {
  if (e.defaultPrevented) return
  const target = e.target as HTMLElement | null
  if (target) {
    const tag = target.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || target.isContentEditable) {
      return
    }
  }
  e.preventDefault()
}

function closeStandalone() {
  try {
    void getCurrentWindow().close()
  } catch {
    // ignore
  }
}

onMounted(() => {
  void loadSettings()
  window.addEventListener('keydown', onGlobalKeyDown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onGlobalKeyDown)
})
</script>

<template>
  <div class="sn-shell" :data-sn-theme="resolvedTheme" @contextmenu="onShellContextMenu">
    <!-- Left Sidebar -->
    <aside class="sn-sidebar">
      <div class="sn-brand" data-tauri-drag-region>
        <div class="sn-logo">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
            <path d="M4 9V6a2 2 0 0 1 2-2h3" stroke="#fff" stroke-width="2.4" stroke-linecap="round" />
            <path d="M15 4h3a2 2 0 0 1 2 2v3" stroke="#fff" stroke-width="2.4" stroke-linecap="round" />
            <path d="M20 15v3a2 2 0 0 1-2 2h-3" stroke="#fff" stroke-width="2.4" stroke-linecap="round" />
            <path d="M9 20H6a2 2 0 0 1-2-2v-3" stroke="#fff" stroke-width="2.4" stroke-linecap="round" />
            <circle cx="12" cy="12" r="2.6" fill="#fff" />
          </svg>
        </div>
        <div class="sn-brand-text" data-tauri-drag-region>
          <div class="sn-brand-name">快捷截图</div>
          <div class="sn-brand-ver">SnowAir Edition · v0.4</div>
        </div>
      </div>

      <!-- Quick Launch Grid -->
      <div class="sn-quick-grid">
        <button type="button" class="sn-quick-btn primary" @click="triggerAction('region')">
          区域截图
        </button>
        <button type="button" class="sn-quick-btn" @click="triggerAction('scroll')">
          长截图
        </button>
        <button type="button" class="sn-quick-btn" @click="triggerAction('record')">
          区域录屏
        </button>
        <button type="button" class="sn-quick-btn" @click="triggerAction('demo')">
          演示标注
        </button>
        <button type="button" class="sn-quick-btn full" @click="triggerAction('quick_pin')">
          快速贴图（剪贴板图片 / 文本）
        </button>
      </div>

      <!-- Navigation -->
      <nav class="sn-nav">
        <button
          v-for="item in navItems"
          :key="item.id"
          type="button"
          class="sn-nav-item"
          :class="{ active: activeNav === item.id }"
          @click="activeNav = item.id"
        >
          <span class="sn-nav-icon">
            <svg v-if="item.id === 'hotkeys'" viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <rect x="2.5" y="4.5" width="15" height="11" rx="2.2" />
              <path d="M5.5 8.5h1M9 8.5h2M13.5 8.5h1M6.5 12h7" stroke-linecap="round" />
            </svg>
            <svg v-else-if="item.id === 'translate'" viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M3.5 5.5h8M7.5 3.5v2M5 5.5c.6 3 2.5 5 5 6M9.5 5.5c-.6 2.7-2.4 4.8-5 6" stroke-linecap="round" />
              <path d="M11 16.5l3-7.5 3 7.5M12 14h4" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <svg v-else-if="item.id === 'appearance'" viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <circle cx="10" cy="10" r="7" />
              <path d="M10 3v14" />
              <path d="M10 6.5a3.5 3.5 0 0 1 0 7" fill="currentColor" />
            </svg>
            <svg v-else-if="item.id === 'features'" viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M4 6h12M4 10h12M4 14h12" stroke-linecap="round" />
              <circle cx="7.5" cy="6" r="1.7" fill="currentColor" />
              <circle cx="13" cy="10" r="1.7" fill="currentColor" />
              <circle cx="9" cy="14" r="1.7" fill="currentColor" />
            </svg>
            <svg v-else-if="item.id === 'history'" viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <circle cx="10" cy="10" r="7" />
              <path d="M10 6v4.2l2.8 1.8" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <svg v-else viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
              <rect x="3" y="3" width="6" height="6" rx="1.2" />
              <rect x="11" y="3" width="6" height="6" rx="1.2" />
              <rect x="3" y="11" width="6" height="6" rx="1.2" />
              <rect x="11" y="11" width="6" height="6" rx="1.2" />
            </svg>
          </span>
          <span>{{ item.label }}</span>
        </button>
      </nav>

      <div class="sn-sidebar-foot">
        <button
          type="button"
          class="sn-save-bar"
          :class="{ dirty }"
          :disabled="saving"
          @click="saveAll"
        >
          <span class="sn-dot" :class="{ active: dirty }" />
          <span>{{ saving ? '保存中…' : dirty ? '保存更改 (Ctrl+S)' : '设置已同步' }}</span>
        </button>
      </div>
    </aside>

    <!-- Main Floating Content Card -->
    <main class="sn-main">
      <div class="sn-topbar" data-tauri-drag-region>
        <div class="sn-topbar-left" data-tauri-drag-region>
          <span v-if="dirty" class="sn-dirty-pill">有未保存更改</span>
        </div>
        <div class="sn-topbar-right">
          <button
            v-if="dirty"
            type="button"
            class="sn-primary-btn sm"
            :disabled="saving"
            @click="saveAll"
          >
            {{ saving ? '保存中…' : '保存设置' }}
          </button>
          <button
            v-if="isStandalone"
            type="button"
            class="sn-win-close"
            title="关闭窗口"
            @click="closeStandalone"
          >
            ×
          </button>
        </div>
      </div>

      <div class="sn-scroll" :class="{ 'canvas-host': activeNav === 'editor' }">
        <HotkeysPanel
          v-if="activeNav === 'hotkeys'"
          :settings="settings"
          @change="dirty = true"
          @action="triggerAction"
        />
        <QuickTranslatePanel
          v-else-if="activeNav === 'translate'"
          :settings="settings"
          @toast="showToast"
        />
        <AppearancePanel
          v-else-if="activeNav === 'appearance'"
          :settings="settings"
          @change="dirty = true"
        />
        <FeaturesPanel
          v-else-if="activeNav === 'features'"
          :settings="settings"
          :tencent-form="tencentForm"
          @change="dirty = true"
          @toast="showToast"
        />
        <HistoryPanel
          v-else-if="activeNav === 'history'"
          @toast="showToast"
          @send-to-editor="onSendToEditor"
        />
        <ImageEditorPanel
          v-else
          :settings="settings"
          :incoming-path="incomingEditorPath"
          @consumed-incoming="incomingEditorPath = null"
          @toast="showToast"
        />
      </div>
    </main>

    <!-- Toast Notification -->
    <Transition name="sn-toast-fade">
      <div v-if="toast" class="sn-toast">{{ toast }}</div>
    </Transition>
  </div>
</template>

<style>
* {
  box-sizing: border-box;
}
html,
body,
#app {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: Inter, 'Segoe UI Variable', 'Segoe UI', 'Microsoft YaHei UI', system-ui, sans-serif;
}

.sn-shell {
  --sn-bg: #141416;
  --sn-card: #1e1e20;
  --sn-subtle: #252528;
  --sn-hover: #2e2e32;
  --sn-text: #f4f4f6;
  --sn-muted: #9a9aa3;
  --sn-border: #2e2e33;
  --sn-accent: #34c759;
  --sn-accent-hover: #2eb850;

  display: flex;
  width: 100%;
  height: 100%;
  background: var(--sn-bg);
  color: var(--sn-text);
  padding: 10px;
  gap: 10px;
  position: relative;
}

.sn-shell[data-sn-theme='light'] {
  --sn-bg: #f2f3f5;
  --sn-card: #ffffff;
  --sn-subtle: #f7f7f9;
  --sn-hover: #eceef1;
  --sn-text: #18181b;
  --sn-muted: #71717a;
  --sn-border: #e4e4e7;
  --sn-accent: #22b855;
  --sn-accent-hover: #1da64b;
}

.sn-sidebar {
  width: 212px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 8px 6px 6px;
}

.sn-brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 6px;
  user-select: none;
}

.sn-logo {
  width: 34px;
  height: 34px;
  border-radius: 10px;
  background: linear-gradient(135deg, #34c759, #15a349);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 12px rgba(52, 199, 89, 0.32);
  flex-shrink: 0;
}

.sn-brand-name {
  font-size: 14.5px;
  font-weight: 700;
  color: var(--sn-text);
  line-height: 1.2;
}

.sn-brand-ver {
  font-size: 10.5px;
  color: var(--sn-muted);
  margin-top: 2px;
}

.sn-quick-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
  padding: 0 2px;
}

.sn-quick-btn {
  padding: 7px 6px;
  border-radius: 9px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-text);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.14s ease;
}

.sn-quick-btn:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
  background: var(--sn-hover);
}

.sn-quick-btn.primary {
  background: var(--sn-accent);
  border-color: var(--sn-accent);
  color: #fff;
  font-weight: 600;
}

.sn-quick-btn.primary:hover {
  background: var(--sn-accent-hover);
  color: #fff;
}

.sn-quick-btn.full {
  grid-column: 1 / -1;
  font-size: 11.5px;
}

.sn-nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.sn-nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border-radius: 10px;
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.14s ease;
  text-align: left;
}

.sn-nav-item:hover {
  background: var(--sn-hover);
  color: var(--sn-text);
}

.sn-nav-item.active {
  background: var(--sn-card);
  color: var(--sn-text);
  font-weight: 600;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.14);
}

.sn-nav-item.active .sn-nav-icon {
  color: var(--sn-accent);
}

.sn-nav-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.sn-sidebar-foot {
  padding-top: 4px;
}

.sn-save-bar {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 9px 10px;
  border-radius: 10px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-muted);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s ease;
}

.sn-save-bar.dirty {
  background: var(--sn-accent);
  border-color: var(--sn-accent);
  color: #fff;
}

.sn-save-bar.dirty:hover {
  background: var(--sn-accent-hover);
}

.sn-dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background: var(--sn-accent);
}

.sn-dot.active {
  background: #fff;
  box-shadow: 0 0 0 3px rgba(255, 255, 255, 0.35);
}

.sn-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  border-radius: 16px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  overflow: hidden;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.12);
}

.sn-topbar {
  height: 40px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--sn-border);
  flex-shrink: 0;
}

.sn-topbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.sn-dirty-pill {
  font-size: 11px;
  font-weight: 600;
  color: #f59e0b;
  padding: 2px 8px;
  border-radius: 999px;
  background: rgba(245, 158, 11, 0.14);
}

.sn-win-close {
  width: 28px;
  height: 28px;
  border-radius: 7px;
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
}

.sn-win-close:hover {
  background: #ef4444;
  color: #fff;
}

.sn-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px 28px;
}

.sn-scroll.canvas-host {
  padding: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

/* Shared SnowAir Panel Design System */
.sn-panel {
  max-width: 820px;
  margin: 0 auto;
}

.sn-section-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 16px;
}

.sn-title {
  margin: 0;
  font-size: 19px;
  font-weight: 700;
  color: var(--sn-text);
  letter-spacing: -0.01em;
}

.sn-subtitle {
  margin: 4px 0 0;
  font-size: 12.5px;
  color: var(--sn-muted);
}

.sn-group-label {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--sn-muted);
  margin: 10px 2px 6px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.sn-list-card {
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  overflow: hidden;
}

.sn-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 13px 16px;
  border-bottom: 1px solid var(--sn-border);
}

.sn-row:last-child {
  border-bottom: none;
}

.sn-row.col {
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
}

.sn-row-info {
  min-width: 0;
}

.sn-row-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--sn-text);
}

.sn-row-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--sn-muted);
}

.sn-badge {
  font-size: 10.5px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--sn-accent) 16%, transparent);
  color: var(--sn-accent);
}

.sn-switch {
  width: 42px;
  height: 24px;
  border-radius: 999px;
  border: none;
  background: var(--sn-border);
  position: relative;
  cursor: pointer;
  transition: background 0.18s ease;
  flex-shrink: 0;
  padding: 0;
}

.sn-switch.on {
  background: var(--sn-accent);
}

.sn-switch-thumb {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 999px;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.28);
  transition: transform 0.18s ease;
}

.sn-switch.on .sn-switch-thumb {
  transform: translateX(18px);
}

.sn-seg {
  display: inline-flex;
  padding: 3px;
  border-radius: 9px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  gap: 2px;
  flex-shrink: 0;
}

.sn-seg-btn {
  padding: 5px 11px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.14s ease;
  white-space: nowrap;
}

.sn-seg-btn.active {
  background: var(--sn-accent);
  color: #fff;
  font-weight: 600;
}

.sn-primary-btn {
  padding: 7px 14px;
  border-radius: 8px;
  border: none;
  background: var(--sn-accent);
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.14s ease;
  white-space: nowrap;
}

.sn-primary-btn:hover {
  background: var(--sn-accent-hover);
}

.sn-primary-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.sn-primary-btn.sm {
  padding: 5px 11px;
  font-size: 12px;
}

.sn-ghost-btn {
  padding: 6px 12px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-text);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
}

.sn-ghost-btn:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}

.sn-input,
.sn-select {
  height: 32px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-text);
  font-size: 12.5px;
  outline: none;
}

.sn-input {
  width: 100%;
}

.sn-input:focus,
.sn-select:focus {
  border-color: var(--sn-accent);
}

.sn-slider {
  width: 150px;
  accent-color: var(--sn-accent);
  cursor: pointer;
}

.sn-toast {
  position: fixed;
  bottom: 18px;
  right: 22px;
  padding: 9px 16px;
  border-radius: 10px;
  background: #18181b;
  color: #fafafa;
  border: 1px solid rgba(52, 199, 89, 0.5);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  font-size: 12.5px;
  font-weight: 500;
  z-index: 9999;
}

.sn-toast-fade-enter-active,
.sn-toast-fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.sn-toast-fade-enter-from,
.sn-toast-fade-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
