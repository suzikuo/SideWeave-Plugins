<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  chooseImages,
  copyImageDataUrl,
  exportDocument,
  openHistoryFile,
  readImageDataUrl,
  type ScreenshotSettings,
} from '../api'
import {
  cachedHistoryItems,
  cachedHistoryThumbs,
  ensureThumbnails,
  isImagePath,
  loadHistoryCached,
} from '../historyCache'
import {
  exportSceneToDataUrl,
  renderInteractiveCanvas,
} from '../canvas/renderer'
import {
  addImageElementToCanvas,
  autoArrangeImages,
  canvasElements,
  canvasImageCache,
  canvasNumberSeq,
  canvasPaper,
  canvasRedoStack,
  canvasUndoStack,
  canvasViewport,
  redoCanvas,
  snapshotCanvasForUndo,
  undoCanvas,
} from '../canvas/store'
import {
  cloneElement,
  FILL_PALETTE,
  getElementBounds,
  getMultiBounds,
  hitTestElement,
  hitTestHandles,
  newId,
  PAPER_BACKGROUNDS,
  rotatePoint,
  STROKE_PALETTE,
  type ArrowElement,
  type Bounds,
  type CanvasElement,
  type CanvasTool,
  type HandleId,
  type ImageElement,
  type PenElement,
  type Point,
  type TextElement,
} from '../canvas/types'

import ContextMenu, { type ContextMenuItem } from './ContextMenu.vue'

const props = defineProps<{
  settings: ScreenshotSettings
  incomingPath?: string | null
}>()

const emit = defineEmits<{
  toast: [message: string]
  consumedIncoming: []
}>()

const stageRef = ref<HTMLDivElement | null>(null)
const canvasRef = ref<HTMLCanvasElement | null>(null)
const fileInputRef = ref<HTMLInputElement | null>(null)
const textEditorRef = ref<HTMLTextAreaElement | null>(null)

const elements = canvasElements
const vp = canvasViewport
const paper = canvasPaper

const activeTool = ref<CanvasTool>('select')
const selectedIds = ref<Set<string>>(new Set())
const busy = ref(false)
const lastExportPath = ref<string | null>(null)
const showInspector = ref(true)
const showHistoryDrawer = ref(false)
const showExportMenu = ref(false)
const showArrangeMenu = ref(false)
const spaceHeld = ref(false)
const ctxMenu = ref<{ visible: boolean; x: number; y: number }>({
  visible: false,
  x: 0,
  y: 0,
})

const canvasCtxItems = computed<ContextMenuItem[]>(() => {
  const sel = selectedElements.value
  if (sel.length > 0) {
    const items: ContextMenuItem[] = [
      { id: 'duplicate', label: '复制一份', shortcut: 'Ctrl+D' },
      { id: 'front', label: '置于顶层' },
      { id: 'back', label: '置于底层' },
    ]
    if (sel.length === 1 && sel[0].type === 'image') {
      const imgEl = sel[0]
      items.push(
        { id: 'sep_img', divider: true },
        { id: 'flipX', label: '水平翻转' },
        { id: 'flipY', label: '垂直翻转' },
        { id: 'resetRot', label: '旋转归零', disabled: imgEl.rotation === 0 },
        { id: 'toggleShadow', label: '投影阴影', checked: imgEl.shadow },
      )
    } else if (sel.length === 1 && sel[0].type === 'text') {
      items.push(
        { id: 'sep_txt', divider: true },
        { id: 'editText', label: '编辑文字' },
      )
    }
    if (sel.length >= 2) {
      items.push(
        { id: 'sep_arr', divider: true },
        { id: 'arrangeRow', label: '横向对齐排布' },
        { id: 'arrangeGrid2', label: '2 列网格排布' },
      )
    }
    items.push(
      { id: 'sep_del', divider: true },
      { id: 'delete', label: `删除选中 (${sel.length})`, shortcut: 'Del', danger: true },
    )
    return items
  }
  return [
    { id: 'addImages', label: '+ 添加本地图片' },
    { id: 'openHistory', label: '从历史截图导入' },
    { id: 'sep_view', divider: true },
    { id: 'fitView', label: '适应视图', shortcut: 'Ctrl+0' },
    { id: 'resetZoom', label: '重置缩放 100%' },
    { id: 'toggleGrid', label: '显示点阵网格', checked: paper.showGrid },
    { id: 'toggleArtboard', label: '固定画板模式', checked: paper.artboard },
    { id: 'sep_exp', divider: true },
    { id: 'copyCanvas', label: '复制画布到剪贴板', disabled: !elements.value.length && !paper.artboard },
    { id: 'exportPng', label: '导出画布 .PNG', disabled: !elements.value.length && !paper.artboard },
    { id: 'sep_clr', divider: true },
    { id: 'clearAll', label: '清空画布', danger: true, disabled: !elements.value.length },
  ]
})

function onCanvasContextMenu(e: MouseEvent) {
  showExportMenu.value = false
  showArrangeMenu.value = false
  if (editingTextId.value) commitTextEdit()
  const world = screenToWorld(e.clientX, e.clientY)
  const hit = findTopHit(world)
  if (hit) {
    if (!selectedIds.value.has(hit.id)) {
      selectedIds.value = new Set([hit.id])
      requestRender()
    }
  } else {
    selectedIds.value = new Set()
    requestRender()
  }
  ctxMenu.value = { visible: true, x: e.clientX, y: e.clientY }
}

function onCanvasCtxSelect(id: string) {
  const single = primarySelected.value
  switch (id) {
    case 'duplicate':
      duplicateSelected()
      break
    case 'front':
      moveZOrder('front')
      break
    case 'back':
      moveZOrder('back')
      break
    case 'flipX':
      if (single?.type === 'image') {
        snapshotCanvasForUndo()
        single.flipX = !single.flipX
        requestRender()
      }
      break
    case 'flipY':
      if (single?.type === 'image') {
        snapshotCanvasForUndo()
        single.flipY = !single.flipY
        requestRender()
      }
      break
    case 'resetRot':
      if (single) {
        snapshotCanvasForUndo()
        single.rotation = 0
        requestRender()
      }
      break
    case 'toggleShadow':
      if (single?.type === 'image') {
        snapshotCanvasForUndo()
        single.shadow = !single.shadow
        requestRender()
      }
      break
    case 'editText':
      if (single?.type === 'text') beginTextEdit(single)
      break
    case 'arrangeRow':
      onArrange('row')
      break
    case 'arrangeGrid2':
      onArrange('grid2')
      break
    case 'delete':
      deleteSelected()
      break
    case 'addImages':
      void onPickImages()
      break
    case 'openHistory':
      void toggleHistoryDrawer()
      break
    case 'fitView':
      fitView()
      break
    case 'resetZoom':
      resetZoom()
      break
    case 'toggleGrid':
      paper.showGrid = !paper.showGrid
      break
    case 'toggleArtboard':
      paper.artboard = !paper.artboard
      if (paper.artboard) void nextTick(fitView)
      break
    case 'copyCanvas':
      void onCopyCanvas()
      break
    case 'exportPng':
      void onExportCanvas('png', 1)
      break
    case 'clearAll':
      clearCanvasAll()
      break
  }
}

// Default drawing style state
const drawStroke = ref('#ef4444')
const drawFill = ref('transparent')
const drawWidth = ref(4)
const drawDashed = ref(false)
const drawFontSize = ref(24)
const drawMosaicSize = ref(14)

// Inline text editing state
const editingTextId = ref<string | null>(null)
const editingTextDraft = ref('')

// Transient pointer interaction state
const draftElement = ref<CanvasElement | null>(null)
const marquee = ref<Bounds | null>(null)
const liveSizeBadge = ref(false)

type InteractionMode =
  | { kind: 'none' }
  | { kind: 'pan'; startScreen: Point; origPanX: number; origPanY: number }
  | { kind: 'move'; startWorld: Point; snapshots: Map<string, CanvasElement>; moved: boolean }
  | {
      kind: 'resize'
      id: string
      handle: HandleId
      orig: CanvasElement
      origBounds: Bounds
      startWorld: Point
    }
  | { kind: 'rotate'; id: string; cx: number; cy: number; startAngle: number; origRot: number }
  | { kind: 'marquee'; startWorld: Point }
  | { kind: 'draw'; startWorld: Point }
  | { kind: 'erase'; erasedAny: boolean }

let interaction: InteractionMode = { kind: 'none' }
let rafHandle: number | null = null
let resizeObserver: ResizeObserver | null = null

const isDarkTheme = computed(() => {
  if (props.settings.themeMode === 'dark') return true
  if (props.settings.themeMode === 'light') return false
  return !props.settings.systemPrefersLight
})

