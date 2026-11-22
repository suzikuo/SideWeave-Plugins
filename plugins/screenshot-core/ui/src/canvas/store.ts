import { reactive, ref } from 'vue'
import {
  cloneElement,
  newId,
  type CanvasElement,
  type CanvasPaper,
  type CanvasViewport,
  type ImageElement,
} from './types'

const MAX_HISTORY = 60

/**
 * Persistent module-level Canvas store so switching between History / Settings / Canvas
 * keeps the entire whiteboard scene, zoom, pan, images, and undo history intact.
 */
export const canvasElements = ref<CanvasElement[]>([])
export const canvasImageCache = new Map<string, HTMLImageElement>()
export const canvasViewport = reactive<CanvasViewport>({
  panX: 60,
  panY: 60,
  zoom: 0.72,
})
export const canvasPaper = reactive<CanvasPaper>({
  artboard: false,
  width: 1280,
  height: 720,
  bg: '#ffffff',
  showGrid: true,
})
export const canvasUndoStack = ref<CanvasElement[][]>([])
export const canvasRedoStack = ref<CanvasElement[][]>([])
export const canvasNumberSeq = ref(1)

export function snapshotCanvasForUndo() {
  canvasUndoStack.value.push(canvasElements.value.map(cloneElement))
  if (canvasUndoStack.value.length > MAX_HISTORY) {
    canvasUndoStack.value.shift()
  }
  canvasRedoStack.value = []
}

export function undoCanvas(): boolean {
  const prev = canvasUndoStack.value.pop()
  if (!prev) return false
  canvasRedoStack.value.push(canvasElements.value.map(cloneElement))
  canvasElements.value = prev
  return true
}

export function redoCanvas(): boolean {
  const next = canvasRedoStack.value.pop()
  if (!next) return false
  canvasUndoStack.value.push(canvasElements.value.map(cloneElement))
  canvasElements.value = next
  return true
}

export function loadHtmlImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve) => {
    const img = new Image()
    img.onload = () => resolve(img)
    img.onerror = () => resolve(img)
    img.src = src
  })
}

export async function addImageElementToCanvas(params: {
  src: string
  path: string
  name: string
  width?: number
  height?: number
  centerWorld?: { x: number; y: number }
  offsetIndex?: number
}): Promise<ImageElement> {
  const img = await loadHtmlImage(params.src)
  const natW = params.width || img.naturalWidth || 800
  const natH = params.height || img.naturalHeight || 600
  // Fit initial placement to a pleasant working size (max 560px wide/tall)
  const scale = Math.min(1, 560 / Math.max(natW, natH))
  const w = Math.round(natW * scale)
  const h = Math.round(natH * scale)
  const cascade = (params.offsetIndex ?? 0) * 32
  const cx = params.centerWorld?.x ?? 420
  const cy = params.centerWorld?.y ?? 280
  const el: ImageElement = {
    id: newId('img'),
    type: 'image',
    x: Math.round(cx - w * 0.5 + cascade),
    y: Math.round(cy - h * 0.5 + cascade),
    w,
    h,
    rotation: 0,
    opacity: 1,
    src: params.src,
    path: params.path,
    name: params.name,
    naturalW: natW,
    naturalH: natH,
    radius: 10,
    shadow: true,
    flipX: false,
    flipY: false,
  }
  canvasImageCache.set(el.id, img)
  canvasImageCache.set(el.src, img)
  canvasElements.value.push(el)
  return el
}

/**
 * Smart one-click collage arrangement for all images (or selected images) on the canvas!
 */
export function autoArrangeImages(
  layout: 'row' | 'col' | 'grid2' | 'grid3',
  selectedIds: Set<string>,
): boolean {
  const selectedImages = canvasElements.value.filter(
    (e): e is ImageElement => e.type === 'image' && selectedIds.has(e.id),
  )
  const targets =
    selectedImages.length >= 2
      ? selectedImages
      : canvasElements.value.filter((e): e is ImageElement => e.type === 'image')
  if (!targets.length) return false

  snapshotCanvasForUndo()
  const gap = 24
  const pad = 32
  const cols =
    layout === 'row'
      ? targets.length
      : layout === 'col'
        ? 1
        : layout === 'grid2'
          ? 2
          : 3

  // Normalize image sizes neatly per row/column
  const targetCellW = layout === 'col' ? 640 : layout === 'grid3' ? 380 : 480
  if (layout === 'row') {
    const targetH = 380
    let x = pad
    for (const img of targets) {
      const ratio = img.naturalW / Math.max(1, img.naturalH)
      img.w = Math.round(targetH * ratio)
      img.h = targetH
      img.x = x
      img.y = pad
      img.rotation = 0
      x += img.w + gap
    }
    if (canvasPaper.artboard) {
      canvasPaper.width = x - gap + pad
      canvasPaper.height = targetH + pad * 2
    }
    return true
  }

  let curY = pad
  let maxRowW = 0
  for (let i = 0; i < targets.length; i += cols) {
    const rowItems = targets.slice(i, i + cols)
    let curX = pad
    let rowMaxH = 0
    for (const img of rowItems) {
      const ratio = img.naturalW / Math.max(1, img.naturalH)
      img.w = targetCellW
      img.h = Math.round(targetCellW / Math.max(0.1, ratio))
      img.x = curX
      img.y = curY
      img.rotation = 0
      curX += img.w + gap
      if (img.h > rowMaxH) rowMaxH = img.h
    }
    maxRowW = Math.max(maxRowW, curX - gap + pad)
    curY += rowMaxH + gap
  }
  if (canvasPaper.artboard) {
    canvasPaper.width = Math.max(320, maxRowW)
    canvasPaper.height = Math.max(240, curY - gap + pad)
  }
  return true
}
