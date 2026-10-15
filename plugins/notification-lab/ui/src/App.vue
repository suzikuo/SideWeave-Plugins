<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import {
  sendHostDesktopNotification,
  sendHostSidebarNotification,
  sendTestNotification,
  type DesktopNotificationKind
} from './api'

const presets = [
  { id: 'short', name: '普通短消息', title: '通知测试', body: '这是一条用于检查通知是否正常显示的测试消息。' },
  { id: 'long-title', name: '长标题', title: 'SideWeave 通知实验室：检查一段较长标题的显示效果', body: '观察标题换行、截断和卡片布局。' },
  { id: 'long-body', name: '长正文与换行', title: '多行正文测试', body: '第一行：检查较长正文的显示效果。\n第二行：检查系统是否保留换行。\n第三行：观察通知卡片的滚动和高度。' },
  { id: 'multilingual', name: '多语言和 Emoji', title: '你好 Hello こんにちは 🔔', body: '简体中文、繁體中文、English、日本語、한국어。' },
  { id: 'empty-title', name: '空标题', title: '', body: '检查空标题时系统和宿主卡片的显示方式。' },
  { id: 'empty-body', name: '空正文', title: '空正文测试', body: '' }
]

type PreviewKind = DesktopNotificationKind
interface Preview {
  id: number
  kind: PreviewKind
  title: string
  body: string
}

const selectedPreset = ref('short')
const title = ref(presets[0].title)
const body = ref(presets[0].body)
const iconHint = ref('')
const soundHint = ref('')
const desktopKind = ref<DesktopNotificationKind>('info')
const durationSecs = ref(6)
const repeatCount = ref(3)
const intervalMs = ref(1000)
const sending = ref(false)
const status = ref('')
const statusKind = ref<'success' | 'error'>('success')
const previews = ref<Preview[]>([])
const previewTimers = new Map<number, number>()
let nextPreviewId = 0

const normalizedCount = computed(() => Math.min(10, Math.max(1, Math.floor(Number(repeatCount.value) || 1))))

function choosePreset() {
  const preset = presets.find((item) => item.id === selectedPreset.value)
  if (!preset) return
  title.value = preset.title
  body.value = preset.body
  status.value = ''
}

function wait(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms))
}

function removePreview(id: number) {
  previews.value = previews.value.filter((preview) => preview.id !== id)
  const timer = previewTimers.get(id)
  if (timer !== undefined) window.clearTimeout(timer)
  previewTimers.delete(id)
}

function showPreview(kind: PreviewKind) {
  const sample = {
    info: { title: '有一条新提醒', body: '这是普通信息样式预览。' },
    success: { title: '操作已完成', body: '设置已经保存，可以继续使用。' },
    warning: { title: '需要留意', body: '这条提示用于检查警告样式。' },
    alarm: { title: '专注时间结束', body: '起来活动一下，再开始下一轮。' }
  }[kind]
  const id = ++nextPreviewId
  previews.value.push({ id, kind, ...sample })
  previewTimers.set(id, window.setTimeout(() => removePreview(id), 6000))
}

async function send(channel: 'windows' | 'host' | 'sidebar', count = 1) {
  if (sending.value) return
  sending.value = true
  status.value = ''
  const total = Math.min(10, Math.max(1, Math.floor(count)))
  try {
    for (let index = 0; index < total; index += 1) {
      const sequenceTitle = total > 1 && title.value ? `${title.value} (${index + 1}/${total})` : title.value
      if (channel === 'host') {
        await sendHostDesktopNotification({
          title: sequenceTitle,
          body: body.value,
          kind: desktopKind.value,
          durationMs: Math.min(60, Math.max(1, Number(durationSecs.value) || 6)) * 1000
        })
      } else if (channel === 'sidebar') {
        await sendHostSidebarNotification({
          title: sequenceTitle,
          body: body.value,
          kind: desktopKind.value,
          durationMs: Math.min(60, Math.max(1, Number(durationSecs.value) || 6)) * 1000
        })
      } else {
        await sendTestNotification({
          title: sequenceTitle,
          body: body.value,
          ...(iconHint.value.trim() ? { icon: iconHint.value.trim() } : {}),
          ...(soundHint.value.trim() ? { sound: soundHint.value.trim() } : {})
        })
      }
      if (index + 1 < total) {
        await wait(Math.min(5000, Math.max(250, Number(intervalMs.value) || 1000)))
      }
    }
    statusKind.value = 'success'
    const destination = channel === 'host'
      ? '宿主桌面通知'
      : channel === 'sidebar' ? '侧边栏气泡通知' : 'Windows 系统通知'
    status.value = total === 1 ? `已发送到${destination}。` : `已发送 ${total} 条到${destination}。`
  } catch (error) {
    statusKind.value = 'error'
    status.value = error instanceof Error ? error.message : String(error)
  } finally {
    sending.value = false
  }
}

onUnmounted(() => {
  for (const timer of previewTimers.values()) window.clearTimeout(timer)
  previewTimers.clear()
})
</script>

