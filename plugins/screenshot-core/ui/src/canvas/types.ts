export type CanvasTool =
  | 'select'
  | 'hand'
  | 'rect'
  | 'ellipse'
  | 'arrow'
  | 'line'
  | 'pen'
  | 'highlighter'
  | 'text'
  | 'number'
  | 'mosaic'
  | 'eraser'

export interface Point {
  x: number
  y: number
}

export interface Bounds {
  x: number
  y: number
  w: number
  h: number
}

export type HandleId = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'rotate'

interface BaseElement {
  id: string
  x: number
  y: number
  w: number
  h: number
  rotation: number // radians
  opacity: number // 0..1
  locked?: boolean
}

export interface ImageElement extends BaseElement {
  type: 'image'
  src: string
  path: string
  name: string
  naturalW: number
  naturalH: number
  radius: number
  shadow: boolean
  flipX: boolean
  flipY: boolean
}

export interface RectElement extends BaseElement {
  type: 'rect'
  stroke: string
  fill: string
  strokeWidth: number
  radius: number
  dashed: boolean
}

export interface EllipseElement extends BaseElement {
  type: 'ellipse'
  stroke: string
  fill: string
  strokeWidth: number
  dashed: boolean
}

export interface ArrowElement extends BaseElement {
  type: 'arrow' | 'line'
  x2: number
  y2: number
  stroke: string
  strokeWidth: number
  dashed: boolean
  doubleHead?: boolean
}

export interface PenElement extends BaseElement {
  type: 'pen'
  points: Point[]
  stroke: string
  strokeWidth: number
  highlighter: boolean
}

export interface TextElement extends BaseElement {
  type: 'text'
  text: string
  fontSize: number
  color: string
  bgColor: string
  bold: boolean
}

export interface NumberElement extends BaseElement {
  type: 'number'
  value: number
  color: string
}

export interface MosaicElement extends BaseElement {
  type: 'mosaic'
  pixelSize: number
}

export type CanvasElement =
  | ImageElement
  | RectElement
  | EllipseElement
  | ArrowElement
  | PenElement
  | TextElement
  | NumberElement
  | MosaicElement

export interface CanvasViewport {
  panX: number
  panY: number
  zoom: number
}

export interface CanvasPaper {
  /** If true, renders & exports a fixed artboard; if false, infinite whiteboard exports tight content bounds */
  artboard: boolean
  width: number
  height: number
  bg: string
  showGrid: boolean
}

export const STROKE_PALETTE = [
  '#ef4444',
  '#f59e0b',
  '#34c759',
  '#3b82f6',
  '#a855f7',
  '#ec4899',
  '#ffffff',
  '#18181b',
]

export const FILL_PALETTE = [
  'transparent',
  'rgba(239, 68, 68, 0.16)',
  'rgba(245, 158, 11, 0.18)',
  'rgba(52, 199, 89, 0.16)',
  'rgba(59, 130, 246, 0.16)',
  'rgba(168, 85, 247, 0.16)',
  '#ffffff',
  '#18181b',
]

export const PAPER_BACKGROUNDS: { id: string; label: string; value: string }[] = [
  { id: 'white', label: '纯白', value: '#ffffff' },
  { id: 'dark', label: '深黑', value: '#18181b' },
  { id: 'slate', label: '浅灰', value: '#f4f4f5' },
  { id: 'trans', label: '透明', value: 'transparent' },
  {
    id: 'grad_mint',
    label: '雪绿渐变',
    value: 'linear-gradient(135deg, #d1fae5 0%, #a7f3d0 45%, #bfdbfe 100%)',
  },
  {
    id: 'grad_aurora',
    label: '极光紫蓝',
    value: 'linear-gradient(135deg, #6366f1 0%, #a855f7 50%, #ec4899 100%)',
  },
  {
    id: 'grad_sunset',
    label: '晚霞橙粉',
    value: 'linear-gradient(135deg, #f97316 0%, #fb7185 52%, #c084fc 100%)',
  },
  {
    id: 'grad_obsidian',
    label: '暗夜蓝灰',
    value: 'linear-gradient(135deg, #0f172a 0%, #1e293b 55%, #334155 100%)',
  },
]