const selectedElements = computed(() =>
  elements.value.filter((e) => selectedIds.value.has(e.id)),
)
const primarySelected = computed<CanvasElement | null>(() =>
  selectedElements.value.length === 1 ? selectedElements.value[0] : null,
)
const historyPickerItems = computed(() =>
  cachedHistoryItems.value.filter((it) => isImagePath(it.path)).slice(0, 24),
)
const imageCount = computed(() => elements.value.filter((e) => e.type === 'image').length)

const canvasCursor = computed(() => {
  if (spaceHeld.value || activeTool.value === 'hand') {
    return interaction.kind === 'pan' ? 'grabbing' : 'grab'
  }
  if (activeTool.value === 'select') return 'default'
  if (activeTool.value === 'text') return 'text'
  return 'crosshair'
})

function requestRender() {
  if (rafHandle !== null) return
  rafHandle = window.requestAnimationFrame(() => {
    rafHandle = null
    const canvas = canvasRef.value
    if (!canvas) return
    renderInteractiveCanvas(canvas, {
      elements: elements.value,
      imageCache: canvasImageCache,
      viewport: vp,
      paper,
      selectedIds: selectedIds.value,
      editingTextId: editingTextId.value,
      marquee: marquee.value,
      draftElement: draftElement.value,
      isDarkTheme: isDarkTheme.value,
      showLiveSizeBadge: liveSizeBadge.value,
    })
  })
}

watch(
  [
    elements,
    selectedIds,
    isDarkTheme,
    () => vp.panX,
    () => vp.panY,
    () => vp.zoom,
    () => paper.artboard,
    () => paper.width,
    () => paper.height,
    () => paper.bg,
    () => paper.showGrid,
  ],
  () => requestRender(),
  { deep: true },
)

function screenToWorld(clientX: number, clientY: number): Point {
  const rect = canvasRef.value!.getBoundingClientRect()
  return {
    x: (clientX - rect.left - vp.panX) / vp.zoom,
    y: (clientY - rect.top - vp.panY) / vp.zoom,
  }
}

function viewportCenterWorld(): Point {
  const rect = canvasRef.value?.getBoundingClientRect()
  if (!rect) return { x: 400, y: 300 }
  return {
    x: (rect.width * 0.5 - vp.panX) / vp.zoom,
    y: (rect.height * 0.5 - vp.panY) / vp.zoom,
  }
}

function fitView() {
  const rect = canvasRef.value?.getBoundingClientRect()
  if (!rect || rect.width < 10 || rect.height < 10) return
  const target: Bounds | null = paper.artboard
    ? { x: 0, y: 0, w: paper.width, h: paper.height }
    : getMultiBounds(elements.value)
  if (!target) {
    vp.zoom = 1
    vp.panX = rect.width * 0.5 - 320
    vp.panY = rect.height * 0.5 - 200
    requestRender()
    return
  }
  const pad = 72
  const availW = Math.max(120, rect.width - pad * 2)
  const availH = Math.max(120, rect.height - pad * 2)
  const nextZoom = Math.max(0.15, Math.min(1.5, Math.min(availW / target.w, availH / target.h)))
  vp.zoom = Number(nextZoom.toFixed(3))
  vp.panX = rect.width * 0.5 - (target.x + target.w * 0.5) * vp.zoom
  vp.panY = rect.height * 0.5 - (target.y + target.h * 0.5) * vp.zoom
  requestRender()
}

function zoomBy(factor: number, anchor?: Point) {
  const rect = canvasRef.value?.getBoundingClientRect()
  if (!rect) return
  const ax = anchor ? anchor.x : rect.width * 0.5
  const ay = anchor ? anchor.y : rect.height * 0.5
  const wx = (ax - vp.panX) / vp.zoom
  const wy = (ay - vp.panY) / vp.zoom
  const next = Math.max(0.1, Math.min(6, vp.zoom * factor))
  vp.zoom = next
  vp.panX = ax - wx * next
  vp.panY = ay - wy * next
  requestRender()
}

function resetZoom() {
  const rect = canvasRef.value?.getBoundingClientRect()
  if (!rect) return
  zoomBy(1 / vp.zoom)
}

function fileName(path: string): string {
  const p = path.split(/[\\/]/)
  return p[p.length - 1] || path
}

async function addPaths(paths: string[]) {
  if (!paths.length) return
  busy.value = true
  let added = 0
  const newIds: string[] = []
  try {
    snapshotCanvasForUndo()
    const center = viewportCenterWorld()
    for (let i = 0; i < paths.length; i++) {
      const p = paths[i]
      const res = await readImageDataUrl(p, 1600)
      const el = await addImageElementToCanvas({
        src: res.dataUrl,
        path: p,
        name: fileName(p),
        width: res.width,
        height: res.height,
        centerWorld: center,
        offsetIndex: i,
      })
      newIds.push(el.id)
      added++
    }
    if (newIds.length) {
      selectedIds.value = new Set(newIds)
      activeTool.value = 'select'
    }
    if (elements.value.length === added) {
      await nextTick()
      fitView()
    }
    requestRender()
    if (added > 0) emit('toast', `已添加到画布（${added} 张图片）`)
  } catch (err) {
    emit('toast', `导入图片失败：${String(err)}`)
  } finally {
    busy.value = false
  }
}

async function addLocalFiles(files: FileList | File[]) {
  const list = Array.from(files).filter((f) => f.type.startsWith('image/'))
  if (!list.length) return
  busy.value = true
  try {
    snapshotCanvasForUndo()
    const center = viewportCenterWorld()
    const newIds: string[] = []
    for (let i = 0; i < list.length; i++) {
      const file = list[i]
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onload = () => resolve(String(reader.result || ''))
        reader.onerror = () => reject(new Error(`读取文件失败：${file.name}`))
        reader.readAsDataURL(file)
      })
      if (!dataUrl) continue
      const el = await addImageElementToCanvas({
        src: dataUrl,
        path: '',
        name: file.name || `画布图片_${elements.value.length + 1}.png`,
        centerWorld: center,
        offsetIndex: i,
      })
      newIds.push(el.id)
    }
    if (newIds.length) {
      selectedIds.value = new Set(newIds)
      activeTool.value = 'select'
      if (elements.value.length === newIds.length) {
        await nextTick()
        fitView()
      }
      requestRender()
      emit('toast', `已添加到画布（${newIds.length} 张图片）`)
    }
  } catch (err) {
    emit('toast', `导入失败：${String(err)}`)
  } finally {
    busy.value = false
  }
}

async function onPickImages() {
  if (busy.value) return
  showExportMenu.value = false
  showArrangeMenu.value = false
  try {
    const paths = await chooseImages()
    if (paths.length) {
      await addPaths(paths)
    }
  } catch {
    fileInputRef.value?.click()
  }
}

function onFileInputChange(e: Event) {
  const input = e.target as HTMLInputElement
  if (input.files && input.files.length) {
    void addLocalFiles(input.files)
  }
  input.value = ''
}

async function toggleHistoryDrawer() {
  showHistoryDrawer.value = !showHistoryDrawer.value
  showExportMenu.value = false
  showArrangeMenu.value = false
  if (showHistoryDrawer.value) {
    try {
      const list = await loadHistoryCached(false)
      void ensureThumbnails(list, 24)
    } catch {
      // ignore
    }
  }
}

// Inline text editing overlay geometry
const editingTextStyle = computed(() => {
  if (!editingTextId.value) return {}
  const el = elements.value.find((e): e is TextElement => e.id === editingTextId.value && e.type === 'text')
  if (!el) return {}
  return {
    left: `${el.x * vp.zoom + vp.panX}px`,
    top: `${el.y * vp.zoom + vp.panY}px`,
    fontSize: `${Math.max(12, el.fontSize * vp.zoom)}px`,
    fontWeight: el.bold ? '700' : '500',
    color: el.color,
    background: el.bgColor && el.bgColor !== 'transparent' ? el.bgColor : 'rgba(20,20,24,0.65)',
    minWidth: `${Math.max(100, el.w * vp.zoom)}px`,
  }
})

function measureTextElementSize(text: string, fontSize: number, bold: boolean): { w: number; h: number } {
  const c = document.createElement('canvas')
  const ctx = c.getContext('2d')
  const lines = (text || ' ').split('\n')
  let maxW = 60
  if (ctx) {
    ctx.font = `${bold ? '700' : '500'} ${fontSize}px "Microsoft YaHei UI", "Segoe UI", sans-serif`
    for (const line of lines) {
      maxW = Math.max(maxW, ctx.measureText(line || ' ').width)
    }
  }
  return {
    w: Math.ceil(maxW + 18),
    h: Math.ceil(lines.length * fontSize * 1.32 + 12),
  }
}

function beginTextEdit(el: TextElement) {
  editingTextId.value = el.id
  editingTextDraft.value = el.text
  selectedIds.value = new Set([el.id])
  requestRender()
  void nextTick(() => {
    textEditorRef.value?.focus()
    textEditorRef.value?.select()
  })
}

