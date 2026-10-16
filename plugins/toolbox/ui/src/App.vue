<script setup lang="ts">
/** 工具箱插件详情页：左窄导航 + 右宽工具面板，全部纯前端实现（无 Rust 命令） */
import { computed, provide, ref, type Component } from 'vue'
import Toast from '../../../ui-sdk/Toast.vue'
import { TOOLBOX_API_KEY, type ToastKind } from './shared'
import JsonTool from './JsonTool.vue'
import Base64Tool from './Base64Tool.vue'
import UrlTool from './UrlTool.vue'
import TimestampTool from './TimestampTool.vue'
import UuidTool from './UuidTool.vue'
import RegexTool from './RegexTool.vue'
import HashTool from './HashTool.vue'
import ColorTool from './ColorTool.vue'
import QrTool from './QrTool.vue'
import PasswordTool from './PasswordTool.vue'
import './toolbox.css'

interface ToolEntry {
  id: string
  label: string
  component: Component
}

const TOOLS: ToolEntry[] = [
  { id: 'json', label: 'JSON 工具', component: JsonTool },
  { id: 'base64', label: 'Base64', component: Base64Tool },
  { id: 'url', label: 'URL 编解码', component: UrlTool },
  { id: 'timestamp', label: '时间戳', component: TimestampTool },
  { id: 'uuid', label: 'UUID 生成', component: UuidTool },
  { id: 'regex', label: '正则测试', component: RegexTool },
  { id: 'hash', label: '哈希计算', component: HashTool },
  { id: 'color', label: '颜色转换', component: ColorTool },
  { id: 'qr', label: '二维码识别', component: QrTool },
  { id: 'password', label: '密码生成', component: PasswordTool }
]

const activeId = ref('json')
const current = computed<ToolEntry>(() => TOOLS.find((t) => t.id === activeId.value) ?? TOOLS[0])

// ---------- Toast 与剪贴板（provide 给各工具复用） ----------

const toast = ref('')
const toastKind = ref<ToastKind>('info')

function showToast(message: string, kind: ToastKind = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

/** clipboard API 不可用时的降级方案（隐藏 textarea + execCommand） */
function legacyCopy(text: string): boolean {
  try {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(ta)
    return ok
  } catch {
    return false
  }
}

async function copyText(text: string, okMsg = '已复制') {
  if (!text) {
    showToast('内容为空，未复制', 'error')
    return
  }
  try {
    await navigator.clipboard.writeText(text)
    showToast(okMsg, 'success')
  } catch {
    if (legacyCopy(text)) showToast(okMsg, 'success')
    else showToast('复制失败：剪贴板不可用', 'error')
  }
}

provide(TOOLBOX_API_KEY, { copyText, showToast })
</script>

<template>
  <div class="toolbox">
    <!-- 左侧工具导航（纯文字，选中 accent 高亮） -->
    <nav class="tb-nav" aria-label="工具箱导航">
      <button
        v-for="t in TOOLS"
        :key="t.id"
        class="nav-item"
        :class="{ on: t.id === activeId }"
        @click="activeId = t.id"
      >
        {{ t.label }}
      </button>
    </nav>

    <!-- 右侧当前工具面板（独立滚动，KeepAlive 保留各工具状态） -->
    <section class="tb-main">
      <KeepAlive>
        <component :is="current.component" />
      </KeepAlive>
    </section>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.toolbox {
  display: flex;
  gap: 10px;
  height: 100%;
  min-height: 0;
}

/* ---------- 左侧导航 ---------- */
.tb-nav {
  flex: none;
  width: 140px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 45%, transparent);
  overflow-y: auto;
}

.nav-item {
  flex: none;
  padding: 7px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12.5px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  transition: background 0.15s, color 0.15s;
}
.nav-item:hover {
  background: color-mix(in srgb, var(--border) 30%, transparent);
  color: var(--text);
}
.nav-item.on {
  background: color-mix(in srgb, var(--primary) 18%, transparent);
  color: color-mix(in srgb, var(--primary) 60%, var(--text));
  font-weight: 600;
}
.nav-item:focus-visible {
  outline: 1px solid var(--primary);
  outline-offset: 1px;
}

/* ---------- 右侧工具面板 ---------- */
.tb-main {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 1px 2px 4px 0;
}
</style>
