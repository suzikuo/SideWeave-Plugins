<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import {
  DEFAULT_TOOLBAR_ACTIVE,
  DEFAULT_TOOLBAR_INACTIVE,
  type ScreenshotSettings,
} from '../api'
import ToolbarIcon from './ToolbarIcon.vue'

const props = defineProps<{
  settings: ScreenshotSettings
}>()

const emit = defineEmits<{
  change: []
}>()

const ALL_TOOLS: Record<string, { label: string }> = {
  select: { label: '选择' },
  rect: { label: '形状' },
  arrow: { label: '箭头' },
  pen: { label: '画笔' },
  text: { label: '文字' },
  number: { label: '序号' },
  mosaic: { label: '特效' },
  eraser: { label: '橡皮擦' },
  undo: { label: '撤销' },
  redo: { label: '重做' },
  record: { label: '录屏' },
  pin: { label: '贴图' },
  ocr: { label: '识别文字' },
  translate: { label: '图片翻译' },
  scroll: { label: '长截图' },
  save: { label: '另存为' },
  finish: { label: '完成' },
  edit: { label: '编辑窗' },
  qr: { label: '二维码' },
}

const PRESET_COLORS = [
  '#ef4444',
  '#f59e0b',
  '#34c759',
  '#3b82f6',
  '#a855f7',
  '#ffffff',
  '#18181b',
]

interface DragState {
  id: string
  originList: 'active' | 'inactive'
  startX: number
  startY: number
  curX: number
  curY: number
  dragging: boolean
  hoverZone: 'active' | 'inactive' | null
  dirty: boolean
}

const drag = ref<DragState | null>(null)
const activeZoneRef = ref<HTMLElement | null>(null)
const inactiveZoneRef = ref<HTMLElement | null>(null)
const previewSelectedId = ref('rect')

const inactiveList = computed(() => {
  const activeSet = new Set(props.settings.toolbarActive)
  const fromInactive = props.settings.toolbarInactive.filter(
    (id) => !activeSet.has(id) && ALL_TOOLS[id],
  )
  const missing = Object.keys(ALL_TOOLS).filter(
    (id) => !activeSet.has(id) && !fromInactive.includes(id),
  )
  return [...fromInactive, ...missing]
})

/** Mirrors Rust `build_main_actions()` 1:1 so the preview matches the real floating toolbar */
const previewActions = computed(() => {
  const out: string[] = ['drag']
  let hasFinish = false
  for (const id of props.settings.toolbarActive) {
    if (id === 'cancel') continue
    if (id === 'finish') {
      hasFinish = true
      continue
    }
    if (ALL_TOOLS[id] && !out.includes(id)) out.push(id)
  }
  out.push('cancel')
  if (hasFinish || !props.settings.toolbarInactive.includes('finish')) {
    out.push('finish')
  }
  return out
})

function needsLeadingDivider(id: string, idx: number): boolean {
  if (idx === 1) return true
  return ['undo', 'record', 'save', 'cancel'].includes(id)
}

function syncInactiveFromActive(nextActive: string[]) {
  props.settings.toolbarActive = nextActive
  props.settings.toolbarInactive = Object.keys(ALL_TOOLS).filter((id) => !nextActive.includes(id))
}

function pointInEl(el: HTMLElement | null, x: number, y: number): boolean {
  if (!el) return false
  const r = el.getBoundingClientRect()
  return x >= r.left - 10 && x <= r.right + 10 && y >= r.top - 10 && y <= r.bottom + 10
}

function computeActiveInsertIndex(x: number, y: number, draggedId: string): number {
  const zone = activeZoneRef.value
  if (!zone) return props.settings.toolbarActive.length
  const chips = Array.from(zone.querySelectorAll<HTMLElement>('[data-chip-id]'))
  let bestIdx = 0
  let bestDist = Number.POSITIVE_INFINITY
  let visualIdx = 0
  for (const el of chips) {
    const cid = el.dataset.chipId
    if (!cid || cid === draggedId) continue
    const r = el.getBoundingClientRect()
    const cy = (r.top + r.bottom) / 2
    const cx = (r.left + r.right) / 2
    // Prioritize same-row distance strongly over horizontal distance
    const dy = y - cy
    const dx = x - cx
    const dist = Math.abs(dy) * 3.5 + Math.abs(dx)
    if (dist < bestDist) {
      bestDist = dist
      bestIdx = x > cx ? visualIdx + 1 : visualIdx
    }
    visualIdx++
  }
  return bestIdx
}

