<script setup lang="ts">
import { onMounted, ref } from 'vue'
import {
  getSettings,
  setSettings,
  chooseSaveDirectory,
  openSaveDirectory,
  type ScreenshotSettings,
} from './api'

const settings = ref<ScreenshotSettings>({
  hotkey: 'ctrl+shift+a',
  saveDirectory: '',
  autoCopy: true,
  scrollStep: 640,
  watermarkText: '',
  annotationColor: '#ff3a48',
  textSize: 18,
  ocrRegion: '',
  ocrSelectModel: 0,
  tencentOcrConfigured: false,
  recordingMp4Fps: 15,
  recordingMp4MaxHeight: 1080,
  recordingMp4BitrateMbps: 16,
  recordingGifFps: 15,
  recordingWebpFps: 15,
})
const recordingFpsOptions = [5, 8, 10, 12, 15, 20, 24, 30]
const message = ref('')
const saving = ref(false)
const secretId = ref('')
const secretKey = ref('')
const token = ref('')
const clearTencentCredentials = ref(false)
const editingTencentCredentials = ref(false)
function showMessage(value: string) {
  message.value = value
  window.setTimeout(() => {
    if (message.value === value) message.value = ''
  }, 2600)
}

async function refresh() {
  try {
    settings.value = await getSettings()
    editingTencentCredentials.value = !settings.value.tencentOcrConfigured
  } catch (error) {
    showMessage(String(error))
  }
}

async function saveSettings() {
  saving.value = true
  try {
    const enteredCredentials = Boolean(
      secretId.value.trim() || secretKey.value.trim() || token.value.trim(),
    )
    await setSettings(settings.value, {
      secretId: secretId.value,
      secretKey: secretKey.value,
      token: token.value,
      clear: clearTencentCredentials.value && !enteredCredentials,
    })
    const savedSettings = await getSettings()
    settings.value = savedSettings
    if (enteredCredentials && !clearTencentCredentials.value && !savedSettings.tencentOcrConfigured) {
      throw new Error('腾讯云凭据未能从宿主密钥库读回，请检查后端存储状态。')
    }
    secretId.value = ''
    secretKey.value = ''
    token.value = ''
    clearTencentCredentials.value = false
    editingTencentCredentials.value = !savedSettings.tencentOcrConfigured
    showMessage('设置已保存')
  } catch (error) {
    showMessage(String(error))
  } finally {
    saving.value = false
  }
}

async function onChooseDirectory() {
  try {
    const dir = await chooseSaveDirectory()
    if (dir) {
      settings.value.saveDirectory = dir
      showMessage('已选择保存目录')
    }
  } catch (error) {
    showMessage(String(error))
  }
}

async function onOpenDirectory() {
  try {
    await openSaveDirectory(settings.value.saveDirectory)
  } catch (error) {
    showMessage(String(error))
  }
}

onMounted(() => {
  void refresh()
})
</script>