export function newId(prefix = 'el'): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`
}

export function getElementBounds(el: CanvasElement): Bounds {
  if (el.type === 'arrow' || el.type === 'line') {
    const minX = Math.min(el.x, el.x2)
    const minY = Math.min(el.y, el.y2)
    const maxX = Math.max(el.x, el.x2)
    const maxY = Math.max(el.y, el.y2)
    return {
      x: minX,
      y: minY,
      w: Math.max(8, maxX - minX),
      h: Math.max(8, maxY - minY),
    }
  }
  if (el.type === 'pen') {
    if (!el.points.length) return { x: el.x, y: el.y, w: 1, h: 1 }
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const p of el.points) {
      if (p.x < minX) minX = p.x
      if (p.y < minY) minY = p.y
      if (p.x > maxX) maxX = p.x
      if (p.y > maxY) maxY = p.y
    }
    return {
      x: minX,
      y: minY,
      w: Math.max(6, maxX - minX),
      h: Math.max(6, maxY - minY),
    }
  }
  return {
    x: el.w >= 0 ? el.x : el.x + el.w,
    y: el.h >= 0 ? el.y : el.y + el.h,
    w: Math.abs(el.w),
    h: Math.abs(el.h),
  }
}

export function getMultiBounds(elements: CanvasElement[]): Bounds | null {
  if (!elements.length) return null
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const el of elements) {
    const b = getElementBounds(el)
    if (b.x < minX) minX = b.x
    if (b.y < minY) minY = b.y
    if (b.x + b.w > maxX) maxX = b.x + b.w
    if (b.y + b.h > maxY) maxY = b.y + b.h
  }
  return { x: minX, y: minY, w: Math.max(1, maxX - minX), h: Math.max(1, maxY - minY) }
}

export function rotatePoint(pt: Point, cx: number, cy: number, rad: number): Point {
  if (!rad) return pt
  const cos = Math.cos(rad)
  const sin = Math.sin(rad)
  const dx = pt.x - cx
  const dy = pt.y - cy
  return { x: cx + dx * cos - dy * sin, y: cy + dx * sin + dy * cos }
}

function pointToSegmentDist(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x
  const dy = b.y - a.y
  const lenSq = dx * dx + dy * dy
  if (lenSq < 1e-6) return Math.hypot(p.x - a.x, p.y - a.y)
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / lenSq))
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
}

export function hitTestElement(el: CanvasElement, worldPt: Point, zoom: number): boolean {
  const tol = Math.max(8 / zoom, 6)
  if (el.type === 'arrow' || el.type === 'line') {
    return pointToSegmentDist(worldPt, { x: el.x, y: el.y }, { x: el.x2, y: el.y2 }) <= tol + el.strokeWidth * 0.6
  }
  if (el.type === 'pen') {
    const hitR = tol + el.strokeWidth * 0.65
    for (let i = 0; i < el.points.length - 1; i++) {
      if (pointToSegmentDist(worldPt, el.points[i], el.points[i + 1]) <= hitR) return true
    }
    if (el.points.length === 1) {
      return Math.hypot(worldPt.x - el.points[0].x, worldPt.y - el.points[0].y) <= hitR
    }
    return false
  }
  const b = getElementBounds(el)
  const cx = b.x + b.w * 0.5
  const cy = b.y + b.h * 0.5
  const local = rotatePoint(worldPt, cx, cy, -el.rotation)
  if (el.type === 'number') {
    const r = Math.max(b.w, b.h) * 0.5 + tol * 0.5
    return Math.hypot(local.x - cx, local.y - cy) <= r
  }
  if (el.type === 'ellipse') {
    const rx = Math.max(1, b.w * 0.5)
    const ry = Math.max(1, b.h * 0.5)
    const nx = (local.x - cx) / (rx + tol * 0.5)
    const ny = (local.y - cy) / (ry + tol * 0.5)
    const d = nx * nx + ny * ny
    if (el.fill && el.fill !== 'transparent') return d <= 1.05
    const irx = Math.max(1, rx - tol - el.strokeWidth)
    const iry = Math.max(1, ry - tol - el.strokeWidth)
    const inx = (local.x - cx) / irx
    const iny = (local.y - cy) / iry
    return d <= 1.05 && inx * inx + iny * iny >= 0.85
  }
  if (el.type === 'rect' && (!el.fill || el.fill === 'transparent')) {
    const insideOuter =
      local.x >= b.x - tol &&
      local.x <= b.x + b.w + tol &&
      local.y >= b.y - tol &&
      local.y <= b.y + b.h + tol
    const insideInner =
      local.x >= b.x + tol + el.strokeWidth &&
      local.x <= b.x + b.w - tol - el.strokeWidth &&
      local.y >= b.y + tol + el.strokeWidth &&
      local.y <= b.y + b.h - tol - el.strokeWidth
    return insideOuter && !insideInner
  }
  return (
    local.x >= b.x - tol * 0.4 &&
    local.x <= b.x + b.w + tol * 0.4 &&
    local.y >= b.y - tol * 0.4 &&
    local.y <= b.y + b.h + tol * 0.4
  )
}

export function getHandlePositions(el: CanvasElement, zoom: number): Record<HandleId, Point> {
  const b = getElementBounds(el)
  const cx = b.x + b.w * 0.5
  const cy = b.y + b.h * 0.5
  const rotStem = 26 / zoom
  const raw: Record<HandleId, Point> = {
    nw: { x: b.x, y: b.y },
    n: { x: cx, y: b.y },
    ne: { x: b.x + b.w, y: b.y },
    e: { x: b.x + b.w, y: cy },
    se: { x: b.x + b.w, y: b.y + b.h },
    s: { x: cx, y: b.y + b.h },
    sw: { x: b.x, y: b.y + b.h },
    w: { x: b.x, y: cy },
    rotate: { x: cx, y: b.y - rotStem },
  }
  if (!el.rotation) return raw
  const out = {} as Record<HandleId, Point>
  for (const k of Object.keys(raw) as HandleId[]) {
    out[k] = rotatePoint(raw[k], cx, cy, el.rotation)
  }
  return out
}

export function hitTestHandles(
  el: CanvasElement,
  worldPt: Point,
  zoom: number,
): HandleId | null {
  if (el.type === 'arrow' || el.type === 'line' || el.type === 'pen' || el.type === 'number') {
    return null
  }
  const handles = getHandlePositions(el, zoom)
  const r = 10 / zoom
  for (const k of ['rotate', 'nw', 'ne', 'se', 'sw', 'n', 'e', 's', 'w'] as HandleId[]) {
    const p = handles[k]
    if (Math.hypot(worldPt.x - p.x, worldPt.y - p.y) <= r) {
      return k
    }
  }
  return null
}

export function cloneElement(el: CanvasElement): CanvasElement {
  if (el.type === 'pen') {
    return { ...el, points: el.points.map((p) => ({ ...p })) }
  }
  return { ...el }
}