function onChipPointerDown(list: 'active' | 'inactive', id: string, e: PointerEvent) {
  if (e.button !== 0) return
  e.preventDefault()
  drag.value = {
    id,
    originList: list,
    startX: e.clientX,
    startY: e.clientY,
    curX: e.clientX,
    curY: e.clientY,
    dragging: false,
    hoverZone: list,
    dirty: false,
  }
  window.addEventListener('pointermove', onWindowPointerMove)
  window.addEventListener('pointerup', onWindowPointerUp)
  window.addEventListener('pointercancel', onWindowPointerCancel)
}

function onWindowPointerMove(e: PointerEvent) {
  const st = drag.value
  if (!st) return
  st.curX = e.clientX
  st.curY = e.clientY
  if (!st.dragging) {
    if (Math.hypot(e.clientX - st.startX, e.clientY - st.startY) < 5) return
    st.dragging = true
  }

  if (pointInEl(activeZoneRef.value, e.clientX, e.clientY)) {
    st.hoverZone = 'active'
    const without = props.settings.toolbarActive.filter((x) => x !== st.id)
    const targetIdx = Math.max(
      0,
      Math.min(computeActiveInsertIndex(e.clientX, e.clientY, st.id), without.length),
    )
    const curIdx = props.settings.toolbarActive.indexOf(st.id)
    if (curIdx !== targetIdx) {
      without.splice(targetIdx, 0, st.id)
      syncInactiveFromActive(without)
      st.dirty = true
    }
  } else if (pointInEl(inactiveZoneRef.value, e.clientX, e.clientY)) {
    st.hoverZone = 'inactive'
    if (props.settings.toolbarActive.includes(st.id) && props.settings.toolbarActive.length > 2) {
      const without = props.settings.toolbarActive.filter((x) => x !== st.id)
      syncInactiveFromActive(without)
      st.dirty = true
    }
  } else {
    st.hoverZone = null
  }
}

function cleanupDragListeners() {
  window.removeEventListener('pointermove', onWindowPointerMove)
  window.removeEventListener('pointerup', onWindowPointerUp)
  window.removeEventListener('pointercancel', onWindowPointerCancel)
}

function onWindowPointerUp() {
  const st = drag.value
  cleanupDragListeners()
  drag.value = null
  if (!st) return
  if (!st.dragging) {
    toggleTool(st.id)
  } else if (st.dirty) {
    emit('change')
  }
}

function onWindowPointerCancel() {
  cleanupDragListeners()
  const wasDirty = drag.value?.dirty
  drag.value = null
  if (wasDirty) emit('change')
}

onUnmounted(() => {
  cleanupDragListeners()
})

function toggleTool(id: string) {
  const inActive = props.settings.toolbarActive.includes(id)
  if (inActive) {
    if (props.settings.toolbarActive.length <= 2) return
    syncInactiveFromActive(props.settings.toolbarActive.filter((x) => x !== id))
  } else {
    syncInactiveFromActive([...props.settings.toolbarActive, id])
  }
  emit('change')
}

function resetToolbar() {
  props.settings.toolbarActive = [...DEFAULT_TOOLBAR_ACTIVE]
  props.settings.toolbarInactive = [...DEFAULT_TOOLBAR_INACTIVE]
  emit('change')
}

const previewMetrics = computed(() => {
  if (props.settings.toolbarSize === 'small') return { btn: 34, icon: 15 }
  if (props.settings.toolbarSize === 'large') return { btn: 44, icon: 20 }
  return { btn: 38, icon: 17 }
})

const previewPalette = computed(() => {
  const mode = props.settings.toolbarTheme
  if (mode === 'custom') {
    return {
      bg: props.settings.toolbarCustomBg,
      hover: props.settings.toolbarCustomHover,
      icon: props.settings.toolbarCustomIcon,
      accent: props.settings.toolbarCustomAccent,
      border: 'rgba(128,128,136,0.28)',
      divider: 'rgba(128,128,136,0.28)',
      activeBg: `${props.settings.toolbarCustomAccent}28`,
    }
  }
  const isLight = mode === 'light' || (mode === 'system' && props.settings.systemPrefersLight)
  if (isLight) {
    return {
      bg: '#fafafa',
      hover: '#e6e6e8',
      icon: '#45454a',
      accent: '#34c759',
      border: 'rgba(69,69,74,0.16)',
      divider: 'rgba(69,69,74,0.18)',
      activeBg: 'rgba(52,199,89,0.14)',
    }
  }
  return {
    bg: '#1e1e1f',
    hover: '#333336',
    icon: '#f2f2f5',
    accent: '#34c759',
    border: 'rgba(242,242,245,0.16)',
    divider: 'rgba(242,242,245,0.18)',
    activeBg: 'rgba(52,199,89,0.18)',
  }
})
</script>

