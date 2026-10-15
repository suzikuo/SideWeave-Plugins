/** 工具箱共享逻辑：注入 API、防抖、Base64/哈希编码、JSON 错误定位、UUID、颜色转换 */
import { onScopeDispose, ref, watch } from 'vue'
import type { InjectionKey, Ref } from 'vue'

// ---------- 注入 API ----------

export type ToastKind = 'info' | 'error' | 'success'

/** ToolboxView 提供的全局能力：剪贴板复制与 Toast 提示 */
export interface ToolboxApi {
  copyText(text: string, okMsg?: string): Promise<void>
  showToast(message: string, kind?: ToastKind): void
}

export const TOOLBOX_API_KEY: InjectionKey<ToolboxApi> = Symbol('toolbox-api')

/** 兜底实现（ToolboxView 始终注入，仅防御性保留） */
export const FALLBACK_API: ToolboxApi = {
  async copyText() {},
  showToast() {}
}

// ---------- 通用 ----------

/** 防抖 ref：输入即时计算场景使用（默认 200ms） */
export function debouncedRef<T>(source: Ref<T>, delay = 200): Ref<T> {
  const result = ref(source.value) as Ref<T>
  let timer = 0
  watch(source, (value) => {
    window.clearTimeout(timer)
    timer = window.setTimeout(() => {
      result.value = value
    }, delay)
  })
  onScopeDispose(() => window.clearTimeout(timer))
  return result
}

export function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

/** 本地时间 "YYYY-MM-DD HH:mm:ss" */
export function formatLocal(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
}

/** datetime-local 输入框取值 "YYYY-MM-DDTHH:mm:ss"（按本地时区解释） */
export function toLocalInputValue(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
}

// ---------- Base64（UTF-8 安全） ----------

/** 分块大小：避免 String.fromCharCode 展开参数超出引擎栈上限 */
const BINARY_CHUNK = 0x8000

function bytesToBinaryString(bytes: Uint8Array): string {
  let binary = ''
  for (let i = 0; i < bytes.length; i += BINARY_CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + BINARY_CHUNK))
  }
  return binary
}

/** 文本 → Base64：TextEncoder 编码后分块 btoa，非 ASCII 与大文本均安全 */
export function encodeBase64(text: string): string {
  return btoa(bytesToBinaryString(new TextEncoder().encode(text)))
}

/** Base64 → 文本：容忍 data: 前缀、URL-safe 变体（-_）与缺失填充 */
export function decodeBase64(b64: string): string {
  const stripped = b64.trim().replace(/^data:[^,]*,/i, '')
  const cleaned = stripped.replace(/\s+/g, '').replace(/-/g, '+').replace(/_/g, '/')
  const padded = cleaned + '='.repeat((4 - (cleaned.length % 4)) % 4)
  const binary = atob(padded)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return new TextDecoder().decode(bytes)
}

// ---------- 哈希输出编码 ----------

/** ArrayBuffer → 小写 hex */
export function bufferToHex(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf)
  let hex = ''
  for (let i = 0; i < bytes.length; i++) hex += bytes[i].toString(16).padStart(2, '0')
  return hex
}

/** ArrayBuffer → 标准 Base64 */
export function bufferToBase64(buf: ArrayBuffer): string {
  return btoa(bytesToBinaryString(new Uint8Array(buf)))
}

// ---------- JSON 校验 ----------

export interface JsonIssue {
  line: number
  column: number
  message: string
}

/** JSON.parse 包装：失败时从错误信息中的 position 换算出行/列号 */
export function tryParseJson(text: string): { value?: unknown; error?: JsonIssue } {
  try {
    return { value: JSON.parse(text) }
  } catch (e) {
    const raw = e instanceof Error ? e.message : String(e)
    const m = /position\s+(\d+)/i.exec(raw)
    let line = 1
    let column = 1
    if (m) {
      const pos = Math.min(Number(m[1]), text.length)
      for (let i = 0; i < pos; i++) {
        if (text.charCodeAt(i) === 10) {
          line++
          column = 1
        } else {
          column++
        }
      }
    }
    const message =
      raw
        .replace(/\s*\(line \d+ column \d+\)/i, '')
        .replace(/\s+at position\s+\d+/i, '')
        .trim() || raw
    return { error: { line, column, message } }
  }
}

// ---------- UUID ----------

/** v4 UUID：优先 crypto.randomUUID，缺失时用 getRandomValues 手工拼装 */
export function generateUuid(): string {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

// ---------- 颜色转换 ----------

export interface Rgba {
  r: number
  g: number
  b: number
  a: number
}

export interface Hsla {
  h: number
  s: number
  l: number
  a: number
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v))
}

function round1(v: number): number {
  return Math.round(v * 10) / 10
}

function round2(v: number): number {
  return Math.round(v * 100) / 100
}

function parseAlpha(raw: string | undefined): number {
  if (raw === undefined) return 1
  const v = raw.endsWith('%') ? parseFloat(raw) / 100 : parseFloat(raw)
  return Number.isFinite(v) ? clamp(v, 0, 1) : 1
}

