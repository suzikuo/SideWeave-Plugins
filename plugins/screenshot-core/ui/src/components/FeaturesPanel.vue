<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  chooseSaveDirectory,
  openSaveDirectory,
  type ScreenshotSettings,
  type TencentCredentialsUpdate,
} from '../api'

const props = defineProps<{
  settings: ScreenshotSettings
  tencentForm: TencentCredentialsUpdate
}>()

const emit = defineEmits<{
  change: []
  toast: [message: string]
}>()

type SubTab = 'basic' | 'recording' | 'output' | 'ocr'
const activeTab = ref<SubTab>('basic')

const recordingFpsOptions = [5, 8, 10, 12, 15, 20, 24, 30]
const bitrateOptions = [4, 6, 8, 10, 12, 16, 20, 24]
const templateTokens = ['{{YYYY}}', '{{MM}}', '{{DD}}', '{{HH}}', '{{mm}}', '{{ss}}']

const fileNamePreview = computed(() => {
  const now = new Date()
  const pad = (n: number) => String(n).padStart(2, '0')
  const stem = (props.settings.fileNameTemplate || 'SideWeave_{{YYYY}}{{MM}}{{DD}}_{{HH}}{{mm}}{{ss}}')
    .replaceAll('{{YYYY}}', String(now.getFullYear()))
    .replaceAll('{{MM}}', pad(now.getMonth() + 1))
    .replaceAll('{{DD}}', pad(now.getDate()))
    .replaceAll('{{HH}}', pad(now.getHours()))
    .replaceAll('{{mm}}', pad(now.getMinutes()))
    .replaceAll('{{ss}}', pad(now.getSeconds()))
  return `${stem}.${props.settings.saveFormat || 'png'}`
})

function insertToken(token: string) {
  props.settings.fileNameTemplate = (props.settings.fileNameTemplate || '') + token
  emit('change')
}

async function onBrowseDir() {
  try {
    const dir = await chooseSaveDirectory()
    if (dir) {
      props.settings.saveDirectory = dir
      emit('change')
      emit('toast', '已更新保存目录')
    }
  } catch (err) {
    emit('toast', String(err))
  }
}

async function onOpenDir() {
  try {
    await openSaveDirectory(props.settings.saveDirectory)
  } catch (err) {
    emit('toast', String(err))
  }
}
</script>