function commitTextEdit() {
  if (!editingTextId.value) return
  const id = editingTextId.value
  const val = editingTextDraft.value.trim()
  editingTextId.value = null
  const idx = elements.value.findIndex((e) => e.id === id)
  if (idx >= 0 && elements.value[idx].type === 'text') {
    if (!val) {
      elements.value.splice(idx, 1)
      selectedIds.value = new Set()
    } else {
      const el = elements.value[idx] as TextElement
      const dims = measureTextElementSize(val, el.fontSize, el.bold)
      el.text = val
      el.w = dims.w
      el.h = dims.h
    }
  }
  requestRender()
}

// Pointer event handlers
function onCanvasWheel(e: WheelEvent) {
  e.preventDefault()
  const rect = canvasRef.value!.getBoundingClientRect()
  const anchor = { x: e.clientX - rect.left, y: e.clientY - rect.top }
  const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12
  zoomBy(factor, anchor)
}

function onPointerDown(e: PointerEvent) {
  showExportMenu.value = false
  showArrangeMenu.value = false
  if (editingTextId.value) {
    commitTextEdit()
  }
  const canvas = canvasRef.value
  if (!canvas) return
  canvas.setPointerCapture(e.pointerId)

  const world = screenToWorld(e.clientX, e.clientY)

  // Panning via Middle mouse button, Spacebar, or Hand tool
  if (e.button === 1 || spaceHeld.value || activeTool.value === 'hand') {
    interaction = {
      kind: 'pan',
      startScreen: { x: e.clientX, y: e.clientY },
      origPanX: vp.panX,
      origPanY: vp.panY,
    }
    return
  }
  if (e.button !== 0) return

  // Eraser tool
  if (activeTool.value === 'eraser') {
    const hit = findTopHit(world)
    if (hit) {
      snapshotCanvasForUndo()
      elements.value = elements.value.filter((x) => x.id !== hit.id)
      selectedIds.value.delete(hit.id)
      interaction = { kind: 'erase', erasedAny: true }
      requestRender()
    } else {
      interaction = { kind: 'erase', erasedAny: false }
    }
    return
  }

  // Numbered callout badge tool
  if (activeTool.value === 'number') {
    snapshotCanvasForUndo()
    const size = 32
    const el: CanvasElement = {
      id: newId('num'),
      type: 'number',
      x: world.x - size * 0.5,
      y: world.y - size * 0.5,
      w: size,
      h: size,
      rotation: 0,
      opacity: 1,
      value: canvasNumberSeq.value++,
      color: drawStroke.value,
    }
    elements.value.push(el)
    selectedIds.value = new Set([el.id])
    requestRender()
    return
  }

  // Text tool
  if (activeTool.value === 'text') {
    snapshotCanvasForUndo()
    const el: TextElement = {
      id: newId('txt'),
      type: 'text',
      x: world.x,
      y: world.y - drawFontSize.value * 0.5,
      w: 140,
      h: Math.round(drawFontSize.value * 1.4 + 10),
      rotation: 0,
      opacity: 1,
      text: '双击编辑文字',
      fontSize: drawFontSize.value,
      color: drawStroke.value,
      bgColor: 'transparent',
      bold: true,
    }
    elements.value.push(el)
    activeTool.value = 'select'
    beginTextEdit(el)
    return
  }

  // Select tool
  if (activeTool.value === 'select') {
    // 1. Check resize/rotate handles of single selected element
    if (primarySelected.value && !primarySelected.value.locked) {
      const handle = hitTestHandles(primarySelected.value, world, vp.zoom)
      if (handle === 'rotate') {
        const b = getElementBounds(primarySelected.value)
        const cx = b.x + b.w * 0.5
        const cy = b.y + b.h * 0.5
        snapshotCanvasForUndo()
        interaction = {
          kind: 'rotate',
          id: primarySelected.value.id,
          cx,
          cy,
          startAngle: Math.atan2(world.y - cy, world.x - cx),
          origRot: primarySelected.value.rotation,
        }
        return
      }
      if (handle) {
        snapshotCanvasForUndo()
        liveSizeBadge.value = true
        interaction = {
          kind: 'resize',
          id: primarySelected.value.id,
          handle,
          orig: cloneElement(primarySelected.value),
          origBounds: getElementBounds(primarySelected.value),
          startWorld: world,
        }
        return
      }
    }

    // 2. Check element hit from top to bottom
    const hit = findTopHit(world)
    if (hit) {
      if (e.shiftKey) {
        const next = new Set(selectedIds.value)
        if (next.has(hit.id)) next.delete(hit.id)
        else next.add(hit.id)
        selectedIds.value = next
      } else if (!selectedIds.value.has(hit.id)) {
        selectedIds.value = new Set([hit.id])
      }
      const snapshots = new Map<string, CanvasElement>()
      for (const el of elements.value) {
        if (selectedIds.value.has(el.id) && !el.locked) {
          snapshots.set(el.id, cloneElement(el))
        }
      }
      if (snapshots.size > 0) {
        interaction = { kind: 'move', startWorld: world, snapshots, moved: false }
      }
      requestRender()
      return
    }

    // 3. Start rubber-band marquee selection
    if (!e.shiftKey) selectedIds.value = new Set()
    marquee.value = { x: world.x, y: world.y, w: 0, h: 0 }
    interaction = { kind: 'marquee', startWorld: world }
    requestRender()
    return
  }

  // Drawing tools: rect / ellipse / arrow / line / pen / highlighter / mosaic
  const id = newId(activeTool.value)
  if (activeTool.value === 'pen' || activeTool.value === 'highlighter') {
    draftElement.value = {
      id,
      type: 'pen',
      x: world.x,
      y: world.y,
      w: 1,
      h: 1,
      rotation: 0,
      opacity: 1,
      points: [world],
      stroke: activeTool.value === 'highlighter' ? '#facc15' : drawStroke.value,
      strokeWidth: activeTool.value === 'highlighter' ? Math.max(18, drawWidth.value * 4) : drawWidth.value,
      highlighter: activeTool.value === 'highlighter',
    }
  } else if (activeTool.value === 'arrow' || activeTool.value === 'line') {
    draftElement.value = {
      id,
      type: activeTool.value,
      x: world.x,
      y: world.y,
      x2: world.x,
      y2: world.y,
      w: 1,
      h: 1,
      rotation: 0,
      opacity: 1,
      stroke: drawStroke.value,
      strokeWidth: drawWidth.value,
      dashed: drawDashed.value,
    }
  } else if (activeTool.value === 'rect' || activeTool.value === 'ellipse') {
    draftElement.value = {
      id,
      type: activeTool.value,
      x: world.x,
      y: world.y,
      w: 1,
      h: 1,
      rotation: 0,
      opacity: 1,
      stroke: drawStroke.value,
      fill: drawFill.value,
      strokeWidth: drawWidth.value,
      radius: activeTool.value === 'rect' ? 8 : 0,
      dashed: drawDashed.value,
    } as CanvasElement
  } else if (activeTool.value === 'mosaic') {
    draftElement.value = {
      id,
      type: 'mosaic',
      x: world.x,
      y: world.y,
      w: 1,
      h: 1,
      rotation: 0,
      opacity: 1,
      pixelSize: drawMosaicSize.value,
    }
  }
  interaction = { kind: 'draw', startWorld: world }
  requestRender()
}