<template>
  <main class="page">
    <header class="hero">
      <div>
        <div class="eyebrow">SCREENSHOT CORE</div>
        <h1>截图设置</h1>
        <p>配置快捷键、保存位置、滚动截图默认值和 OCR 服务。</p>
      </div>
      <button class="primary save-top" :disabled="saving" @click="saveSettings">
        {{ saving ? '保存中…' : '保存设置' }}
      </button>
    </header>

    <section class="card">
      <h2>截图与输出</h2>
      <div class="grid">
        <label>全局快捷键<input v-model="settings.hotkey" spellcheck="false" /></label>
        <label>滚动步长<input v-model.number="settings.scrollStep" type="number" min="120" max="1600" step="40" /></label>
      </div>
      <div class="dir-row">
        <span class="dir-label">保存目录</span>
        <div class="dir-control">
          <input
            :value="settings.saveDirectory"
            readonly
            class="dir-input"
            placeholder="默认：图片/Agile Tiles/Screenshots"
            @click="onChooseDirectory"
          />
          <button type="button" class="btn-action" title="浏览并选择保存目录" @click="onChooseDirectory">
            选择目录
          </button>
          <button type="button" class="btn-action" title="在文件管理器中打开保存目录" @click="onOpenDirectory">
            打开目录
          </button>
          <button
            v-if="settings.saveDirectory"
            type="button"
            class="btn-reset"
            title="恢复默认存储位置"
            @click="settings.saveDirectory = ''"
          >
            重置
          </button>
        </div>
      </div>
      <label>水印文字<input v-model="settings.watermarkText" placeholder="可选，导出时绘制在右下角" /></label>
      <label class="check"><input v-model="settings.autoCopy" type="checkbox" />完成后自动复制到图片剪贴板</label>
    </section>

    <section class="card">
      <h2>腾讯云 OCR 配置</h2>
      <p class="hint">点击截图工具栏或编辑器中的“提取文字”时，当前截图会通过 HTTPS 发送给腾讯云 RecognizeAgent。详情页只配置参数，不会发起识别。</p>
      <p class="credential-status" :class="{ ready: settings.tencentOcrConfigured }">
        {{ settings.tencentOcrConfigured ? '凭据已安全保存' : '尚未配置 SecretId 和 SecretKey' }}
      </p>
      <div v-if="settings.tencentOcrConfigured && !editingTencentCredentials" class="saved-credentials">
        <span>腾讯云密钥保存在此设备，可直接识别，无需重复输入。</span>
        <button
          type="button"
          class="btn-action"
          @click="editingTencentCredentials = true; clearTencentCredentials = false"
        >更换密钥</button>
      </div>
      <template v-if="!settings.tencentOcrConfigured || editingTencentCredentials">
        <label>SecretId<input v-model="secretId" autocomplete="off" placeholder="腾讯云 SecretId" /></label>
        <label>SecretKey<input v-model="secretKey" type="password" autocomplete="new-password" placeholder="腾讯云 SecretKey" /></label>
        <label>临时 Token<input v-model="token" type="password" autocomplete="new-password" placeholder="可选；临时密钥需要填写" /></label>
        <p class="hint">凭据保存在宿主的加密密钥库，不会写入截图设置文件。更换密钥时请同时填写 SecretId 和 SecretKey。</p>
      </template>
      <div class="grid">
        <label>地域<input v-model="settings.ocrRegion" placeholder="可选，例如 ap-guangzhou" /></label>
        <label>
          识别模型
          <select v-model.number="settings.ocrSelectModel">
            <option :value="0">推理模型（默认）</option>
            <option :value="1">识别、推理模型</option>
          </select>
        </label>
      </div>
      <label v-if="settings.tencentOcrConfigured && !editingTencentCredentials" class="check"><input v-model="clearTencentCredentials" type="checkbox" />清除已保存的腾讯云凭据</label>
    </section>

    <section class="card">
      <h2>录制与动图</h2>
      <p class="hint">分别设置录制为 MP4、GIF 或 WebP 时的帧率。预览导出为对应格式时也会使用该格式的帧率。</p>
      <div class="grid">
        <label>
          MP4 视频帧率
          <select v-model.number="settings.recordingMp4Fps">
            <option v-for="fps in recordingFpsOptions" :key="fps" :value="fps">{{ fps }} FPS</option>
          </select>
        </label>
        <label>
          MP4 分辨率上限
          <select v-model.number="settings.recordingMp4MaxHeight">
            <option :value="720">最高 720p</option>
            <option :value="1080">最高 1080p</option>
          </select>
        </label>
        <label>
          MP4 目标码率
          <select v-model.number="settings.recordingMp4BitrateMbps">
            <option v-for="mbps in [4, 6, 8, 10, 12, 16, 20, 24]" :key="mbps" :value="mbps">
              {{ mbps }} Mbps
            </option>
          </select>
        </label>
        <label>
          GIF 动图帧率
          <select v-model.number="settings.recordingGifFps">
            <option v-for="fps in recordingFpsOptions" :key="fps" :value="fps">{{ fps }} FPS</option>
          </select>
        </label>
        <label>
          WebP 动图帧率
          <select v-model.number="settings.recordingWebpFps">
            <option v-for="fps in recordingFpsOptions" :key="fps" :value="fps">{{ fps }} FPS</option>
          </select>
        </label>
      </div>
      <p class="hint">MP4 分辨率上限和目标码率越高，画面越清晰、文件也越大；帧率越高，动作越连贯。MP4 支持手动停止，GIF/WebP 直接录制最长 10 秒。</p>
    </section>

    <div class="footer">
      <button class="primary" :disabled="saving" @click="saveSettings">{{ saving ? '保存中…' : '保存设置' }}</button>
      <span v-if="message" class="message">{{ message }}</span>
    </div>
  </main>