/** 解析 #HEX / rgb() / hsl() 文本（含 rgba/hsla、百分比、空格与斜杠写法），失败返回 null */
export function parseColor(input: string): Rgba | null {
  const text = input.trim().toLowerCase()
  if (!text) return null

  // #RGB / #RGBA / #RRGGBB / #RRGGBBAA（允许省略 #）
  const hex = /^#?([0-9a-f]+)$/.exec(text)
  if (hex && [3, 4, 6, 8].includes(hex[1].length)) {
    const h = hex[1]
    if (h.length <= 4) {
      const expand = (c: string) => parseInt(c + c, 16)
      return {
        r: expand(h[0]),
        g: expand(h[1]),
        b: expand(h[2]),
        a: h.length === 4 ? expand(h[3]) / 255 : 1
      }
    }
    return {
      r: parseInt(h.slice(0, 2), 16),
      g: parseInt(h.slice(2, 4), 16),
      b: parseInt(h.slice(4, 6), 16),
      a: h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1
    }
  }

  // rgb() / rgba() / hsl() / hsla()
  const fn = /^(rgba?|hsla?)\(([^)]*)\)$/.exec(text)
  if (!fn) return null
  const isHsl = fn[1].startsWith('hsl')
  const parts = fn[2].trim().split(/[\s,/]+/).filter(Boolean)
  if (parts.length < 3) return null
  const alpha = parseAlpha(parts[3])

  /** 通道值：百分比或 0-255 数字 */
  const channel = (raw: string): number =>
    raw.endsWith('%') ? (parseFloat(raw) / 100) * 255 : parseFloat(raw)

  /** 饱和度/亮度：百分比优先，裸数字按 0-1 小数或 0-100 数值都接受 */
  const percentish = (raw: string): number => {
    if (raw.endsWith('%')) return parseFloat(raw)
    const v = parseFloat(raw)
    return Number.isFinite(v) && v <= 1 ? v * 100 : v
  }

  if (isHsl) {
    const hDeg = parseFloat(parts[0])
    if (
      !Number.isFinite(hDeg) ||
      !parts.slice(1, 3).every((p) => Number.isFinite(parseFloat(p)))
    ) {
      return null
    }
    return hslToRgb({
      h: ((hDeg % 360) + 360) % 360,
      s: clamp(percentish(parts[1]), 0, 100),
      l: clamp(percentish(parts[2]), 0, 100),
      a: alpha
    })
  }

  const r = channel(parts[0])
  const g = channel(parts[1])
  const b = channel(parts[2])
  if (![r, g, b].every(Number.isFinite)) return null
  return {
    r: Math.round(clamp(r, 0, 255)),
    g: Math.round(clamp(g, 0, 255)),
    b: Math.round(clamp(b, 0, 255)),
    a: alpha
  }
}

/** RGB → HSL（h / s / l 保留 1 位小数，s、l 为 0-100） */
export function rgbToHsl({ r, g, b, a }: Rgba): Hsla {
  const rn = r / 255
  const gn = g / 255
  const bn = b / 255
  const max = Math.max(rn, gn, bn)
  const min = Math.min(rn, gn, bn)
  const l = (max + min) / 2
  let h = 0
  let s = 0
  if (max !== min) {
    const d = max - min
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    if (max === rn) h = (gn - bn) / d + (gn < bn ? 6 : 0)
    else if (max === gn) h = (bn - rn) / d + 2
    else h = (rn - gn) / d + 4
    h *= 60
  }
  return { h: round1(h), s: round1(s * 100), l: round1(l * 100), a }
}

/** HSL → RGB（通道值四舍五入到整数） */
export function hslToRgb({ h, s, l, a }: Hsla): Rgba {
  const sn = s / 100
  const ln = l / 100
  const k = (n: number) => (n + h / 30) % 12
  const f = (n: number) =>
    ln - sn * Math.min(ln, 1 - ln) * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1))
  return {
    r: Math.round(f(0) * 255),
    g: Math.round(f(8) * 255),
    b: Math.round(f(4) * 255),
    a
  }
}

/** #RRGGBB（带透明度时为 #RRGGBBAA） */
export function formatHex({ r, g, b, a }: Rgba): string {
  const to2 = (v: number) => clamp(Math.round(v), 0, 255).toString(16).padStart(2, '0')
  const base = `#${to2(r)}${to2(g)}${to2(b)}`
  return a < 1 ? base + to2(a * 255) : base
}

export function formatRgb({ r, g, b, a }: Rgba): string {
  return a < 1 ? `rgba(${r}, ${g}, ${b}, ${round2(a)})` : `rgb(${r}, ${g}, ${b})`
}

export function formatHsl({ h, s, l, a }: Hsla): string {
  return a < 1 ? `hsla(${h}, ${s}%, ${l}%, ${round2(a)})` : `hsl(${h}, ${s}%, ${l}%)`
}