function onPointerMove(e: PointerEvent) {
  if (interaction.kind === 'none') return
  const world = screenToWorld(e.clientX, e.clientY)

  if (interaction.kind === 'pan') {
    vp.panX = interaction.origPanX + (e.clientX - interaction.startScreen.x)
    vp.panY = interaction.origPanY + (e.clientY - interaction.startScreen.y)
    requestRender()
    return
  }

  if (interaction.kind === 'erase') {
    const hit = findTopHit(world)
    if (hit) {
      if (!interaction.erasedAny) {
        snapshotCanvasForUndo()
        interaction.erasedAny = true
      }
      elements.value = elements.value.filter((x) => x.id !== hit.id)
      selectedIds.value.delete(hit.id)
      requestRender()
    }
    return
  }

  if (interaction.kind === 'move') {
    const dx = world.x - interaction.startWorld.x
    const dy = world.y - interaction.startWorld.y
    if (!interaction.moved && Math.hypot(dx, dy) > 2 / vp.zoom) {
      snapshotCanvasForUndo()
      interaction.moved = true
    }
    if (!interaction.moved) return
    for (const el of elements.value) {
      const orig = interaction.snapshots.get(el.id)
      if (!orig) continue
      el.x = orig.x + dx
      el.y = orig.y + dy
      if (el.type === 'arrow' || el.type === 'line') {
        const ao = orig as ArrowElement
        el.x2 = ao.x2 + dx
        el.y2 = ao.y2 + dy
      } else if (el.type === 'pen') {
        const po = orig as PenElement
        el.points = po.points.map((p) => ({ x: p.x + dx, y: p.y + dy }))
      }
    }
    requestRender()
    return
  }

  if (interaction.kind === 'rotate') {
    const el = elements.value.find((x) => x.id === interaction.id)
    if (!el) return
    const ang = Math.atan2(world.y - interaction.cy, world.x - interaction.cx)
    let rot = interaction.origRot + (ang - interaction.startAngle)
    if (e.shiftKey) {
      const snap = Math.PI / 12 // 15 deg
      rot = Math.round(rot / snap) * snap
    }
    el.rotation = rot
    requestRender()
    return
  }

  if (interaction.kind === 'resize') {
    const el = elements.value.find((x) => x.id === interaction.id)
    if (!el) return
    const ob = interaction.origBounds
    const cx = ob.x + ob.w * 0.5
    const cy = ob.y + ob.h * 0.5
    const startLocal = rotatePoint(interaction.startWorld, cx, cy, -interaction.orig.rotation)
    const curLocal = rotatePoint(world, cx, cy, -interaction.orig.rotation)
    const dx = curLocal.x - startLocal.x
    const dy = curLocal.y - startLocal.y
    let left = ob.x
    let top = ob.y
    let right = ob.x + ob.w
    let bottom = ob.y + ob.h
    const h = interaction.handle
    if (h.includes('w')) left = Math.min(right - 16, ob.x + dx)
    if (h.includes('e')) right = Math.max(left + 16, ob.x + ob.w + dx)
    if (h.includes('n')) top = Math.min(bottom - 16, ob.y + dy)
    if (h.includes('s')) bottom = Math.max(top + 16, ob.y + ob.h + dy)

    // Keep aspect ratio on corner resize for images by default (hold Shift to free-stretch),
    // and for shapes when holding Shift.
    const isCorner = h.length === 2
    const keepRatio =
      isCorner && ((el.type === 'image' && !e.shiftKey) || (el.type !== 'image' && e.shiftKey))
    if (keepRatio && ob.w > 0 && ob.h > 0) {
      const ratio = ob.w / ob.h
      const nw = right - left
      const nh = Math.max(16, nw / ratio)
      if (h.includes('n')) top = bottom - nh
      else bottom = top + nh
    }

    el.x = left
    el.y = top
    el.w = Math.max(16, right - left)
    el.h = Math.max(16, bottom - top)
    requestRender()
    return
  }

  if (interaction.kind === 'marquee') {
    const sx = interaction.startWorld.x
    const sy = interaction.startWorld.y
    marquee.value = {
      x: Math.min(sx, world.x),
      y: Math.min(sy, world.y),
      w: Math.abs(world.x - sx),
      h: Math.abs(world.y - sy),
    }
    const m = marquee.value
    const next = new Set<string>()
    for (const el of elements.value) {
      const b = getElementBounds(el)
      const intersects =
        b.x <= m.x + m.w && b.x + b.w >= m.x && b.y <= m.y + m.h && b.y + b.h >= m.y
      if (intersects) next.add(el.id)
    }
    selectedIds.value = next
    requestRender()
    return
  }

  if (interaction.kind === 'draw' && draftElement.value) {
    const d = draftElement.value
    const sx = interaction.startWorld.x
    const sy = interaction.startWorld.y
    if (d.type === 'pen') {
      const last = d.points[d.points.length - 1]
      if (!last || Math.hypot(world.x - last.x, world.y - last.y) >= 2 / vp.zoom) {
        d.points.push(world)
      }
    } else if (d.type === 'arrow' || d.type === 'line') {
      let tx = world.x
      let ty = world.y
      if (e.shiftKey) {
        const ang = Math.atan2(ty - sy, tx - sx)
        const snap = Math.round(ang / (Math.PI / 4)) * (Math.PI / 4)
        const dist = Math.hypot(tx - sx, ty - sy)
        tx = sx + Math.cos(snap) * dist
        ty = sy + Math.sin(snap) * dist
      }
      d.x2 = tx
      d.y2 = ty
    } else {
      let w = world.x - sx
      let h = world.y - sy
      if (e.shiftKey) {
        const side = Math.max(Math.abs(w), Math.abs(h))
        w = Math.sign(w || 1) * side
        h = Math.sign(h || 1) * side
      }
      d.x = w >= 0 ? sx : sx + w
      d.y = h >= 0 ? sy : sy + h
      d.w = Math.max(4, Math.abs(w))
      d.h = Math.max(4, Math.abs(h))
    }
    requestRender()
  }
}

function onPointerUp(e: PointerEvent) {
  const canvas = canvasRef.value
  if (canvas?.hasPointerCapture(e.pointerId)) {
    canvas.releasePointerCapture(e.pointerId)
  }
  liveSizeBadge.value = false
  if (interaction.kind === 'marquee') {
    marquee.value = null
  }
  if (interaction.kind === 'draw' && draftElement.value) {
    const d = draftElement.value
    draftElement.value = null
    const valid =
      d.type === 'pen'
        ? d.points.length >= 2
        : d.type === 'arrow' || d.type === 'line'
          ? Math.hypot(d.x2 - d.x, d.y2 - d.y) > 5
          : d.w > 6 && d.h > 6
    if (valid) {
      snapshotCanvasForUndo()
      elements.value.push(d)
      selectedIds.value = new Set([d.id])
      if (d.type !== 'pen') {
        activeTool.value = 'select'
      }
    }
  }
  interaction = { kind: 'none' }
  requestRender()
}

function onDoubleClick(e: MouseEvent) {
  const world = screenToWorld(e.clientX, e.clientY)
  const hit = findTopHit(world)
  if (hit && hit.type === 'text') {
    beginTextEdit(hit)
  }
}

function findTopHit(world: Point): CanvasElement | null {
  for (let i = elements.value.length - 1; i >= 0; i--) {
    if (hitTestElement(elements.value[i], world, vp.zoom)) {
      return elements.value[i]
    }
  }
  return null
}

// Element actions
function deleteSelected() {
  if (!selectedIds.value.size) return
  snapshotCanvasForUndo()
  elements.value = elements.value.filter((e) => !selectedIds.value.has(e.id))
  selectedIds.value = new Set()
  requestRender()
}

function duplicateSelected() {
  if (!selectedIds.value.size) return
  snapshotCanvasForUndo()
  const copies: CanvasElement[] = []
  for (const el of elements.value) {
    if (!selectedIds.value.has(el.id)) continue
    const c = cloneElement(el)
    c.id = newId(el.type)
    c.x += 24
    c.y += 24
    if (c.type === 'arrow' || c.type === 'line') {
      c.x2 += 24
      c.y2 += 24
    } else if (c.type === 'pen') {
      c.points = c.points.map((p) => ({ x: p.x + 24, y: p.y + 24 }))
    } else if (c.type === 'image') {
      const img = canvasImageCache.get(el.id)
      if (img) canvasImageCache.set(c.id, img)
    }
    copies.push(c)
  }
  elements.value.push(...copies)
  selectedIds.value = new Set(copies.map((c) => c.id))
  requestRender()
}

function moveZOrder(dir: 'front' | 'back') {
  if (!selectedIds.value.size) return
  snapshotCanvasForUndo()
  const sel = elements.value.filter((e) => selectedIds.value.has(e.id))
  const rest = elements.value.filter((e) => !selectedIds.value.has(e.id))
  elements.value = dir === 'front' ? [...rest, ...sel] : [...sel, ...rest]
  requestRender()
}

function onArrange(layout: 'row' | 'col' | 'grid2' | 'grid3') {
  showArrangeMenu.value = false
  if (autoArrangeImages(layout, selectedIds.value)) {
    void nextTick(fitView)
    emit('toast', '已按布局自动排版画布图片')
  } else {
    emit('toast', '请先添加图片到画布')
  }
}

function clearCanvasAll() {
  if (!elements.value.length) return
  snapshotCanvasForUndo()
  elements.value = []
  selectedIds.value = new Set()
  canvasNumberSeq.value = 1
  requestRender()
}

function onUndo() {
  if (undoCanvas()) {
    selectedIds.value = new Set()
    requestRender()
  }
}

function onRedo() {
  if (redoCanvas()) {
    selectedIds.value = new Set()
    requestRender()
  }
}

// Export & Copy
async function onCopyCanvas() {
  if (!elements.value.length && !paper.artboard) {
    emit('toast', '画布为空，请先添加图片或绘制内容')
    return
  }
  busy.value = true
  try {
    const rendered = exportSceneToDataUrl(elements.value, canvasImageCache, paper, 'image/png', 1)
    if (!rendered) throw new Error('画布无可导出内容')
    await copyImageDataUrl(rendered.dataUrl)
    emit('toast', `画布已复制到剪贴板（${rendered.width}×${rendered.height}）`)
  } catch (err) {
    emit('toast', `复制失败：${String(err)}`)
  } finally {
    busy.value = false
  }
}

