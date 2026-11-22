<script setup lang="ts">
import { ref } from 'vue'
import type { ScreenshotSettings } from '../api'

const props = defineProps<{
  settings: ScreenshotSettings
}>()

const emit = defineEmits<{
  change: []
  action: [kind: 'region' | 'scroll' | 'record' | 'demo' | 'quick_pin']
}>()

type HotkeyField = 'hotkey' | 'scrollHotkey' | 'recordHotkey' | 'demoHotkey' | 'quickPinHotkey'

const recordingField = ref<HotkeyField | null>(null)

const hotkeyItems: {
  field: HotkeyField
  title: string
  desc: string
  action: 'region' | 'scroll' | 'record' | 'demo' | 'quick_pin'
  badge: string
}[] = [
  {
    field: 'hotkey',
    title: '区域截图 / 停止录制',
    desc: '智能窗口吸附、圆角阴影调节、丰富标注、OCR 原地框选与图片翻译',
    action: 'region',
    badge: '核心',
  },
  {
    field: 'scrollHotkey',
    title: '滚动长截图',
    desc: '自动/手动拼接超长网页与文档，支持空格暂停/恢复与侧边滚动预览',
    action: 'scroll',
    badge: '长图',
  },
  {
    field: 'recordHotkey',
    title: '区域录屏',
    desc: '高清 MP4 / GIF / WebP 录制，录制中支持一键切换鼠标穿透与实时画笔',
    action: 'record',
    badge: '录制',
  },
  {
    field: 'demoHotkey',
    title: '演示标注模式',
    desc: '全屏透明桌面画板、渐隐激光笔、白板/黑板/图片幻灯片演示',
    action: 'demo',
    badge: '演示',
  },
  {
    field: 'quickPinHotkey',
    title: '快速贴图',
    desc: '置顶贴出剪贴板图片或文字卡片，支持滚轮光标缩放与双击缩略图',
    action: 'quick_pin',
    badge: '贴图',
  },
]

function formatHotkeyDisplay(raw: string): string {
  if (!raw || !raw.trim()) return '未设置'
  return raw
    .split('+')
    .map((p) => {
      const low = p.trim().toLowerCase()
      if (low === 'ctrl' || low === 'control') return 'Ctrl'
      if (low === 'alt') return 'Alt'
      if (low === 'shift') return 'Shift'
      if (low === 'win' || low === 'meta') return 'Win'
      if (low === 'space') return 'Space'
      return p.trim().toUpperCase()
    })
    .join(' + ')
}

function startRecording(field: HotkeyField) {
  recordingField.value = field
}