<template>
  <div class="sn-panel">
    <div class="sn-section-head">
      <div>
        <h2 class="sn-title">外观设置</h2>
        <p class="sn-subtitle">双主题界面、DPI 自适应工具栏尺寸、配色与拖拽排序（所见即所得）</p>
      </div>
    </div>

    <div class="sn-list-card">
      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">设置中心主题</div>
          <div class="sn-row-desc">切换设置窗口的明暗外观风格</div>
        </div>
        <div class="sn-seg">
          <button
            v-for="m in [
              { id: 'system', label: '跟随系统' },
              { id: 'dark', label: '深色' },
              { id: 'light', label: '浅色' },
            ] as const"
            :key="m.id"
            type="button"
            class="sn-seg-btn"
            :class="{ active: settings.themeMode === m.id }"
            @click="settings.themeMode = m.id; emit('change')"
          >
            {{ m.label }}
          </button>
        </div>
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">截图悬浮工具栏主题</div>
          <div class="sn-row-desc">SnowAir 深灰 / 极简浅白，或启用 5 槽位自定义配色</div>
        </div>
        <div class="sn-seg">
          <button
            v-for="m in [
              { id: 'system', label: '跟随系统' },
              { id: 'dark', label: '深色' },
              { id: 'light', label: '浅色' },
              { id: 'custom', label: '自定义' },
            ] as const"
            :key="m.id"
            type="button"
            class="sn-seg-btn"
            :class="{ active: settings.toolbarTheme === m.id }"
            @click="settings.toolbarTheme = m.id; emit('change')"
          >
            {{ m.label }}
          </button>
        </div>
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">悬浮工具栏尺寸</div>
          <div class="sn-row-desc">自动适配高分屏 DPI 缩放，并支持小 / 中 / 大三档精细调节</div>
        </div>
        <div class="sn-seg">
          <button
            v-for="sz in [
              { id: 'small', label: '小' },
              { id: 'medium', label: '中 (推荐)' },
              { id: 'large', label: '大' },
            ] as const"
            :key="sz.id"
            type="button"
            class="sn-seg-btn"
            :class="{ active: settings.toolbarSize === sz.id }"
            @click="settings.toolbarSize = sz.id; emit('change')"
          >
            {{ sz.label }}
          </button>
        </div>
      </div>

      <div v-if="settings.toolbarTheme === 'custom'" class="custom-color-row">
        <label class="color-slot">
          <span>工具栏背景</span>
          <input v-model="settings.toolbarCustomBg" type="color" @input="emit('change')" />
        </label>
        <label class="color-slot">
          <span>悬停底色</span>
          <input v-model="settings.toolbarCustomHover" type="color" @input="emit('change')" />
        </label>
        <label class="color-slot">
          <span>图标颜色</span>
          <input v-model="settings.toolbarCustomIcon" type="color" @input="emit('change')" />
        </label>
        <label class="color-slot">
          <span>强调色</span>
          <input v-model="settings.toolbarCustomAccent" type="color" @input="emit('change')" />
        </label>
        <label class="color-slot">
          <span>选区边框</span>
          <input v-model="settings.toolbarCustomSelection" type="color" @input="emit('change')" />
        </label>
      </div>
    </div>

    <!-- 1:1 Live Floating Toolbar Preview & Drag-and-Drop Customizer -->
    <div class="tb-card">
      <div class="tb-head">
        <div>
          <div class="sn-row-title">工具栏实时预览与自定义排序</div>
          <div class="sn-row-desc">预览与实际截图悬浮工具栏 1:1 一致；拖拽调整顺序，点击可在启用与收纳间移动</div>
        </div>
        <button type="button" class="sn-ghost-btn" @click="resetToolbar">恢复默认</button>
      </div>

      <!-- Exact 1:1 SnowAir Floating Toolbar Preview -->
      <div class="tb-preview-stage">
        <div
          class="tb-live-bar"
          :style="{
            background: previewPalette.bg,
            borderColor: previewPalette.border,
            color: previewPalette.icon,
          }"
        >
          <template v-for="(id, idx) in previewActions" :key="id">
            <span
              v-if="idx > 0 && needsLeadingDivider(id, idx)"
              class="tb-live-div"
              :style="{ background: previewPalette.divider }"
            />
            <button
              type="button"
              class="tb-live-btn"
              :style="{
                width: `${previewMetrics.btn}px`,
                height: `${previewMetrics.btn}px`,
                color:
                  id === 'cancel'
                    ? '#ef4444'
                    : id === 'finish' || previewSelectedId === id
                      ? previewPalette.accent
                      : id === 'drag'
                        ? `${previewPalette.icon}80`
                        : previewPalette.icon,
                background: previewSelectedId === id ? previewPalette.activeBg : 'transparent',
              }"
              :title="id === 'drag' ? '拖拽移动' : id === 'cancel' ? '取消' : ALL_TOOLS[id]?.label || id"
              @click="id !== 'drag' && id !== 'cancel' && id !== 'finish' && (previewSelectedId = id)"
            >
              <ToolbarIcon :id="id" :size="previewMetrics.icon" />
              <span
                v-if="previewSelectedId === id"
                class="tb-live-ind"
                :style="{ background: previewPalette.accent }"
              />
            </button>
          </template>
        </div>
      </div>

      <div class="tb-zone-label">已启用工具栏（按住拖拽实时排序 · 单击隐藏）</div>
      <div
        ref="activeZoneRef"
        class="tb-zone"
        :class="{ 'drop-target': drag?.dragging && drag.hoverZone === 'active' }"
      >
        <div
          v-for="id in settings.toolbarActive"
          :key="id"
          :data-chip-id="id"
          class="tb-chip active"
          :class="{ ghost: drag?.dragging && drag.id === id }"
          @pointerdown="onChipPointerDown('active', id, $event)"
        >
          <span class="tb-grip" aria-hidden="true">⋮⋮</span>
          <ToolbarIcon :id="id" :size="14" />
          <span>{{ ALL_TOOLS[id]?.label || id }}</span>
        </div>
      </div>

      <div class="tb-zone-label">隐藏收纳托盘（拖回上方或单击启用）</div>
      <div
        ref="inactiveZoneRef"
        class="tb-zone muted"
        :class="{ 'drop-target': drag?.dragging && drag.hoverZone === 'inactive' }"
      >
        <div
          v-for="id in inactiveList"
          :key="id"
          :data-chip-id="id"
          class="tb-chip"
          :class="{ ghost: drag?.dragging && drag.id === id }"
          @pointerdown="onChipPointerDown('inactive', id, $event)"
        >
          <span class="tb-grip" aria-hidden="true">⋮⋮</span>
          <ToolbarIcon :id="id" :size="14" />
          <span>{{ ALL_TOOLS[id]?.label || id }}</span>
        </div>
        <span v-if="!inactiveList.length" class="tb-empty">全部工具均已显示在主工具栏中</span>
      </div>

      <!-- Floating pointer-drag preview pill -->
      <Teleport to="body">
        <div
          v-if="drag?.dragging"
          class="tb-drag-floating"
          :style="{ left: `${drag.curX}px`, top: `${drag.curY}px` }"
        >
          <span class="tb-grip">⋮⋮</span>
          <ToolbarIcon :id="drag.id" :size="14" />
          <span>{{ ALL_TOOLS[drag.id]?.label || drag.id }}</span>
        </div>
      </Teleport>
    </div>

    <!-- Default Annotation Styles -->
    <div class="sn-list-card" style="margin-top: 14px">
      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">默认标注颜色</div>
          <div class="sn-row-desc">打开截图画布时的初始画笔、形状与文字颜色</div>
        </div>
        <div class="swatch-row">
          <button
            v-for="c in PRESET_COLORS"
            :key="c"
            type="button"
            class="swatch-dot"
            :class="{ active: settings.annotationColor.toLowerCase() === c }"
            :style="{ background: c }"
            @click="settings.annotationColor = c; emit('change')"
          />
          <input
            v-model="settings.annotationColor"
            type="color"
            class="swatch-custom"
            title="自定义颜色"
            @input="emit('change')"
          />
        </div>
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">默认线条粗细 ({{ settings.strokeWidth }}px)</div>
          <div class="sn-row-desc">矩形、箭头、画笔、尺寸线等矢量标注的初始线宽</div>
        </div>
        <input
          v-model.number="settings.strokeWidth"
          type="range"
          min="1"
          max="14"
          class="sn-slider"
          @input="emit('change')"
        />
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">默认文字字号 ({{ settings.textSize }}px)</div>
          <div class="sn-row-desc">文字标注工具的初始字体大小</div>
        </div>
        <input
          v-model.number="settings.textSize"
          type="range"
          min="12"
          max="48"
          class="sn-slider"
          @input="emit('change')"
        />
      </div>

      <div class="sn-row">
        <div class="sn-row-info">
          <div class="sn-row-title">默认马赛克笔刷 ({{ settings.brushSize }}px)</div>
          <div class="sn-row-desc">马赛克与电子包浆工具的初始笔刷尺寸</div>
        </div>
        <input
          v-model.number="settings.brushSize"
          type="range"
          min="10"
          max="64"
          class="sn-slider"
          @input="emit('change')"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.custom-color-row {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  padding: 12px 16px;
  border-top: 1px solid var(--sn-border);
  background: var(--sn-subtle);
}
.color-slot {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--sn-muted);
}
.color-slot input[type='color'] {
  width: 28px;
  height: 24px;
  border: 1px solid var(--sn-border);
  border-radius: 6px;
  background: transparent;
  cursor: pointer;
}
.tb-card {
  margin-top: 14px;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
}
.tb-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.tb-preview-stage {
  display: flex;
  justify-content: center;
  padding: 20px 14px;
  border-radius: 12px;
  background:
    radial-gradient(circle at 20% 20%, rgba(52, 199, 89, 0.14), transparent 55%),
    var(--sn-card);
  border: 1px dashed var(--sn-border);
  margin-bottom: 14px;
  overflow-x: auto;
}
.tb-live-bar {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 5px 9px;
  border-radius: 12px;
  border: 1px solid;
  box-shadow:
    0 10px 28px rgba(0, 0, 0, 0.28),
    0 2px 6px rgba(0, 0, 0, 0.18);
}
.tb-live-div {
  width: 1px;
  height: 20px;
  margin: 0 3px;
  flex-shrink: 0;
}
.tb-live-btn {
  position: relative;
  border: none;
  border-radius: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  transition: transform 0.12s ease, background 0.12s ease;
}
.tb-live-btn:hover {
  transform: translateY(-1px);
}
.tb-live-ind {
  position: absolute;
  bottom: 3px;
  left: 50%;
  transform: translateX(-50%);
  width: 10px;
  height: 2.5px;
  border-radius: 99px;
}
.tb-zone-label {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--sn-muted);
  margin: 10px 0 6px;
}
.tb-zone {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  min-height: 44px;
  padding: 8px;
  border-radius: 10px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  transition: border-color 0.14s ease, background 0.14s ease;
}
.tb-zone.drop-target {
  border-color: var(--sn-accent);
  background: color-mix(in srgb, var(--sn-accent) 7%, var(--sn-card));
}
.tb-zone.muted {
  background: transparent;
  border-style: dashed;
}
.tb-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px 5px 7px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-text);
  font-size: 12px;
  cursor: grab;
  user-select: none;
  touch-action: none;
  transition: border-color 0.14s ease, transform 0.14s ease, opacity 0.14s ease;
}
.tb-chip:active {
  cursor: grabbing;
}
.tb-chip:hover {
  border-color: var(--sn-accent);
  transform: translateY(-1px);
}
.tb-chip.active {
  border-color: color-mix(in srgb, var(--sn-accent) 45%, var(--sn-border));
}
.tb-chip.ghost {
  opacity: 0.32;
  border-style: dashed;
  border-color: var(--sn-accent);
}
.tb-grip {
  font-size: 10px;
  letter-spacing: -1.5px;
  opacity: 0.45;
  margin-right: 1px;
}
.tb-drag-floating {
  position: fixed;
  z-index: 9999;
  pointer-events: none;
  transform: translate(-50%, -50%) scale(1.06);
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px 6px 8px;
  border-radius: 9px;
  border: 1.5px solid #34c759;
  background: #1e1e22;
  color: #f4f4f6;
  font-size: 12px;
  font-weight: 600;
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.36);
}
.tb-empty {
  font-size: 12px;
  color: var(--sn-muted);
  align-self: center;
  padding: 0 6px;
}
.swatch-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.swatch-dot {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  border: 2px solid transparent;
  cursor: pointer;
  box-shadow: inset 0 0 0 1px rgba(128, 128, 128, 0.35);
}
.swatch-dot.active {
  border-color: var(--sn-accent);
  transform: scale(1.12);
}
.swatch-custom {
  width: 26px;
  height: 24px;
  border: 1px solid var(--sn-border);
  border-radius: 6px;
  cursor: pointer;
  background: transparent;
}
</style>