async function onExportCanvas(format: 'png' | 'jpg' | 'webp' | 'pdf', scale = 1) {
  showExportMenu.value = false
  if (!elements.value.length && !paper.artboard) {
    emit('toast', '画布为空，请先添加图片或绘制内容')
    return
  }
  busy.value = true
  try {
    const mime = format === 'jpg' ? 'image/jpeg' : format === 'webp' ? 'image/webp' : 'image/png'
    const rendered = exportSceneToDataUrl(elements.value, canvasImageCache, paper, mime, scale)
    if (!rendered) throw new Error('画布无可导出内容')
    const res = await exportDocument({
      pages: [{ source: rendered.dataUrl }],
      format,
      quality: 94,
    })
    lastExportPath.value = res.path
    emit('toast', `已导出 ${format.toUpperCase()}（${res.width}×${res.height}）`)
  } catch (err) {
    emit('toast', `导出失败：${String(err)}`)
  } finally {
    busy.value = false
  }
}

async function onExportMultiDoc(format: 'pdf' | 'pptx') {
  showExportMenu.value = false
  showArrangeMenu.value = false
  const imgs = elements.value.filter((e): e is ImageElement => e.type === 'image')
  if (!imgs.length) {
    emit('toast', '画布中没有图片，请先添加图片')
    return
  }
  busy.value = true
  try {
    const res = await exportDocument({
      pages: imgs.map((im) => ({ source: im.path || im.src })),
      format,
      quality: 92,
    })
    lastExportPath.value = res.path
    emit('toast', `已将 ${imgs.length} 张画布图片打包为 .${format.toUpperCase()}`)
  } catch (err) {
    emit('toast', `打包失败：${String(err)}`)
  } finally {
    busy.value = false
  }
}

// Keyboard & Clipboard shortcuts
function onKeyDown(e: KeyboardEvent) {
  const tag = (e.target as HTMLElement | null)?.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

  if (e.code === 'Space' && !e.repeat) {
    e.preventDefault()
    spaceHeld.value = true
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
    e.preventDefault()
    if (e.shiftKey) onRedo()
    else onUndo()
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') {
    e.preventDefault()
    onRedo()
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'd') {
    e.preventDefault()
    duplicateSelected()
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key === '0') {
    e.preventDefault()
    fitView()
    return
  }
  if (e.key === 'Delete' || e.key === 'Backspace') {
    if (selectedIds.value.size) {
      e.preventDefault()
      deleteSelected()
    }
    return
  }
  if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key) && selectedIds.value.size) {
    e.preventDefault()
    const step = e.shiftKey ? 10 : 1
    const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0
    const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0
    for (const el of elements.value) {
      if (!selectedIds.value.has(el.id) || el.locked) continue
      el.x += dx
      el.y += dy
      if (el.type === 'arrow' || el.type === 'line') {
        el.x2 += dx
        el.y2 += dy
      } else if (el.type === 'pen') {
        el.points = el.points.map((p) => ({ x: p.x + dx, y: p.y + dy }))
      }
    }
    requestRender()
    return
  }
  if (!e.ctrlKey && !e.metaKey && !e.altKey) {
    const map: Record<string, CanvasTool> = {
      v: 'select',
      h: 'hand',
      r: 'rect',
      o: 'ellipse',
      a: 'arrow',
      l: 'line',
      p: 'pen',
      t: 'text',
      n: 'number',
      m: 'mosaic',
      e: 'eraser',
    }
    const next = map[e.key.toLowerCase()]
    if (next) activeTool.value = next
  }
}

function onKeyUp(e: KeyboardEvent) {
  if (e.code === 'Space') spaceHeld.value = false
}

function onPaste(e: ClipboardEvent) {
  const items = e.clipboardData?.items
  if (!items) return
  const files: File[] = []
  for (const item of Array.from(items)) {
    if (item.type.startsWith('image/')) {
      const f = item.getAsFile()
      if (f) files.push(f)
    }
  }
  if (files.length) {
    e.preventDefault()
    void addLocalFiles(files)
  }
}

function applyPaperRatio(w: number, h: number) {
  paper.artboard = true
  paper.width = w
  paper.height = h
  void nextTick(fitView)
}

watch(
  () => props.incomingPath,
  (p) => {
    if (p) {
      void addPaths([p])
      emit('consumedIncoming')
    }
  },
  { immediate: true },
)

onMounted(() => {
  window.addEventListener('keydown', onKeyDown)
  window.addEventListener('keyup', onKeyUp)
  window.addEventListener('paste', onPaste)
  if (stageRef.value) {
    resizeObserver = new ResizeObserver(() => requestRender())
    resizeObserver.observe(stageRef.value)
  }
  void nextTick(() => {
    fitView()
    requestRender()
  })
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeyDown)
  window.removeEventListener('keyup', onKeyUp)
  window.removeEventListener('paste', onPaste)
  resizeObserver?.disconnect()
  if (rafHandle !== null) window.cancelAnimationFrame(rafHandle)
})
</script>