<template>
  <aside class="preview-stack" aria-live="polite" aria-label="插件内通知预览">
    <article v-for="preview in previews" :key="preview.id" class="preview-toast" :class="preview.kind" role="status">
      <span class="preview-mark" aria-hidden="true">{{ preview.kind === 'success' ? '✓' : preview.kind === 'warning' ? '!' : preview.kind === 'alarm' ? '◷' : 'i' }}</span>
      <div class="preview-copy"><strong>{{ preview.title }}</strong><p>{{ preview.body }}</p></div>
      <button class="icon-button" type="button" aria-label="关闭提示" @click="removePreview(preview.id)">×</button>
    </article>
  </aside>

  <main class="page">
    <header class="heading">
      <div class="mark" aria-hidden="true">◉</div>
      <div><h1>通知实验室</h1><p>对比 Windows 系统通知、宿主桌面通知和侧边栏气泡。</p></div>
    </header>

    <section class="card">
      <div class="section-heading"><div><h2>通知内容</h2><p>选择示例场景，也可以直接编辑内容。</p></div></div>
      <label class="field"><span>测试场景</span>
        <select v-model="selectedPreset" @change="choosePreset">
          <option v-for="preset in presets" :key="preset.id" :value="preset.id">{{ preset.name }}</option>
          <option value="custom">自定义内容</option>
        </select>
      </label>
      <label class="field"><span>标题</span><input v-model="title" type="text" placeholder="可留空" /></label>
      <label class="field"><span>正文</span><textarea v-model="body" rows="3" placeholder="可留空" /></label>
      <details class="advanced">
        <summary>Windows 通知选项</summary>
        <label class="field"><span>图标提示</span><input v-model="iconHint" type="text" placeholder="平台支持的图标名称或路径" /></label>
        <label class="field"><span>声音提示</span><input v-model="soundHint" type="text" placeholder="平台支持的声音名称" /></label>
        <p class="hint">图标和声音只影响 Windows 系统通知，是否生效取决于 Windows。</p>
      </details>
    </section>

    <section class="card">
      <div class="section-heading"><div><h2>宿主桌面通知</h2><p>由宿主统一创建窗口、堆叠消息并按时销毁。</p></div></div>
      <div class="sequence-fields">
        <label class="field"><span>样式</span>
          <select v-model="desktopKind">
            <option value="info">普通</option><option value="success">成功</option>
            <option value="warning">警告</option><option value="alarm">闹钟</option>
          </select>
        </label>
        <label class="field"><span>显示时长 <small>秒，1–60</small></span><input v-model.number="durationSecs" type="number" min="1" max="60" /></label>
      </div>
      <div class="actions">
        <button class="button primary" type="button" :disabled="sending" @click="send('host')">显示一条宿主通知</button>
        <button class="button" type="button" :disabled="sending" @click="send('windows')">发送一条 Windows 通知</button>
      </div>
      <p class="hint">多条消息会同时出现在同一个宿主通知窗口中；每条都能单独关闭，也会在设定时间后自动消失。</p>
    </section>

    <section class="card">
      <div class="section-heading"><div><h2>侧边栏气泡通知</h2><p>在侧边栏旁显示漫画对话框样式，可容纳较长消息。</p></div></div>
      <div class="actions">
        <button class="button primary" type="button" :disabled="sending" @click="send('sidebar')">显示一条侧边栏气泡</button>
      </div>
      <p class="hint">多条消息共用一个按需创建的宿主窗口；每条可单独关闭，也会按设定时长自动销毁。气泡方向跟随侧边栏位置。</p>
    </section>

    <section class="card">
      <div class="section-heading"><div><h2>连续通知</h2><p>检查多条消息的堆叠效果和显示间隔。</p></div></div>
      <div class="sequence-fields">
        <label class="field"><span>条数 <small>最多 10 条</small></span><input v-model.number="repeatCount" type="number" min="1" max="10" /></label>
        <label class="field"><span>间隔 <small>毫秒</small></span><input v-model.number="intervalMs" type="number" min="250" max="5000" step="250" /></label>
      </div>
      <div class="actions">
        <button class="button" type="button" :disabled="sending" @click="send('sidebar', normalizedCount)">发送连续侧边栏气泡</button>
        <button class="button primary" type="button" :disabled="sending" @click="send('host', normalizedCount)">发送连续宿主通知</button>
        <button class="button" type="button" :disabled="sending" @click="send('windows', normalizedCount)">{{ sending ? '发送中…' : `发送 ${normalizedCount} 条 Windows 通知` }}</button>
      </div>
      <p v-if="status" class="status" :class="statusKind" role="status" aria-live="polite">{{ status }}</p>
    </section>

    <section class="card">
      <div class="section-heading"><div><h2>插件内样式预览</h2><p>本地 UI 示例，不会发送系统通知。</p></div></div>
      <div class="preview-buttons">
        <button class="button" type="button" @click="showPreview('info')">普通</button>
        <button class="button success-text" type="button" @click="showPreview('success')">成功</button>
        <button class="button warning-text" type="button" @click="showPreview('warning')">警告</button>
        <button class="button alarm-text" type="button" @click="showPreview('alarm')">闹钟</button>
      </div>
    </section>
    <p class="footnote">Windows 系统通知的外观和显示顺序由 Windows 控制；宿主桌面卡片与侧边栏气泡由 SideWeave 管理外观和生命周期。</p>
  </main>