</template>

<style scoped>
:global(*) { box-sizing: border-box }
:global(body) { margin: 0; background: transparent; color: var(--text, #e8edf7); font-family: var(--font-ui, "Segoe UI", "Microsoft YaHei", sans-serif) }
:global(html[data-agile-tiles-embedded] body) { background: transparent }
.page { min-height: calc(100vh - 32px); padding: 28px; background: color-mix(in srgb, var(--bg, #121722) calc(var(--detail-panel-opacity, 1) * 100%), transparent); border-radius: 18px }
.hero { display: flex; justify-content: space-between; gap: 18px; align-items: flex-start }
.eyebrow { color: var(--accent, #77a9ff); font-size: 10px; letter-spacing: .14em; font-weight: 700 }
h1 { margin: 6px 0; font-size: 28px; letter-spacing: -.03em }
.hero p, .hint { margin: 0; color: var(--text-muted, #8d98ad); font-size: 13px; line-height: 1.55 }
.card { margin-top: 16px; padding: 18px; border: 1px solid var(--border, #30394b); border-radius: 14px; background: color-mix(in srgb, var(--surface-subtle, #1d2533) calc(var(--component-opacity, 1) * 66%), transparent) }
h2 { margin: 0 0 14px; font-size: 13px }
.grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 16px }
label { display: grid; grid-template-columns: 96px 1fr; align-items: center; gap: 12px; margin: 10px 0; color: var(--text-muted, #8d98ad); font-size: 12px }
.dir-row { display: grid; grid-template-columns: 96px 1fr; align-items: center; gap: 12px; margin: 10px 0 }
.dir-label { color: var(--text-muted, #8d98ad); font-size: 12px }
.dir-control { display: flex; align-items: center; gap: 8px }
.dir-input { flex: 1; min-width: 0; cursor: pointer }
.btn-action { white-space: nowrap; padding: 8px 12px; font-size: 12px }
.btn-reset { white-space: nowrap; padding: 8px 10px; font-size: 12px; color: var(--text-muted, #8d98ad); border-color: transparent; background: transparent }
.btn-reset:hover { color: var(--danger, #ff6b6b) }
input:not([type=checkbox]) { min-width: 0; width: 100%; padding: 8px 10px; color: var(--text, #e8edf7); background: color-mix(in srgb, var(--surface, #121722) calc(var(--component-opacity, 1) * 100%), transparent); border: 1px solid var(--border, #30394b); border-radius: 8px; outline: none }
select { min-width: 0; width: 100%; padding: 8px 10px; color: var(--text, #e8edf7); background: color-mix(in srgb, var(--surface, #121722) calc(var(--component-opacity, 1) * 100%), transparent); border: 1px solid var(--border, #30394b); border-radius: 8px; outline: none }
input[type=color] { height: 34px; padding: 3px }
input:focus, select:focus { border-color: var(--accent, #77a9ff) }
.check { display: flex; grid-template-columns: none; gap: 8px }
.check input { accent-color: var(--accent, #77a9ff) }
button { border: 1px solid var(--border, #30394b); border-radius: 10px; padding: 9px 14px; color: var(--text, #e8edf7); background: color-mix(in srgb, var(--surface-subtle, #1d2533) calc(var(--component-opacity, 1) * 100%), transparent); cursor: pointer; font-size: 12px }
button:hover:not(:disabled) { border-color: var(--accent, #77a9ff) }
button:disabled { opacity: .45; cursor: not-allowed }
.primary { color: var(--text-on-accent, #08101e); background: color-mix(in srgb, var(--accent, #77a9ff) calc(var(--component-opacity, 1) * 100%), transparent); border-color: transparent; font-weight: 700 }
.save-top { white-space: nowrap }
.footer { display: flex; align-items: center; gap: 12px; margin-top: 18px }
.message { color: var(--success, #8de0b2); font-size: 12px }
.credential-status { margin: 0 0 10px; color: var(--warning, #ffbc72); font-size: 12px }
.credential-status.ready { color: var(--success, #8de0b2) }
.saved-credentials { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 10px 0 14px; color: var(--text-muted, #8d98ad); font-size: 12px }
code { color: var(--accent, #b8d1ff) }
@media (max-width: 620px) { .grid { grid-template-columns: 1fr } .dir-row { grid-template-columns: 1fr } .save-top { display: none } }
</style>