<template>
  <div ref="stageRef" class="cv-root">
    <input
      ref="fileInputRef"
      type="file"
      multiple
      accept="image/*"
      style="display: none"
      @change="onFileInputChange"
    />

    <!-- Full-pane Interactive HiDPI Canvas -->
    <canvas
      ref="canvasRef"
      class="cv-canvas"
      :style="{ cursor: canvasCursor }"
      @wheel="onCanvasWheel"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @dblclick="onDoubleClick"
      @contextmenu.prevent="onCanvasContextMenu"
    />

    <!-- Inline Floating Textarea for Direct Canvas Text Editing -->
    <textarea
      v-if="editingTextId"
      ref="textEditorRef"
      v-model="editingTextDraft"
      class="cv-inline-text"
      :style="editingTextStyle"
      rows="2"
      @blur="commitTextEdit"
      @keydown.escape.prevent="commitTextEdit"
      @keydown.ctrl.enter.prevent="commitTextEdit"
    />

    <!-- Empty Canvas Welcome Overlay -->
    <div v-if="!elements.length && !paper.artboard" class="cv-empty">
      <div class="cv-empty-card">
        <div class="cv-empty-title">自由画布 · 拼图与标注工作台</div>
        <div class="cv-empty-sub">支持无限画布缩放拖拽、多图自由拼贴、完整标注工具、马赛克与一键排版</div>
        <div class="cv-empty-btns">
          <button type="button" class="sn-primary-btn" :disabled="busy" @click="onPickImages">
            + 添加本地图片
          </button>
          <button type="button" class="sn-ghost-btn" :disabled="busy" @click="toggleHistoryDrawer">
            从历史截图导入
          </button>
        </div>
        <div class="cv-empty-hint">提示：可直接 Ctrl+V 粘贴剪贴板图片，或使用顶部工具栏直接绘制</div>
      </div>
    </div>

    <!-- Top Floating Toolbar (Excalidraw × SnowAir) -->
    <div class="cv-topbar">
      <!-- Left: Add / History / Arrange -->
      <div class="cv-pill">
        <button type="button" class="cv-pill-btn accent" :disabled="busy" title="添加本地图片" @click="onPickImages">
          + 图片
        </button>
        <button
          type="button"
          class="cv-pill-btn"
          :class="{ active: showHistoryDrawer }"
          title="从历史截图导入"
          @click="toggleHistoryDrawer"
        >
          历史
        </button>
        <div class="cv-menu-wrap">
          <button
            type="button"
            class="cv-pill-btn"
            :class="{ active: showArrangeMenu }"
            title="一键自动排版多图"
            @click="showArrangeMenu = !showArrangeMenu; showExportMenu = false"
          >
            排版 ▾
          </button>
          <div v-if="showArrangeMenu" class="cv-dropdown">
            <button type="button" @click="onArrange('row')">横向排布</button>
            <button type="button" @click="onArrange('col')">纵向长图排布</button>
            <button type="button" @click="onArrange('grid2')">2 列拼图网格</button>
            <button type="button" @click="onArrange('grid3')">3 列拼图网格</button>
            <div class="cv-drop-sep" />
            <button type="button" @click="clearCanvasAll">清空画布</button>
          </div>
        </div>
      </div>

      <!-- Center: Main Drawing & Selection Tools -->
      <div class="cv-pill tools">
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'select' }"
          title="选择 / 移动 / 缩放 (V)"
          @click="activeTool = 'select'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M5 3.5l10 7.2-4.6.9-2.2 4.4L5 3.5z" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'hand' }"
          title="抓手拖拽画布 (H / 按住空格)"
          @click="activeTool = 'hand'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6">
            <path d="M7.5 10V5.2a1 1 0 0 1 2 0V9m0-4.2a1 1 0 0 1 2 0V9m0-3.2a1 1 0 0 1 2 0V10m0-2a1 1 0 0 1 2 0v4.2a4.8 4.8 0 0 1-4.8 4.8H9.5a4.5 4.5 0 0 1-3.8-2.1L4.2 12a1 1 0 0 1 1.6-1.2L7.5 12" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <span class="cv-sep" />
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'rect' }"
          title="矩形 (R)"
          @click="activeTool = 'rect'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <rect x="3.5" y="4.5" width="13" height="11" rx="2" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'ellipse' }"
          title="椭圆 (O)"
          @click="activeTool = 'ellipse'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <ellipse cx="10" cy="10" rx="6.5" ry="5.5" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'arrow' }"
          title="箭头 (A)"
          @click="activeTool = 'arrow'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M4.5 15.5L15.5 4.5M9.5 4.5h6v6" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'line' }"
          title="直线 (L)"
          @click="activeTool = 'line'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M4 16L16 4" stroke-linecap="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'pen' }"
          title="画笔 (P)"
          @click="activeTool = 'pen'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M13.5 3.5l3 3-9.5 9.5H4v-3l9.5-9.5z" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'highlighter' }"
          title="荧光笔"
          @click="activeTool = 'highlighter'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M11.5 3.5l5 5-7.5 7.5H5v-4l6.5-8.5zM3.5 17h13" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'text' }"
          title="文字 (T)"
          @click="activeTool = 'text'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M4.5 5.5h11M10 5.5v10M7.5 15.5h5" stroke-linecap="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'number' }"
          title="序号标记 (N)"
          @click="activeTool = 'number'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <circle cx="10" cy="10" r="6.8" />
            <path d="M9.5 7.2l1-.7v6.8" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'mosaic' }"
          title="马赛克打码 (M)"
          @click="activeTool = 'mosaic'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="currentColor">
            <rect x="4" y="4" width="3.5" height="3.5" rx="0.6" />
            <rect x="11" y="4" width="3.5" height="3.5" rx="0.6" />
            <rect x="7.5" y="7.5" width="3.5" height="3.5" rx="0.6" />
            <rect x="14.5" y="7.5" width="2" height="3.5" rx="0.5" opacity="0.55" />
            <rect x="4" y="11" width="3.5" height="3.5" rx="0.6" />
            <rect x="11" y="11" width="3.5" height="3.5" rx="0.6" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: activeTool === 'eraser' }"
          title="橡皮擦 (E)"
          @click="activeTool = 'eraser'"
        >
          <svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M12.5 4.5l4 4-6.8 6.8H5.8L3.5 13l9-8.5zM9 15.3h7" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
      </div>

      <!-- Right: Undo / Redo / Copy / Export / Inspector Toggle -->
      <div class="cv-pill">
        <button
          type="button"
          class="cv-tool"
          :disabled="!canvasUndoStack.length"
          title="撤销 (Ctrl+Z)"
          @click="onUndo"
        >
          <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M6.5 7.5H13a3.5 3.5 0 1 1 0 7H9M6.5 7.5L9 5M6.5 7.5L9 10" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          class="cv-tool"
          :disabled="!canvasRedoStack.length"
          title="重做 (Ctrl+Y)"
          @click="onRedo"
        >
          <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.7">
            <path d="M13.5 7.5H7a3.5 3.5 0 1 0 0 7h4m2.5-7L11 5m2.5 2.5L11 10" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <span class="cv-sep" />
        <button type="button" class="cv-pill-btn" :disabled="busy" title="复制画布到剪贴板" @click="onCopyCanvas">
          复制
        </button>
        <div class="cv-menu-wrap">
          <button
            type="button"
            class="cv-pill-btn primary"
            :disabled="busy"
            @click="showExportMenu = !showExportMenu; showArrangeMenu = false"
          >
            {{ busy ? '处理中…' : '导出 ▾' }}
          </button>
          <div v-if="showExportMenu" class="cv-dropdown right">
            <button type="button" @click="onExportCanvas('png', 1)">导出画布 .PNG (1×)</button>
            <button type="button" @click="onExportCanvas('png', 2)">导出画布 .PNG (2× 高清)</button>
            <button type="button" @click="onExportCanvas('jpg', 1)">导出画布 .JPG</button>
            <button type="button" @click="onExportCanvas('webp', 1)">导出画布 .WebP</button>
            <button type="button" @click="onExportCanvas('pdf', 1)">导出画布 .PDF</button>
            <div class="cv-drop-sep" />
            <button type="button" :disabled="!imageCount" @click="onExportMultiDoc('pdf')">
              多图逐页合并 .PDF ({{ imageCount }}页)
            </button>
            <button type="button" :disabled="!imageCount" @click="onExportMultiDoc('pptx')">
              多图逐页合并 .PPTX ({{ imageCount }}页)
            </button>
            <template v-if="lastExportPath">
              <div class="cv-drop-sep" />
              <button type="button" @click="openHistoryFile(lastExportPath, true)">定位上次导出文件</button>
            </template>
          </div>
        </div>
        <button
          type="button"
          class="cv-tool"
          :class="{ active: showInspector }"
          title="显示/隐藏右侧属性面板"
          @click="showInspector = !showInspector"
        >
          <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.7">
            <rect x="3" y="4" width="14" height="12" rx="2" />
            <path d="M12.5 4v12" />
          </svg>
        </button>
      </div>
    </div>

    <!-- Collapsible History Image Drawer -->
    <div v-if="showHistoryDrawer" class="cv-hist-drawer">
      <div class="cv-hist-head">
        <span>点击历史截图添加到画布</span>
        <button type="button" class="cv-mini-x" @click="showHistoryDrawer = false">×</button>
      </div>
      <div class="cv-hist-list">
        <div v-if="!historyPickerItems.length" class="cv-hist-empty">暂无历史图片</div>
        <button
          v-for="item in historyPickerItems"
          :key="item.path"
          type="button"
          class="cv-hist-item"
          :title="fileName(item.path)"
          @click="addPaths([item.path])"
        >
          <img v-if="cachedHistoryThumbs[item.path]" :src="cachedHistoryThumbs[item.path]" />
          <span v-else class="cv-hist-ph">IMG</span>
          <span class="cv-hist-name">{{ fileName(item.path) }}</span>
        </button>
      </div>
    </div>

    <!-- Contextual Floating Right Inspector -->
    <aside v-if="showInspector" class="cv-inspector">
      <!-- Case 1: Image Element Selected -->
      <template v-if="primarySelected?.type === 'image'">
        <div class="insp-title">图片属性</div>
        <div class="insp-row">
          <span>圆角</span>
          <span class="insp-val">{{ primarySelected.radius }}px</span>
        </div>
        <input
          v-model.number="primarySelected.radius"
          type="range"
          min="0"
          max="48"
          class="sn-slider"
        />
        <div class="insp-row">
          <span>不透明度</span>
          <span class="insp-val">{{ Math.round(primarySelected.opacity * 100) }}%</span>
        </div>
        <input
          v-model.number="primarySelected.opacity"
          type="range"
          min="0.1"
          max="1"
          step="0.05"
          class="sn-slider"
        />
        <div class="insp-toggle">
          <span>投影阴影</span>
          <input v-model="primarySelected.shadow" type="checkbox" />
        </div>
        <div class="insp-btn-grid">
          <button type="button" class="insp-btn" @click="snapshotCanvasForUndo(); primarySelected.flipX = !primarySelected.flipX">
            水平翻转
          </button>
          <button type="button" class="insp-btn" @click="snapshotCanvasForUndo(); primarySelected.flipY = !primarySelected.flipY">
            垂直翻转
          </button>
          <button type="button" class="insp-btn" @click="snapshotCanvasForUndo(); primarySelected.rotation = 0">
            旋转归零
          </button>
          <button type="button" class="insp-btn" @click="duplicateSelected">
            复制一份
          </button>
          <button type="button" class="insp-btn" @click="moveZOrder('front')">
            置于顶层
          </button>
          <button type="button" class="insp-btn" @click="moveZOrder('back')">
            置于底层
          </button>
        </div>
        <button type="button" class="insp-btn danger full" @click="deleteSelected">
          删除选中图片 (Del)
        </button>
      </template>

      <!-- Case 2: Annotation / Shape / Text / Mosaic Selected or Drawing Tool Active -->
      <template v-else-if="primarySelected || (activeTool !== 'select' && activeTool !== 'hand' && activeTool !== 'eraser')">
        <div class="insp-title">
          {{ primarySelected ? '选中元素样式' : '绘制工具样式' }}
        </div>

        <template v-if="!primarySelected || primarySelected.type !== 'mosaic'">
          <div class="insp-lbl">主色 / 描边</div>
          <div class="insp-swatches">
            <button
              v-for="c in STROKE_PALETTE"
              :key="c"
              type="button"
              class="insp-dot"
              :class="{
                active:
                  primarySelected
                    ? ('stroke' in primarySelected ? primarySelected.stroke : 'color' in primarySelected ? primarySelected.color : '') === c
                    : drawStroke === c,
              }"
              :style="{ background: c }"
              @click="
                drawStroke = c;
                if (primarySelected) {
                  snapshotCanvasForUndo();
                  if ('stroke' in primarySelected) primarySelected.stroke = c;
                  else if ('color' in primarySelected) primarySelected.color = c;
                }
              "
            />
          </div>
        </template>

        <template v-if="(!primarySelected && (activeTool === 'rect' || activeTool === 'ellipse')) || primarySelected?.type === 'rect' || primarySelected?.type === 'ellipse'">
          <div class="insp-lbl">填充色</div>
          <div class="insp-swatches">
            <button
              v-for="c in FILL_PALETTE"
              :key="c"
              type="button"
              class="insp-dot"
              :class="{
                trans: c === 'transparent',
                active: primarySelected && 'fill' in primarySelected ? primarySelected.fill === c : drawFill === c,
              }"
              :style="c !== 'transparent' ? { background: c } : {}"
              @click="
                drawFill = c;
                if (primarySelected && 'fill' in primarySelected) {
                  snapshotCanvasForUndo();
                  primarySelected.fill = c;
                }
              "
            />
          </div>
        </template>

        <template v-if="primarySelected?.type === 'text' || (!primarySelected && activeTool === 'text')">
          <div class="insp-row">
            <span>字号</span>
            <span class="insp-val">{{ primarySelected?.type === 'text' ? primarySelected.fontSize : drawFontSize }}px</span>
          </div>
          <input
            :value="primarySelected?.type === 'text' ? primarySelected.fontSize : drawFontSize"
            type="range"
            min="14"
            max="72"
            class="sn-slider"
            @input="
              const v = Number(($event.target as HTMLInputElement).value);
              drawFontSize = v;
              if (primarySelected?.type === 'text') {
                primarySelected.fontSize = v;
                const d = measureTextElementSize(primarySelected.text, v, primarySelected.bold);
                primarySelected.w = d.w;
                primarySelected.h = d.h;
              }
            "
          />
          <button
            v-if="primarySelected?.type === 'text'"
            type="button"
            class="insp-btn full"
            @click="beginTextEdit(primarySelected)"
          >
            编辑文字内容
          </button>
        </template>

        <template v-else-if="primarySelected?.type === 'mosaic' || (!primarySelected && activeTool === 'mosaic')">
          <div class="insp-row">
            <span>像素块大小</span>
            <span class="insp-val">{{ primarySelected?.type === 'mosaic' ? primarySelected.pixelSize : drawMosaicSize }}px</span>
          </div>
          <input
            :value="primarySelected?.type === 'mosaic' ? primarySelected.pixelSize : drawMosaicSize"
            type="range"
            min="6"
            max="36"
            class="sn-slider"
            @input="
              const v = Number(($event.target as HTMLInputElement).value);
              drawMosaicSize = v;
              if (primarySelected?.type === 'mosaic') primarySelected.pixelSize = v;
            "
          />
        </template>

        <template v-else-if="!primarySelected || 'strokeWidth' in primarySelected">
          <div class="insp-row">
            <span>线宽</span>
            <span class="insp-val">{{ primarySelected && 'strokeWidth' in primarySelected ? primarySelected.strokeWidth : drawWidth }}px</span>
          </div>
          <input
            :value="primarySelected && 'strokeWidth' in primarySelected ? primarySelected.strokeWidth : drawWidth"
            type="range"
            min="1"
            max="24"
            class="sn-slider"
            @input="
              const v = Number(($event.target as HTMLInputElement).value);
              drawWidth = v;
              if (primarySelected && 'strokeWidth' in primarySelected) primarySelected.strokeWidth = v;
            "
          />
          <div v-if="!primarySelected || 'dashed' in primarySelected" class="insp-toggle">
            <span>虚线描边</span>
            <input
              :checked="primarySelected && 'dashed' in primarySelected ? primarySelected.dashed : drawDashed"
              type="checkbox"
              @change="
                const v = ($event.target as HTMLInputElement).checked;
                drawDashed = v;
                if (primarySelected && 'dashed' in primarySelected) primarySelected.dashed = v;
              "
            />
          </div>
        </template>

        <template v-if="primarySelected">
          <div class="insp-btn-grid">
            <button type="button" class="insp-btn" @click="moveZOrder('front')">置顶</button>
            <button type="button" class="insp-btn" @click="moveZOrder('back')">置底</button>
            <button type="button" class="insp-btn" @click="duplicateSelected">复制</button>
            <button type="button" class="insp-btn danger" @click="deleteSelected">删除</button>
          </div>
        </template>
      </template>

      <!-- Case 3: Multi-select Actions -->
      <template v-else-if="selectedElements.length > 1">
        <div class="insp-title">已选中 {{ selectedElements.length }} 个对象</div>
        <div class="insp-btn-grid">
          <button type="button" class="insp-btn" @click="onArrange('row')">横向对齐</button>
          <button type="button" class="insp-btn" @click="onArrange('col')">纵向对齐</button>
          <button type="button" class="insp-btn" @click="onArrange('grid2')">2列排版</button>
          <button type="button" class="insp-btn" @click="duplicateSelected">批量复制</button>
          <button type="button" class="insp-btn" @click="moveZOrder('front')">置于顶层</button>
          <button type="button" class="insp-btn" @click="moveZOrder('back')">置于底层</button>
        </div>
        <button type="button" class="insp-btn danger full" @click="deleteSelected">
          删除选中对象 (Del)
        </button>
      </template>

      <!-- Case 4: Canvas / Artboard Settings (nothing selected) -->
      <template v-else>
        <div class="insp-title">画布与画板</div>
        <div class="insp-seg">
          <button
            type="button"
            :class="{ active: !paper.artboard }"
            @click="paper.artboard = false"
          >
            无限画布
          </button>
          <button
            type="button"
            :class="{ active: paper.artboard }"
            @click="paper.artboard = true; nextTick(fitView)"
          >
            固定画板
          </button>
        </div>

        <template v-if="paper.artboard">
          <div class="insp-lbl">画板比例预设</div>
          <div class="insp-btn-grid">
            <button type="button" class="insp-btn" @click="applyPaperRatio(1280, 720)">16:9 横屏</button>
            <button type="button" class="insp-btn" @click="applyPaperRatio(1200, 900)">4:3 演示</button>
            <button type="button" class="insp-btn" @click="applyPaperRatio(1080, 1080)">1:1 方形</button>
            <button type="button" class="insp-btn" @click="applyPaperRatio(720, 1280)">9:16 竖屏</button>
          </div>
          <div class="insp-dims">
            <label>宽 <input v-model.number="paper.width" type="number" min="200" max="4096" /></label>
            <label>高 <input v-model.number="paper.height" type="number" min="200" max="4096" /></label>
          </div>
        </template>

        <div class="insp-lbl">导出背景色 / 渐变</div>
        <div class="insp-bg-grid">
          <button
            v-for="bg in PAPER_BACKGROUNDS"
            :key="bg.id"
            type="button"
            class="insp-bg-chip"
            :class="{ active: paper.bg === bg.value, trans: bg.value === 'transparent' }"
            :style="bg.value !== 'transparent' ? { background: bg.value } : {}"
            :title="bg.label"
            @click="paper.bg = bg.value"
          />
        </div>

        <div class="insp-toggle">
          <span>显示点阵网格</span>
          <input v-model="paper.showGrid" type="checkbox" />
        </div>
      </template>
    </aside>

    <!-- Bottom-Left Floating Zoom Capsule -->
    <div class="cv-zoom-bar">
      <button type="button" title="缩小" @click="zoomBy(1 / 1.2)">−</button>
      <button type="button" class="cv-zoom-pct" title="重置为 100%" @click="resetZoom">
        {{ Math.round(vp.zoom * 100) }}%
      </button>
      <button type="button" title="放大" @click="zoomBy(1.2)">+</button>
      <span class="cv-sep sm" />
      <button type="button" class="cv-zoom-fit" title="适应视图 (Ctrl+0)" @click="fitView">适应</button>
    </div>

    <!-- Bottom-Right Status Hint -->
    <div class="cv-status-pill">
      {{ elements.length }} 个对象 · 滚轮缩放 · 空格拖拽 · 右键菜单 · Ctrl+V 贴图
    </div>

    <ContextMenu
      :visible="ctxMenu.visible"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :items="canvasCtxItems"
      @select="onCanvasCtxSelect"
      @close="ctxMenu.visible = false"
    />
  </div>