</template>

<style scoped>
.page { display: grid; gap: 14px; max-width: 720px; margin: 0 auto; padding: 18px; }
.heading { display: flex; align-items: center; gap: 12px; padding: 2px 2px 4px; }
.mark { display: grid; width: 38px; height: 38px; flex: 0 0 auto; place-items: center; border: 1px solid color-mix(in srgb, var(--accent) 42%, var(--border)); border-radius: 12px; background: color-mix(in srgb, var(--accent) 15%, var(--surface)); color: var(--accent); font-size: 24px; }
h1, h2, p { margin: 0; }
h1 { font-size: 18px; font-weight: 650; }
.heading p, .section-heading p { margin-top: 3px; color: var(--text-muted); font-size: 12px; }
.card { display: grid; gap: 13px; padding: 15px; border: 1px solid var(--border); border-radius: var(--radius-lg, 12px); background: color-mix(in srgb, var(--surface) 86%, transparent); box-shadow: var(--shadow); }
h2 { font-size: 14px; font-weight: 620; }
.field { display: grid; min-width: 0; gap: 6px; color: var(--text-muted); font-size: 12px; }
.field > span { display: flex; align-items: baseline; gap: 6px; }
small { color: var(--text-muted); font-size: 11px; opacity: .8; }
input, textarea, select { width: 100%; border: 1px solid var(--border); border-radius: 8px; outline: none; padding: 9px 10px; background: var(--surface-subtle); color: var(--text); }
textarea { min-height: 76px; resize: vertical; line-height: 1.55; }
input:focus, textarea:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 18%, transparent); }
.advanced { display: grid; gap: 11px; border-top: 1px solid var(--border); padding-top: 11px; }
summary { width: fit-content; color: var(--text-muted); font-size: 12px; cursor: pointer; }
.hint, .footnote { color: var(--text-muted); font-size: 11.5px; line-height: 1.5; }
.actions, .preview-buttons { display: flex; flex-wrap: wrap; gap: 8px; }
.button { min-height: 36px; padding: 7px 13px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-subtle); color: var(--text); font-weight: 550; }
.button:hover:not(:disabled) { border-color: var(--border-strong); filter: brightness(1.08); }
.button.primary { border-color: var(--accent); background: var(--accent); color: #fff; }
.button:disabled { cursor: wait; opacity: .65; }
.sequence-fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.status { border-radius: 7px; padding: 8px 10px; background: color-mix(in srgb, var(--success) 12%, var(--surface)); color: var(--success); font-size: 12px; overflow-wrap: anywhere; }
.status.error { background: color-mix(in srgb, var(--danger) 12%, var(--surface)); color: var(--danger); }
.success-text { color: var(--success); }.warning-text { color: var(--warning); }.alarm-text { color: var(--danger); }
.preview-stack { position: fixed; z-index: 10; top: 12px; right: 12px; display: grid; width: min(340px, calc(100vw - 24px)); gap: 8px; pointer-events: none; }
.preview-toast { --preview-accent: var(--accent); display: grid; grid-template-columns: 28px minmax(0, 1fr) 22px; align-items: start; gap: 9px; padding: 11px; border: 1px solid color-mix(in srgb, var(--preview-accent) 45%, var(--border)); border-left: 3px solid var(--preview-accent); border-radius: 10px; background: var(--surface-container, var(--surface)); color: var(--text); box-shadow: var(--shadow); animation: enter .2s ease-out; pointer-events: auto; }
.preview-toast.success { --preview-accent: var(--success); }.preview-toast.warning { --preview-accent: var(--warning); }.preview-toast.alarm { --preview-accent: var(--danger); }
.preview-mark { display: grid; width: 26px; height: 26px; place-items: center; border-radius: 8px; background: color-mix(in srgb, var(--preview-accent) 15%, transparent); color: var(--preview-accent); font-weight: 700; }
.preview-copy { min-width: 0; }.preview-copy strong { display: block; margin: 1px 0 3px; font-size: 12.5px; }.preview-copy p { color: var(--text-muted); font-size: 11.5px; line-height: 1.45; overflow-wrap: anywhere; }
.icon-button { padding: 0; border: 0; background: transparent; color: var(--text-muted); font-size: 18px; line-height: 1; }
.icon-button:hover { color: var(--text); }.footnote { padding: 0 3px; }
@keyframes enter { from { opacity: 0; transform: translateY(-5px); } to { opacity: 1; transform: translateY(0); } }
@media (max-width: 560px) { .page { padding: 13px; }.sequence-fields { grid-template-columns: 1fr 1fr; } }
</style>