function onKeyDown(e: KeyboardEvent, field: HotkeyField) {
  if (recordingField.value !== field) return
  e.preventDefault()
  e.stopPropagation()

  if (e.key === 'Escape') {
    recordingField.value = null
    return
  }
  if (e.key === 'Backspace' || e.key === 'Delete') {
    if (field !== 'hotkey') {
      props.settings[field] = ''
      emit('change')
    }
    recordingField.value = null
    return
  }

  const key = e.key
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(key)) {
    return
  }

  const parts: string[] = []
  if (e.ctrlKey) parts.push('ctrl')
  if (e.altKey) parts.push('alt')
  if (e.shiftKey) parts.push('shift')
  if (e.metaKey) parts.push('win')

  let normalized = ''
  if (/^F([1-9]|1[0-2])$/i.test(key)) {
    normalized = key.toLowerCase()
  } else if (key === ' ') {
    normalized = 'space'
  } else if (key.length === 1 && /[a-zA-Z0-9\-=\[\]`,./;'\\]/.test(key)) {
    normalized = key.toLowerCase()
  } else if (['PrintScreen', 'Insert', 'Home', 'End', 'PageUp', 'PageDown'].includes(key)) {
    normalized = key.toLowerCase()
  } else {
    return
  }

  if (parts.length === 0 && !normalized.startsWith('f') && normalized !== 'printscreen') {
    return
  }

  parts.push(normalized)
  props.settings[field] = parts.join('+')
  recordingField.value = null
  emit('change')
}

function toggleDoubleTapF1() {
  props.settings.doubleTapF1 = !props.settings.doubleTapF1
  emit('change')
}
</script>

<template>
  <div class="sn-panel">
    <div class="sn-section-head">
      <div>
        <h2 class="sn-title">全局热键</h2>
        <p class="sn-subtitle">点击热键胶囊后按下组合键即可录制，按 Esc 取消、Backspace 清除</p>
      </div>
    </div>

    <div class="sn-list-card">
      <div
        v-for="item in hotkeyItems"
        :key="item.field"
        class="sn-row"
      >
        <div class="sn-row-info">
          <div class="sn-row-title">
            <span>{{ item.title }}</span>
            <span class="sn-badge">{{ item.badge }}</span>
          </div>
          <div class="sn-row-desc">{{ item.desc }}</div>
        </div>

        <div class="sn-row-actions">
          <button
            type="button"
            class="sn-hotkey-pill"
            :class="{ recording: recordingField === item.field }"
            @click="startRecording(item.field)"
            @keydown="onKeyDown($event, item.field)"
            @blur="recordingField === item.field && (recordingField = null)"
          >
            <template v-if="recordingField === item.field">
              请按下快捷键…
            </template>
            <template v-else>
              {{ formatHotkeyDisplay(settings[item.field]) }}
            </template>
          </button>

          <button
            type="button"
            class="sn-ghost-btn"
            title="立即启动"
            @click="emit('action', item.action)"
          >
            启动
          </button>
        </div>
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">双击 F1 快速截图</div>
          <div class="sn-row-desc">连续两次轻按 F1 键即可呼出区域截图（录屏时则停止录制）</div>
        </div>
        <button
          type="button"
          class="sn-switch"
          :class="{ on: settings.doubleTapF1 }"
          role="switch"
          :aria-checked="settings.doubleTapF1"
          @click="toggleDoubleTapF1"
        >
          <span class="sn-switch-thumb" />
        </button>
      </div>
    </div>

    <div class="sn-tip-box">
      <div class="sn-tip-title">截图画布快捷操作提示</div>
      <div class="sn-kbd-grid">
        <div class="sn-kbd-item"><kbd>Esc</kbd><span>取消 / 关闭</span></div>
        <div class="sn-kbd-item"><kbd>Enter</kbd><span>完成截图</span></div>
        <div class="sn-kbd-item"><kbd>Ctrl+Z</kbd><span>撤销标注</span></div>
        <div class="sn-kbd-item"><kbd>Ctrl+Y</kbd><span>重做标注</span></div>
        <div class="sn-kbd-item"><kbd>Space</kbd><span>长截图暂停/继续</span></div>
        <div class="sn-kbd-item"><kbd>`</kbd><span>演示模式鼠标穿透</span></div>
        <div class="sn-kbd-item"><kbd>PgUp / PgDn</kbd><span>演示幻灯片翻页</span></div>
        <div class="sn-kbd-item"><kbd>Ctrl+滚轮</kbd><span>贴图透明度调节</span></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sn-hotkey-pill {
  min-width: 148px;
  height: 32px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-text);
  font-family: var(--font-mono, Consolas, monospace);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s ease;
  text-align: center;
}
.sn-hotkey-pill:hover {
  border-color: var(--sn-accent);
  background: var(--sn-hover);
}
.sn-hotkey-pill.recording {
  border-color: var(--sn-accent);
  background: color-mix(in srgb, var(--sn-accent) 16%, transparent);
  color: var(--sn-accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--sn-accent) 18%, transparent);
}
.sn-row-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.sn-tip-box {
  margin-top: 16px;
  padding: 14px 16px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
}
.sn-tip-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--sn-muted);
  margin-bottom: 10px;
}
.sn-kbd-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(175px, 1fr));
  gap: 8px 14px;
}
.sn-kbd-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--sn-muted);
}
.sn-kbd-item kbd {
  padding: 2px 6px;
  border-radius: 5px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-text);
  font-family: var(--font-mono, Consolas, monospace);
  font-size: 11px;
  white-space: nowrap;
}
</style>