</template>

<style scoped>
.cv-root {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 520px;
  overflow: hidden;
  user-select: none;
}

.cv-canvas {
  display: block;
  width: 100%;
  height: 100%;
  touch-action: none;
}

.cv-inline-text {
  position: absolute;
  z-index: 30;
  padding: 5px 8px;
  border: 1.5px solid var(--sn-accent);
  border-radius: 6px;
  outline: none;
  resize: both;
  font-family: 'Microsoft YaHei UI', 'Segoe UI', sans-serif;
  line-height: 1.32;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
}

.cv-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  z-index: 5;
}

.cv-empty-card {
  pointer-events: auto;
  text-align: center;
  padding: 26px 32px;
  border-radius: 16px;
  border: 1px dashed var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 90%, transparent);
  backdrop-filter: blur(12px);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.16);
  max-width: 440px;
}

.cv-empty-title {
  font-size: 16px;
  font-weight: 700;
  color: var(--sn-text);
  margin-bottom: 6px;
}

.cv-empty-sub {
  font-size: 12.5px;
  color: var(--sn-muted);
  margin-bottom: 16px;
  line-height: 1.5;
}

.cv-empty-btns {
  display: flex;
  justify-content: center;
  gap: 10px;
  margin-bottom: 12px;
}

.cv-empty-hint {
  font-size: 11px;
  color: var(--sn-muted);
  opacity: 0.8;
}

