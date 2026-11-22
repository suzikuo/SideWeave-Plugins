import {
  getElementBounds,
  getHandlePositions,
  getMultiBounds,
  type Bounds,
  type CanvasElement,
  type CanvasPaper,
  type CanvasViewport,
  type HandleId,
} from './types'

export interface RenderOptions {
  elements: CanvasElement[]
  imageCache: Map<string, HTMLImageElement>
  viewport: CanvasViewport
  paper: CanvasPaper
  selectedIds: Set<string>
  editingTextId: string | null
  marquee: Bounds | null
  draftElement: CanvasElement | null
  isDarkTheme: boolean
  showLiveSizeBadge?: boolean
}

function fillBackgroundStyle(
  ctx: CanvasRenderingContext2D,
  bg: string,
  x: number,
  y: number,
  w: number,
  h: number,
) {
  if (bg === 'transparent') return
  if (bg.startsWith('linear-gradient')) {
    const colors = bg.match(/#[0-9a-fA-F]{3,8}|rgba?\([^)]+\)/g)
    if (colors && colors.length >= 2) {
      const grad = ctx.createLinearGradient(x, y, x + w, y + h)
      colors.forEach((c, idx) => {
        grad.addColorStop(idx / (colors.length - 1), c)
      })
      ctx.fillStyle = grad
      ctx.fillRect(x, y, w, h)
      return
    }
  }
  ctx.fillStyle = bg
  ctx.fillRect(x, y, w, h)
}

function drawCheckerboard(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  cell = 16,
) {
  ctx.save()
  ctx.beginPath()
  ctx.rect(x, y, w, h)
  ctx.clip()
  ctx.fillStyle = '#ffffff'
  ctx.fillRect(x, y, w, h)
  ctx.fillStyle = '#e5e7eb'
  const cols = Math.ceil(w / cell)
  const rows = Math.ceil(h / cell)
  for (let r = 0; r < rows; r++) {
    for (let c = (r % 2); c < cols; c += 2) {
      ctx.fillRect(x + c * cell, y + r * cell, cell, cell)
    }
  }
  ctx.restore()
}

function drawDotGrid(
  ctx: CanvasRenderingContext2D,
  viewW: number,
  viewH: number,
  vp: CanvasViewport,
  isDark: boolean,
) {
  // Adaptive grid step so dots stay clean across 10%..600% zoom
  let step = 24
  while (step * vp.zoom < 14) step *= 2
  while (step * vp.zoom > 56) step /= 2

  const worldLeft = -vp.panX / vp.zoom
  const worldTop = -vp.panY / vp.zoom
  const worldRight = (viewW - vp.panX) / vp.zoom
  const worldBottom = (viewH - vp.panY) / vp.zoom

  const startX = Math.floor(worldLeft / step) * step
  const startY = Math.floor(worldTop / step) * step

  ctx.save()
  ctx.fillStyle = isDark ? 'rgba(255, 255, 255, 0.13)' : 'rgba(24, 24, 27, 0.14)'
  const r = Math.max(1, Math.min(1.8, 1.15 * Math.sqrt(vp.zoom)))
  for (let wx = startX; wx <= worldRight; wx += step) {
    const sx = wx * vp.zoom + vp.panX
    for (let wy = startY; wy <= worldBottom; wy += step) {
      const sy = wy * vp.zoom + vp.panY
      ctx.fillRect(sx - r * 0.5, sy - r * 0.5, r, r)
    }
  }
  ctx.restore()
}

function drawArrowhead(
  ctx: CanvasRenderingContext2D,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  size: number,
) {
  const ang = Math.atan2(toY - fromY, toX - fromX)
  ctx.beginPath()
  ctx.moveTo(toX, toY)
  ctx.lineTo(toX - size * Math.cos(ang - 0.42), toY - size * Math.sin(ang - 0.42))
  ctx.lineTo(toX - size * 0.72 * Math.cos(ang), toY - size * 0.72 * Math.sin(ang))
  ctx.lineTo(toX - size * Math.cos(ang + 0.42), toY - size * Math.sin(ang + 0.42))
  ctx.closePath()
  ctx.fill()
}

function applyMosaicRegion(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  pixelSize: number,
) {
  const m = ctx.getTransform()
  // Map world rect corners through current canvas transform into backing device pixels
  const p0 = m.transformPoint(new DOMPoint(x, y))
  const p1 = m.transformPoint(new DOMPoint(x + w, y + h))
  const left = Math.max(0, Math.floor(Math.min(p0.x, p1.x)))
  const top = Math.max(0, Math.floor(Math.min(p0.y, p1.y)))
  const right = Math.min(ctx.canvas.width, Math.ceil(Math.max(p0.x, p1.x)))
  const bottom = Math.min(ctx.canvas.height, Math.ceil(Math.max(p0.y, p1.y)))
  const rw = right - left
  const rh = bottom - top
  if (rw < 2 || rh < 2) return

  const scaleX = Math.hypot(m.a, m.b)
  const block = Math.max(4, Math.round(pixelSize * Math.max(0.5, scaleX)))
  const imgData = ctx.getImageData(left, top, rw, rh)
  const data = imgData.data
  for (let by = 0; by < rh; by += block) {
    const bh = Math.min(block, rh - by)
    for (let bx = 0; bx < rw; bx += block) {
      const bw = Math.min(block, rw - bx)
      let r = 0
      let g = 0
      let b = 0
      let a = 0
      let cnt = 0
      for (let py = by; py < by + bh; py++) {
        let row = (py * rw + bx) * 4
        for (let px = 0; px < bw; px++) {
          r += data[row]
          g += data[row + 1]
          b += data[row + 2]
          a += data[row + 3]
          row += 4
          cnt++
        }
      }
      if (!cnt) continue
      const mr = (r / cnt) | 0
      const mg = (g / cnt) | 0
      const mb = (b / cnt) | 0
      const ma = (a / cnt) | 0
      for (let py = by; py < by + bh; py++) {
        let row = (py * rw + bx) * 4
        for (let px = 0; px < bw; px++) {
          data[row] = mr
          data[row + 1] = mg
          data[row + 2] = mb
          data[row + 3] = ma
          row += 4
        }
      }
    }
  }
  ctx.putImageData(imgData, left, top)
}

export function drawElement(
  ctx: CanvasRenderingContext2D,
  el: CanvasElement,
  imageCache: Map<string, HTMLImageElement>,
  skipTextId: string | null = null,
) {
  ctx.save()
  ctx.globalAlpha = Math.max(0, Math.min(1, el.opacity))

  if (el.type === 'image') {
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    ctx.translate(cx, cy)
    if (el.rotation) ctx.rotate(el.rotation)
    if (el.flipX || el.flipY) ctx.scale(el.flipX ? -1 : 1, el.flipY ? -1 : 1)

    const rx = -b.w * 0.5
    const ry = -b.h * 0.5
    const rad = Math.min(el.radius, b.w * 0.5, b.h * 0.5)

    if (el.shadow) {
      ctx.save()
      ctx.shadowColor = 'rgba(0, 0, 0, 0.28)'
      ctx.shadowBlur = 24
      ctx.shadowOffsetX = 0
      ctx.shadowOffsetY = 10
      ctx.fillStyle = '#000000'
      ctx.beginPath()
      ctx.roundRect(rx, ry, b.w, b.h, rad)
      ctx.fill()
      ctx.restore()
    }

    ctx.beginPath()
    ctx.roundRect(rx, ry, b.w, b.h, rad)
    ctx.clip()

    const img = imageCache.get(el.id) || imageCache.get(el.src)
    if (img && img.complete && img.naturalWidth > 0) {
      ctx.drawImage(img, rx, ry, b.w, b.h)
    } else {
      ctx.fillStyle = '#27272a'
      ctx.fillRect(rx, ry, b.w, b.h)
    }
    ctx.restore()
    return
  }

  if (el.type === 'mosaic') {
    const b = getElementBounds(el)
    applyMosaicRegion(ctx, b.x, b.y, b.w, b.h, el.pixelSize)
    ctx.restore()
    return
  }

  if (el.type === 'rect') {
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    ctx.translate(cx, cy)
    if (el.rotation) ctx.rotate(el.rotation)
    const rad = Math.min(el.radius, b.w * 0.5, b.h * 0.5)
    ctx.beginPath()
    ctx.roundRect(-b.w * 0.5, -b.h * 0.5, b.w, b.h, rad)
    if (el.fill && el.fill !== 'transparent') {
      ctx.fillStyle = el.fill
      ctx.fill()
    }
    if (el.strokeWidth > 0) {
      ctx.strokeStyle = el.stroke
      ctx.lineWidth = el.strokeWidth
      if (el.dashed) ctx.setLineDash([el.strokeWidth * 3, el.strokeWidth * 2])
      ctx.stroke()
    }
    ctx.restore()
    return
  }

  if (el.type === 'ellipse') {
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    ctx.translate(cx, cy)
    if (el.rotation) ctx.rotate(el.rotation)
    ctx.beginPath()
    ctx.ellipse(0, 0, Math.max(1, b.w * 0.5), Math.max(1, b.h * 0.5), 0, 0, Math.PI * 2)
    if (el.fill && el.fill !== 'transparent') {
      ctx.fillStyle = el.fill
      ctx.fill()
    }
    if (el.strokeWidth > 0) {
      ctx.strokeStyle = el.stroke
      ctx.lineWidth = el.strokeWidth
      if (el.dashed) ctx.setLineDash([el.strokeWidth * 3, el.strokeWidth * 2])
      ctx.stroke()
    }
    ctx.restore()
    return
  }

  if (el.type === 'arrow' || el.type === 'line') {
    ctx.strokeStyle = el.stroke
    ctx.fillStyle = el.stroke
    ctx.lineWidth = el.strokeWidth
    ctx.lineCap = 'round'
    ctx.lineJoin = 'round'
    if (el.dashed) ctx.setLineDash([el.strokeWidth * 3, el.strokeWidth * 2])
    ctx.beginPath()
    ctx.moveTo(el.x, el.y)
    ctx.lineTo(el.x2, el.y2)
    ctx.stroke()
    ctx.setLineDash([])
    if (el.type === 'arrow') {
      const head = Math.max(14, el.strokeWidth * 3.8)
      drawArrowhead(ctx, el.x, el.y, el.x2, el.y2, head)
      if (el.doubleHead) {
        drawArrowhead(ctx, el.x2, el.y2, el.x, el.y, head)
      }
    }
    ctx.restore()
    return
  }

  if (el.type === 'pen') {
    if (!el.points.length) {
      ctx.restore()
      return
    }
    ctx.strokeStyle = el.stroke
    ctx.lineWidth = el.strokeWidth
    ctx.lineCap = 'round'
    ctx.lineJoin = 'round'
    if (el.highlighter) {
      ctx.globalAlpha = Math.min(el.opacity, 0.36)
    }
    ctx.beginPath()
    ctx.moveTo(el.points[0].x, el.points[0].y)
    if (el.points.length === 1) {
      ctx.lineTo(el.points[0].x + 0.1, el.points[0].y + 0.1)
    } else if (el.points.length === 2) {
      ctx.lineTo(el.points[1].x, el.points[1].y)
    } else {
      for (let i = 1; i < el.points.length - 1; i++) {
        const midX = (el.points[i].x + el.points[i + 1].x) * 0.5
        const midY = (el.points[i].y + el.points[i + 1].y) * 0.5
        ctx.quadraticCurveTo(el.points[i].x, el.points[i].y, midX, midY)
      }
      const last = el.points[el.points.length - 1]
      ctx.lineTo(last.x, last.y)
    }
    ctx.stroke()
    ctx.restore()
    return
  }

  if (el.type === 'text') {
    if (skipTextId === el.id) {
      ctx.restore()
      return
    }
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    ctx.translate(cx, cy)
    if (el.rotation) ctx.rotate(el.rotation)
    const padX = 8
    const padY = 5
    const lines = (el.text || '').split('\n')
    const lineHeight = el.fontSize * 1.32
    ctx.font = `${el.bold ? '700' : '500'} ${el.fontSize}px "Microsoft YaHei UI", "Segoe UI", sans-serif`
    if (el.bgColor && el.bgColor !== 'transparent') {
      ctx.fillStyle = el.bgColor
      ctx.beginPath()
      ctx.roundRect(-b.w * 0.5, -b.h * 0.5, b.w, b.h, 8)
      ctx.fill()
    }
    ctx.fillStyle = el.color
    ctx.textBaseline = 'top'
    ctx.textAlign = 'left'
    for (let i = 0; i < lines.length; i++) {
      ctx.fillText(lines[i], -b.w * 0.5 + padX, -b.h * 0.5 + padY + i * lineHeight)
    }
    ctx.restore()
    return
  }

  if (el.type === 'number') {
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    const r = Math.max(12, Math.min(b.w, b.h) * 0.5)
    ctx.save()
    ctx.shadowColor = 'rgba(0,0,0,0.28)'
    ctx.shadowBlur = 8
    ctx.shadowOffsetY = 2
    ctx.fillStyle = el.color
    ctx.beginPath()
    ctx.arc(cx, cy, r, 0, Math.PI * 2)
    ctx.fill()
    ctx.restore()

    ctx.strokeStyle = '#ffffff'
    ctx.lineWidth = 2.2
    ctx.beginPath()
    ctx.arc(cx, cy, r, 0, Math.PI * 2)
    ctx.stroke()

    ctx.fillStyle = '#ffffff'
    ctx.font = `700 ${Math.round(r * 1.1)}px "Segoe UI", "Microsoft YaHei UI", sans-serif`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText(String(el.value), cx, cy + 0.5)
    ctx.restore()
    return
  }

  ctx.restore()
}

export function renderInteractiveCanvas(
  canvas: HTMLCanvasElement,
  opts: RenderOptions,
) {
  const rect = canvas.getBoundingClientRect()
  const dpr = Math.min(window.devicePixelRatio || 1, 2)
  const targetW = Math.max(1, Math.round(rect.width * dpr))
  const targetH = Math.max(1, Math.round(rect.height * dpr))
  if (canvas.width !== targetW || canvas.height !== targetH) {
    canvas.width = targetW
    canvas.height = targetH
  }
  const ctx = canvas.getContext('2d')
  if (!ctx) return

  const viewW = rect.width
  const viewH = rect.height
  const { viewport: vp, paper, elements, imageCache, selectedIds, isDarkTheme } = opts

  ctx.save()
  ctx.scale(dpr, dpr)

  // 1. Workspace backdrop
  ctx.fillStyle = isDarkTheme ? '#131316' : '#eff1f5'
  ctx.fillRect(0, 0, viewW, viewH)

  // 2. Infinite dot grid
  if (paper.showGrid) {
    drawDotGrid(ctx, viewW, viewH, vp, isDarkTheme)
  }

  // 3. World transform
  ctx.save()
  ctx.translate(vp.panX, vp.panY)
  ctx.scale(vp.zoom, vp.zoom)

  // 4. Optional fixed Artboard paper
  if (paper.artboard) {
    ctx.save()
    ctx.shadowColor = isDarkTheme ? 'rgba(0, 0, 0, 0.5)' : 'rgba(15, 23, 42, 0.16)'
    ctx.shadowBlur = 32 / vp.zoom
    ctx.shadowOffsetY = 8 / vp.zoom
    ctx.fillStyle = paper.bg === 'transparent' ? '#ffffff' : '#000000'
    ctx.fillRect(0, 0, paper.width, paper.height)
    ctx.restore()

    if (paper.bg === 'transparent') {
      drawCheckerboard(ctx, 0, 0, paper.width, paper.height, 16)
    } else {
      fillBackgroundStyle(ctx, paper.bg, 0, 0, paper.width, paper.height)
    }

    ctx.strokeStyle = isDarkTheme ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.1)'
    ctx.lineWidth = 1 / vp.zoom
    ctx.strokeRect(0, 0, paper.width, paper.height)
  }

  // 5. Render scene elements in z-order
  for (const el of elements) {
    drawElement(ctx, el, imageCache, opts.editingTextId)
  }
  if (opts.draftElement) {
    drawElement(ctx, opts.draftElement, imageCache, null)
  }

  // 6. Selection chrome (SnowAir green #34C759 + crisp Excalidraw/Figma handles)
  const selectedList = elements.filter((e) => selectedIds.has(e.id))
  if (selectedList.length === 1) {
    const el = selectedList[0]
    const b = getElementBounds(el)
    const cx = b.x + b.w * 0.5
    const cy = b.y + b.h * 0.5
    ctx.save()
    ctx.translate(cx, cy)
    if (el.rotation) ctx.rotate(el.rotation)
    ctx.strokeStyle = '#34c759'
    ctx.lineWidth = 1.6 / vp.zoom
    ctx.strokeRect(-b.w * 0.5, -b.h * 0.5, b.w, b.h)
    ctx.restore()

    // Handles for resizable elements
    if (el.type !== 'arrow' && el.type !== 'line' && el.type !== 'pen' && el.type !== 'number') {
      const handles = getHandlePositions(el, vp.zoom)
      // Stem to rotation handle
      ctx.save()
      ctx.strokeStyle = '#34c759'
      ctx.lineWidth = 1.5 / vp.zoom
      ctx.beginPath()
      ctx.moveTo(handles.n.x, handles.n.y)
      ctx.lineTo(handles.rotate.x, handles.rotate.y)
      ctx.stroke()

      const hs = 8 / vp.zoom
      for (const k of ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as HandleId[]) {
        const p = handles[k]
        ctx.fillStyle = '#ffffff'
        ctx.strokeStyle = '#34c759'
        ctx.lineWidth = 1.6 / vp.zoom
        ctx.beginPath()
        ctx.roundRect(p.x - hs * 0.5, p.y - hs * 0.5, hs, hs, 2 / vp.zoom)
        ctx.fill()
        ctx.stroke()
      }
      // Circular rotation knob
      const rr = 5.2 / vp.zoom
      ctx.fillStyle = '#ffffff'
      ctx.strokeStyle = '#34c759'
      ctx.lineWidth = 1.8 / vp.zoom
      ctx.beginPath()
      ctx.arc(handles.rotate.x, handles.rotate.y, rr, 0, Math.PI * 2)
      ctx.fill()
      ctx.stroke()
      ctx.restore()
    } else if (el.type === 'arrow' || el.type === 'line') {
      for (const pt of [
        { x: el.x, y: el.y },
        { x: el.x2, y: el.y2 },
      ]) {
        const rr = 5 / vp.zoom
        ctx.fillStyle = '#ffffff'
        ctx.strokeStyle = '#34c759'
        ctx.lineWidth = 1.8 / vp.zoom
        ctx.beginPath()
        ctx.arc(pt.x, pt.y, rr, 0, Math.PI * 2)
        ctx.fill()
        ctx.stroke()
      }
    }

    if (opts.showLiveSizeBadge) {
      const label = `${Math.round(b.w)} × ${Math.round(b.h)}`
      ctx.save()
      ctx.font = `600 ${11 / vp.zoom}px "Segoe UI", sans-serif`
      const tw = ctx.measureText(label).width
      const padX = 6 / vp.zoom
      const bh = 18 / vp.zoom
      const bx = cx - tw * 0.5 - padX
      const by = b.y + b.h + 8 / vp.zoom
      ctx.fillStyle = '#34c759'
      ctx.beginPath()
      ctx.roundRect(bx, by, tw + padX * 2, bh, 4 / vp.zoom)
      ctx.fill()
      ctx.fillStyle = '#ffffff'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillText(label, cx, by + bh * 0.5)
      ctx.restore()
    }
  } else if (selectedList.length > 1) {
    // Individual faint boxes + combined outer frame
    for (const el of selectedList) {
      const b = getElementBounds(el)
      ctx.strokeStyle = 'rgba(52, 199, 89, 0.5)'
      ctx.lineWidth = 1.1 / vp.zoom
      ctx.strokeRect(b.x, b.y, b.w, b.h)
    }
    const mb = getMultiBounds(selectedList)
    if (mb) {
      ctx.strokeStyle = '#34c759'
      ctx.lineWidth = 1.6 / vp.zoom
      ctx.setLineDash([5 / vp.zoom, 3 / vp.zoom])
      ctx.strokeRect(mb.x, mb.y, mb.w, mb.h)
      ctx.setLineDash([])
    }
  }

  // 7. Rubber-band marquee selection rectangle
  if (opts.marquee) {
    const m = opts.marquee
    ctx.fillStyle = 'rgba(52, 199, 89, 0.12)'
    ctx.strokeStyle = '#34c759'
    ctx.lineWidth = 1.2 / vp.zoom
    ctx.fillRect(m.x, m.y, m.w, m.h)
    ctx.strokeRect(m.x, m.y, m.w, m.h)
  }

  ctx.restore()
  ctx.restore()
}