<template>
  <div class="sn-panel">
    <div class="sn-section-head">
      <div>
        <h2 class="sn-title">功能设置</h2>
        <p class="sn-subtitle">精细定制截图、贴图、演示、录屏、文件输出与 OCR / 翻译引擎</p>
      </div>
    </div>

    <!-- SnowAir Segmented Pill Tabs -->
    <div class="feat-tabs">
      <button
        v-for="tab in [
          { id: 'basic', label: '基本功能' },
          { id: 'recording', label: '视频录制' },
          { id: 'output', label: '文件输出' },
          { id: 'ocr', label: '识别与翻译' },
        ] as const"
        :key="tab.id"
        type="button"
        class="feat-tab"
        :class="{ active: activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </div>

    <!-- Tab 1: Basic -->
    <div v-if="activeTab === 'basic'" class="feat-body">
      <div class="sn-group-label">截图行为</div>
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">完成截图默认动作</div>
            <div class="sn-row-desc">点击工具栏完成 (✓) 或按 Enter 后的输出行为</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="opt in [
                { id: 'copy_and_save', label: '复制并保存' },
                { id: 'copy_only', label: '仅复制' },
                { id: 'save_only', label: '仅保存' },
              ] as const"
              :key="opt.id"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.finishAction === opt.id }"
              @click="settings.finishAction = opt.id; emit('change')"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">智能窗口边界吸附</div>
            <div class="sn-row-desc">框选前自动高亮光标所在桌面窗口区域</div>
          </div>
          <button
            type="button"
            class="sn-switch"
            :class="{ on: settings.detectWindow }"
            @click="settings.detectWindow = !settings.detectWindow; emit('change')"
          >
            <span class="sn-switch-thumb" />
          </button>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">默认导出圆角 ({{ settings.defaultCornerRadius }}px)</div>
            <div class="sn-row-desc">截图选区左上角信息胶囊也可实时拖拽微调圆角</div>
          </div>
          <input
            v-model.number="settings.defaultCornerRadius"
            type="range"
            min="0"
            max="48"
            class="sn-slider"
            @input="emit('change')"
          />
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">默认添加柔和阴影</div>
            <div class="sn-row-desc">为导出截图外围添加 SnowAir 柔和透明阴影</div>
          </div>
          <button
            type="button"
            class="sn-switch"
            :class="{ on: settings.defaultShadow }"
            @click="settings.defaultShadow = !settings.defaultShadow; emit('change')"
          >
            <span class="sn-switch-thumb" />
          </button>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">HDR 色彩映射策略</div>
            <div class="sn-row-desc">在开启 Windows HDR 显示器时自动进行色调映射防止过曝</div>
          </div>
          <select
            v-model="settings.hdrCaptureMode"
            class="sn-select"
            @change="emit('change')"
          >
            <option value="auto">自动检测 (推荐)</option>
            <option value="force_on">强制 HDR 映射</option>
            <option value="force_off">关闭 HDR 映射</option>
          </select>
        </div>
      </div>

      <div class="sn-group-label">贴图与演示模式</div>
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">贴图滚轮以光标为中心缩放</div>
            <div class="sn-row-desc">关闭时则以贴图窗口几何中心进行缩放</div>
          </div>
          <button
            type="button"
            class="sn-switch"
            :class="{ on: settings.pinCursorZoom }"
            @click="settings.pinCursorZoom = !settings.pinCursorZoom; emit('change')"
          >
            <span class="sn-switch-thumb" />
          </button>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">贴图窗口双击动作</div>
            <div class="sn-row-desc">双击置顶贴图窗口时执行的快捷动作</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="opt in [
                { id: 'thumbnail', label: '切换缩略图' },
                { id: 'copy', label: '复制图片' },
                { id: 'close', label: '关闭贴图' },
              ] as const"
              :key="opt.id"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.pinDoubleClick === opt.id }"
              @click="settings.pinDoubleClick = opt.id; emit('change')"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">演示模式默认工具</div>
            <div class="sn-row-desc">启动全屏演示标注模式时的默认画笔</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="opt in [
                { id: 'fade_pen', label: '渐隐激光笔' },
                { id: 'pen', label: '持久画笔' },
                { id: 'none', label: '仅浏览' },
              ] as const"
              :key="opt.id"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.demoDefaultTool === opt.id }"
              @click="settings.demoDefaultTool = opt.id; emit('change')"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">激光笔渐隐时长 ({{ (settings.demoFadeMs / 1000).toFixed(1) }}s)</div>
            <div class="sn-row-desc">演示模式中渐隐激光笔笔迹自动消失的时间</div>
          </div>
          <input
            v-model.number="settings.demoFadeMs"
            type="range"
            min="800"
            max="6000"
            step="200"
            class="sn-slider"
            @input="emit('change')"
          />
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">演示工具栏默认位置</div>
            <div class="sn-row-desc">启动演示模式时悬浮控制条停靠位置（也可随时拖拽）</div>
          </div>
          <div class="sn-seg">
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.demoToolbarPos === 'top' }"
              @click="settings.demoToolbarPos = 'top'; emit('change')"
            >
              屏幕顶部
            </button>
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.demoToolbarPos === 'bottom' }"
              @click="settings.demoToolbarPos = 'bottom'; emit('change')"
            >
              屏幕底部
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 2: Recording -->
    <div v-else-if="activeTab === 'recording'" class="feat-body">
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">录制帧率 (FPS)</div>
            <div class="sn-row-desc">高帧率画面更流畅，同时占用稍高编码资源</div>
          </div>
          <select v-model.number="settings.recordingFps" class="sn-select" @change="emit('change')">
            <option v-for="fps in recordingFpsOptions" :key="fps" :value="fps">{{ fps }} FPS</option>
          </select>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">录制最大高度</div>
            <div class="sn-row-desc">超出上限高度时按比例平滑缩放</div>
          </div>
          <div class="sn-seg">
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.recordingMaxHeight === 720 }"
              @click="settings.recordingMaxHeight = 720; emit('change')"
            >
              720p
            </button>
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.recordingMaxHeight === 1080 }"
              @click="settings.recordingMaxHeight = 1080; emit('change')"
            >
              1080p
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">MP4 视频码率</div>
            <div class="sn-row-desc">影响 MP4 导出清晰度与体积</div>
          </div>
          <select
            v-model.number="settings.recordingBitrateMbps"
            class="sn-select"
            @change="emit('change')"
          >
            <option v-for="mb in bitrateOptions" :key="mb" :value="mb">{{ mb }} Mbps</option>
          </select>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">GIF 调色板色彩数</div>
            <div class="sn-row-desc">导出 GIF 动图时的量化颜色上限</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="c in [64, 128, 256]"
              :key="c"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.recordingGifColors === c }"
              @click="settings.recordingGifColors = c; emit('change')"
            >
              {{ c }} 色
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">动态 WebP 质量</div>
            <div class="sn-row-desc">导出动画 WebP 的压缩质量</div>
          </div>
          <select
            v-model.number="settings.recordingWebpQuality"
            class="sn-select"
            @change="emit('change')"
          >
            <option v-for="q in [50, 65, 80, 90, 95]" :key="q" :value="q">{{ q }}%</option>
          </select>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">录制鼠标光标</div>
            <div class="sn-row-desc">是否在录制视频中显示系统鼠标指针</div>
          </div>
          <button
            type="button"
            class="sn-switch"
            :class="{ on: settings.recordingShowCursor }"
            @click="settings.recordingShowCursor = !settings.recordingShowCursor; emit('change')"
          >
            <span class="sn-switch-thumb" />
          </button>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">录屏默认鼠标穿透</div>
            <div class="sn-row-desc">录制条可随时一键在「穿透操作」与「实时标注」之间切换</div>
          </div>
          <button
            type="button"
            class="sn-switch"
            :class="{ on: settings.recordingDefaultPassthrough }"
            @click="
              settings.recordingDefaultPassthrough = !settings.recordingDefaultPassthrough;
              emit('change')
            "
          >
            <span class="sn-switch-thumb" />
          </button>
        </div>
      </div>
    </div>

    <!-- Tab 3: Output -->
    <div v-else-if="activeTab === 'output'" class="feat-body">
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">默认保存格式</div>
            <div class="sn-row-desc">自动保存或另存为时的首选图片格式</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="fmt in ['png', 'jpg', 'webp'] as const"
              :key="fmt"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.saveFormat === fmt }"
              @click="settings.saveFormat = fmt; emit('change')"
            >
              {{ fmt.toUpperCase() }}
            </button>
          </div>
        </div>

        <div class="sn-row col">
          <div class="sn-row-info">
            <div class="sn-row-title">保存目录</div>
            <div class="sn-row-desc">留空默认保存至 Pictures\SideWeave\Screenshots</div>
          </div>
          <div class="dir-row">
            <input
              v-model="settings.saveDirectory"
              class="sn-input"
              placeholder="默认：图片\SideWeave\Screenshots"
              @input="emit('change')"
            />
            <button type="button" class="sn-ghost-btn" @click="onBrowseDir">浏览…</button>
            <button type="button" class="sn-ghost-btn" @click="onOpenDir">打开</button>
          </div>
        </div>

        <div class="sn-row col">
          <div class="sn-row-info">
            <div class="sn-row-title">文件名模板</div>
            <div class="sn-row-desc">预览：<code>{{ fileNamePreview }}</code></div>
          </div>
          <input
            v-model="settings.fileNameTemplate"
            class="sn-input"
            @input="emit('change')"
          />
          <div class="token-row">
            <button
              v-for="tk in templateTokens"
              :key="tk"
              type="button"
              class="token-chip"
              @click="insertToken(tk)"
            >
              + {{ tk }}
            </button>
          </div>
        </div>

        <div class="sn-row col">
          <div class="sn-row-info">
            <div class="sn-row-title">全局右下角水印</div>
            <div class="sn-row-desc">留空则不自动叠加（也可在画布中使用斜纹水印工具单独绘制）</div>
          </div>
          <input
            v-model="settings.watermarkText"
            class="sn-input"
            placeholder="例如：@SideWeave · 仅供内部审阅"
            @input="emit('change')"
          />
        </div>
      </div>
    </div>

    <!-- Tab 4: OCR & Translate -->
    <div v-else class="feat-body">
      <div class="sn-group-label">文字识别 (OCR)</div>
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">首选 OCR 引擎</div>
            <div class="sn-row-desc">
              Windows 离线 OCR 无需联网和密钥，且支持在截图上直接框选单行文字复制与原地翻译
            </div>
          </div>
          <div class="sn-seg">
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.ocrEngine === 'system' }"
              @click="settings.ocrEngine = 'system'; emit('change')"
            >
              Windows 离线 OCR
            </button>
            <button
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.ocrEngine === 'tencent' }"
              @click="settings.ocrEngine = 'tencent'; emit('change')"
            >
              腾讯云 OCR
            </button>
          </div>
        </div>

        <div v-if="settings.ocrEngine === 'tencent'" class="tencent-box">
          <div class="tencent-status">
            <span>腾讯云凭据状态：</span>
            <strong :class="{ ok: settings.tencentOcrConfigured }">
              {{ settings.tencentOcrConfigured ? '已安全保存于宿主密钥库' : '尚未配置' }}
            </strong>
          </div>
          <div class="tencent-grid">
            <input
              v-model="tencentForm.secretId"
              class="sn-input"
              placeholder="SecretId（留空则保持不变）"
              @input="emit('change')"
            />
            <input
              v-model="tencentForm.secretKey"
              type="password"
              class="sn-input"
              placeholder="SecretKey（留空则保持不变）"
              @input="emit('change')"
            />
            <input
              v-model="settings.ocrRegion"
              class="sn-input"
              placeholder="区域（可选，例如 ap-beijing）"
              @input="emit('change')"
            />
            <label class="tencent-clear">
              <input v-model="tencentForm.clear" type="checkbox" @change="emit('change')" />
              <span>保存时清除已存储的腾讯云凭据</span>
            </label>
          </div>
        </div>
      </div>

      <div class="sn-group-label">图片原地翻译 & 快捷翻译</div>
      <div class="sn-list-card">
        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">首选翻译引擎</div>
            <div class="sn-row-desc">若首选引擎请求失败，将自动无缝回退至其余可用引擎</div>
          </div>
          <div class="sn-seg">
            <button
              v-for="eng in [
                { id: 'youdao', label: '有道' },
                { id: 'microsoft', label: '微软' },
                { id: 'google', label: '谷歌' },
                { id: 'baidu', label: '百度' },
              ] as const"
              :key="eng.id"
              type="button"
              class="sn-seg-btn"
              :class="{ active: settings.translateEngine === eng.id }"
              @click="settings.translateEngine = eng.id; emit('change')"
            >
              {{ eng.label }}
            </button>
          </div>
        </div>

        <div class="sn-row">
          <div class="sn-row-info">
            <div class="sn-row-title">默认目标语言</div>
            <div class="sn-row-desc">点击截图工具栏「译」按钮原地翻译时的目标语种</div>
          </div>
          <select
            v-model="settings.translateTargetLang"
            class="sn-select"
            @change="emit('change')"
          >
            <option value="zh-CN">简体中文</option>
            <option value="zh-TW">繁体中文</option>
            <option value="en">English</option>
            <option value="ja">日本語</option>
            <option value="ko">한국어</option>
            <option value="fr">Français</option>
            <option value="de">Deutsch</option>
          </select>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.feat-tabs {
  display: inline-flex;
  padding: 4px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  gap: 4px;
  margin-bottom: 16px;
}
.feat-tab {
  padding: 6px 16px;
  border-radius: 9px;
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.14s ease;
}
.feat-tab.active {
  background: var(--sn-card);
  color: var(--sn-text);
  font-weight: 600;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.16);
}
.feat-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.dir-row {
  display: flex;
  gap: 8px;
  width: 100%;
  margin-top: 8px;
}
.token-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.token-chip {
  padding: 3px 8px;
  border-radius: 6px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  color: var(--sn-muted);
  font-size: 11px;
  font-family: var(--font-mono, Consolas, monospace);
  cursor: pointer;
}
.token-chip:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}
.tencent-box {
  padding: 14px 16px;
  border-top: 1px solid var(--sn-border);
  background: var(--sn-subtle);
}
.tencent-status {
  font-size: 12px;
  color: var(--sn-muted);
  margin-bottom: 10px;
}
.tencent-status strong.ok {
  color: var(--sn-accent);
}
.tencent-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}
.tencent-clear {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--sn-muted);
}
</style>