/* Top Floating Excalidraw-style Toolbar */
.cv-topbar {
  position: absolute;
  top: 12px;
  left: 12px;
  right: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  pointer-events: none;
  z-index: 20;
  flex-wrap: wrap;
}

.cv-pill {
  pointer-events: auto;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 4px 6px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 94%, transparent);
  backdrop-filter: blur(12px);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.2);
}

.cv-tool {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--sn-text);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.12s ease;
}

.cv-tool:hover:not(:disabled) {
  background: var(--sn-hover);
}

.cv-tool.active {
  background: color-mix(in srgb, var(--sn-accent) 20%, transparent);
  color: var(--sn-accent);
}

.cv-tool:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.cv-pill-btn {
  height: 30px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--sn-text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.12s ease;
  white-space: nowrap;
}

.cv-pill-btn:hover:not(:disabled) {
  background: var(--sn-hover);
}

.cv-pill-btn.active {
  background: color-mix(in srgb, var(--sn-accent) 20%, transparent);
  color: var(--sn-accent);
}

.cv-pill-btn.accent {
  background: color-mix(in srgb, var(--sn-accent) 16%, transparent);
  color: var(--sn-accent);
}

.cv-pill-btn.accent:hover {
  background: color-mix(in srgb, var(--sn-accent) 26%, transparent);
}

.cv-pill-btn.primary {
  background: var(--sn-accent);
  color: #fff;
}

.cv-pill-btn.primary:hover:not(:disabled) {
  background: var(--sn-accent-hover);
}

.cv-sep {
  width: 1px;
  height: 18px;
  background: var(--sn-border);
  margin: 0 3px;
}

.cv-sep.sm {
  height: 14px;
}

.cv-menu-wrap {
  position: relative;
}

.cv-dropdown {
  position: absolute;
  top: 36px;
  left: 0;
  min-width: 170px;
  padding: 5px;
  border-radius: 10px;
  border: 1px solid var(--sn-border);
  background: var(--sn-card);
  box-shadow: 0 12px 30px rgba(0, 0, 0, 0.3);
  display: flex;
  flex-direction: column;
  gap: 2px;
  z-index: 40;
}

.cv-dropdown.right {
  left: auto;
  right: 0;
}

.cv-dropdown button {
  border: none;
  border-radius: 7px;
  padding: 7px 10px;
  background: transparent;
  color: var(--sn-text);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  white-space: nowrap;
}

.cv-dropdown button:hover:not(:disabled) {
  background: var(--sn-hover);
  color: var(--sn-accent);
}

.cv-dropdown button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.cv-drop-sep {
  height: 1px;
  background: var(--sn-border);
  margin: 3px 4px;
}

/* History Picker Drawer */
.cv-hist-drawer {
  position: absolute;
  top: 58px;
  left: 12px;
  right: 248px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 96%, transparent);
  backdrop-filter: blur(12px);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.25);
  z-index: 18;
}

.cv-hist-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--sn-muted);
  margin-bottom: 8px;
}

.cv-mini-x {
  border: none;
  background: transparent;
  color: var(--sn-muted);
  font-size: 16px;
  cursor: pointer;
}

.cv-hist-list {
  display: flex;
  gap: 8px;
  overflow-x: auto;
  padding-bottom: 2px;
}

.cv-hist-empty {
  font-size: 12px;
  color: var(--sn-muted);
  padding: 12px 4px;
}

.cv-hist-item {
  flex: 0 0 88px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 4px;
  border-radius: 8px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  cursor: pointer;
}

.cv-hist-item:hover {
  border-color: var(--sn-accent);
}

.cv-hist-item img,
.cv-hist-ph {
  width: 100%;
  height: 52px;
  object-fit: cover;
  border-radius: 5px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  color: var(--sn-muted);
}

.cv-hist-name {
  font-size: 10px;
  color: var(--sn-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Right Contextual Floating Inspector */
.cv-inspector {
  position: absolute;
  top: 58px;
  right: 12px;
  width: 220px;
  max-height: calc(100% - 108px);
  overflow-y: auto;
  padding: 12px;
  border-radius: 12px;
  border: 1px solid var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 94%, transparent);
  backdrop-filter: blur(12px);
  box-shadow: 0 8px 26px rgba(0, 0, 0, 0.22);
  display: flex;
  flex-direction: column;
  gap: 10px;
  z-index: 16;
}

.insp-title {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--sn-text);
  padding-bottom: 6px;
  border-bottom: 1px solid var(--sn-border);
}

.insp-lbl {
  font-size: 11px;
  font-weight: 600;
  color: var(--sn-muted);
}

.insp-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11.5px;
  color: var(--sn-muted);
}

.insp-val {
  color: var(--sn-text);
  font-weight: 600;
}

.insp-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--sn-text);
}

.insp-seg {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4px;
  padding: 3px;
  border-radius: 8px;
  background: var(--sn-subtle);
  border: 1px solid var(--sn-border);
}

.insp-seg button {
  border: none;
  border-radius: 6px;
  padding: 5px 6px;
  background: transparent;
  color: var(--sn-muted);
  font-size: 11.5px;
  font-weight: 600;
  cursor: pointer;
}

.insp-seg button.active {
  background: var(--sn-card);
  color: var(--sn-accent);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.16);
}

.insp-swatches {
  display: grid;
  grid-template-columns: repeat(8, 1fr);
  gap: 5px;
}

.insp-dot {
  width: 20px;
  height: 20px;
  border-radius: 999px;
  border: 2px solid transparent;
  box-shadow: inset 0 0 0 1px rgba(128, 128, 128, 0.35);
  cursor: pointer;
}

.insp-dot.active {
  border-color: var(--sn-accent);
  transform: scale(1.1);
}

.insp-dot.trans,
.insp-bg-chip.trans {
  background: linear-gradient(45deg, #bbb 25%, transparent 25%, transparent 75%, #bbb 75%),
    linear-gradient(45deg, #bbb 25%, #fff 25%, #fff 75%, #bbb 75%);
  background-size: 8px 8px;
  background-position: 0 0, 4px 4px;
}

.insp-bg-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}

.insp-bg-chip {
  height: 26px;
  border-radius: 7px;
  border: 2px solid transparent;
  box-shadow: inset 0 0 0 1px rgba(128, 128, 128, 0.3);
  cursor: pointer;
}

.insp-bg-chip.active {
  border-color: var(--sn-accent);
}

.insp-dims {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
}

.insp-dims label {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--sn-muted);
}

.insp-dims input {
  width: 100%;
  padding: 4px 6px;
  border-radius: 6px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-text);
  font-size: 11.5px;
}

.insp-btn-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 5px;
}

.insp-btn {
  padding: 6px 8px;
  border-radius: 7px;
  border: 1px solid var(--sn-border);
  background: var(--sn-subtle);
  color: var(--sn-text);
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
}

.insp-btn:hover {
  border-color: var(--sn-accent);
  color: var(--sn-accent);
}

.insp-btn.danger:hover {
  border-color: #ef4444;
  color: #ef4444;
}

.insp-btn.full {
  width: 100%;
}

/* Bottom Floating Zoom & Status */
.cv-zoom-bar {
  position: absolute;
  bottom: 12px;
  left: 12px;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 3px 6px;
  border-radius: 10px;
  border: 1px solid var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 92%, transparent);
  backdrop-filter: blur(10px);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
  z-index: 16;
}

.cv-zoom-bar button {
  border: none;
  background: transparent;
  color: var(--sn-text);
  font-size: 12px;
  font-weight: 600;
  min-width: 26px;
  height: 26px;
  border-radius: 6px;
  cursor: pointer;
}

.cv-zoom-bar button:hover {
  background: var(--sn-hover);
}

.cv-zoom-pct {
  min-width: 44px !important;
  font-size: 11.5px !important;
}

.cv-zoom-fit {
  padding: 0 8px;
  font-size: 11.5px !important;
  color: var(--sn-accent) !important;
}

.cv-status-pill {
  position: absolute;
  bottom: 14px;
  right: 12px;
  padding: 5px 10px;
  border-radius: 999px;
  border: 1px solid var(--sn-border);
  background: color-mix(in srgb, var(--sn-card) 88%, transparent);
  color: var(--sn-muted);
  font-size: 11px;
  pointer-events: none;
  z-index: 15;
}
</style>