/**
 * Renders a clean export bitmap (zero grid dots, zero selection chrome) of either
 * the fixed Artboard or the tight bounding box of all canvas elements.
 */
export function exportSceneToDataUrl(
  elements: CanvasElement[],
  imageCache: Map<string, HTMLImageElement>,
  paper: CanvasPaper,
  mime: 'image/png' | 'image/jpeg' | 'image/webp' = 'image/png',
  scale = 1,
): { dataUrl: string; width: number; height: number } | null {
  let box: Bounds
  if (paper.artboard) {
    box = { x: 0, y: 0, w: Math.max(16, paper.width), h: Math.max(16, paper.height) }
  } else {
    const content = getMultiBounds(elements)
    if (!content) return null
    const pad = 28
    box = {
      x: content.x - pad,
      y: content.y - pad,
      w: content.w + pad * 2,
      h: content.h + pad * 2,
    }
  }

  // Cap max dimension at 4096 so export stays fast & memory-safe
  const maxDim = Math.max(box.w * scale, box.h * scale)
  const effScale = maxDim > 4096 ? (4096 / Math.max(box.w, box.h)) : scale
  const outW = Math.max(1, Math.round(box.w * effScale))
  const outH = Math.max(1, Math.round(box.h * effScale))

  const off = document.createElement('canvas')
  off.width = outW
  off.height = outH
  const ctx = off.getContext('2d')
  if (!ctx) return null

  ctx.scale(effScale, effScale)
  ctx.translate(-box.x, -box.y)

  if (paper.bg !== 'transparent') {
    fillBackgroundStyle(ctx, paper.bg, box.x, box.y, box.w, box.h)
  } else if (mime === 'image/jpeg') {
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(box.x, box.y, box.w, box.h)
  }

  for (const el of elements) {
    drawElement(ctx, el, imageCache, null)
  }

  const quality = mime === 'image/png' ? undefined : 0.94
  return {
    dataUrl: off.toDataURL(mime, quality),
    width: outW,
    height: outH,
  }
}
