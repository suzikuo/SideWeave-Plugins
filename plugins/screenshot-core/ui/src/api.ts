import { pluginCall } from '../../../ui-sdk/runtime'

export type ThemeMode = 'system' | 'dark' | 'light'
export type ToolbarThemeMode = 'system' | 'dark' | 'light' | 'custom'
export type ToolbarSizeMode = 'small' | 'medium' | 'large'
export type SaveFormat = 'png' | 'jpg' | 'webp'
export type FinishAction = 'copy_and_save' | 'copy_only' | 'save_only'
export type PinDoubleClick = 'thumbnail' | 'close' | 'copy'
export type DemoDefaultTool = 'fade_pen' | 'pen' | 'none'
export type DemoToolbarPos = 'top' | 'bottom'
export type OcrEngine = 'system' | 'tencent'
export type TranslateEngine = 'youdao' | 'microsoft' | 'google' | 'baidu'

export interface ScreenshotSettings {
  // Hotkeys
  hotkey: string
  recordHotkey: string
  demoHotkey: string
  scrollHotkey: string
  quickPinHotkey: string
  doubleTapF1: boolean

  // Appearance
  themeMode: ThemeMode
  toolbarTheme: ToolbarThemeMode
  toolbarSize: ToolbarSizeMode
  toolbarCustomBg: string
  toolbarCustomHover: string
  toolbarCustomIcon: string
  toolbarCustomAccent: string
  toolbarCustomSelection: string
  toolbarActive: string[]
  toolbarInactive: string[]

  // Capture & Output
  saveDirectory: string
  fileNameTemplate: string
  saveFormat: SaveFormat
  autoSave: boolean
  autoCopy: boolean
  finishAction: FinishAction
  detectWindow: boolean
  defaultCornerRadius: number
  defaultShadow: boolean
  scrollStep: number
  hdrCaptureMode: 'auto' | 'force_on' | 'force_off'
  watermarkText: string

  // Annotation Defaults
  annotationColor: string
  strokeWidth: number
  brushSize: number
  textSize: number

  // Pin
  pinCursorZoom: boolean
  pinAutoOcr: boolean
  pinDoubleClick: PinDoubleClick

  // Demo
  demoDefaultTool: DemoDefaultTool
  demoFadeMs: number
  demoToolbarPos: DemoToolbarPos

  // OCR & Translate
  ocrEngine: OcrEngine
  ocrRegion: string
  ocrSelectModel: number
  tencentOcrConfigured?: boolean
  systemPrefersLight?: boolean
  translateEngine: TranslateEngine
  translateTargetLang: string

  // Recording
  recordingFps: number
  recordingMaxHeight: number
  recordingBitrateMbps: number
  recordingGifColors: number
  recordingWebpQuality: number
  recordingShowCursor: boolean
  recordingDefaultPassthrough: boolean
}

export interface ScreenshotStatus {
  active: boolean
  demoActive?: boolean
  lastPath: string | null
  lastSize: { width: number; height: number } | null
  ocrText: string | null
  pinned: boolean
  error: string | null
  recording: RecordingStatus
}

export interface RecordingStatus {
  phase:
    | 'idle'
    | 'selecting'
    | 'recording'
    | 'processing'
    | 'preview'
    | 'exporting'
    | 'cancelled'
    | 'cancelling'
    | 'error'
  paused: boolean
  fps: number
  frameCount: number
  elapsedMs: number
  progressPercent: number
  maxDurationMs: number
  message: string | null
  previewPath: string | null
  lastPath: string | null
}

export interface TencentCredentialsUpdate {
  secretId: string
  secretKey: string
  token: string
  clear: boolean
}

export interface TranslateResponse {
  text: string
  providerUsed: string
}

export interface HistoryItem {
  id: string
  kind: 'screenshot' | 'scroll' | 'recording' | 'edit' | string
  path: string
  width: number
  height: number
  createdAtMs: number
  fileSize: number
  tags: string[]
}

export interface ImageDataUrlResponse {
  dataUrl: string
  width: number
  height: number
}

export interface ExportDocumentResponse {
  path: string
  width: number
  height: number
}

export const DEFAULT_TOOLBAR_ACTIVE = [
  'select',
  'rect',
  'arrow',
  'pen',
  'text',
  'number',
  'mosaic',
  'eraser',
  'undo',
  'redo',
  'record',
  'pin',
  'ocr',
  'translate',
  'scroll',
  'save',
  'finish',
]

export const DEFAULT_TOOLBAR_INACTIVE = ['edit', 'qr']

export const DEFAULT_SETTINGS: ScreenshotSettings = {
  hotkey: 'ctrl+shift+a',
  recordHotkey: 'alt+shift+r',
  demoHotkey: 'alt+shift+d',
  scrollHotkey: 'alt+shift+s',
  quickPinHotkey: 'alt+shift+p',
  doubleTapF1: true,

  themeMode: 'system',
  toolbarTheme: 'system',
  toolbarSize: 'medium',
  toolbarCustomBg: '#1e1e1f',
  toolbarCustomHover: '#333336',
  toolbarCustomIcon: '#f2f2f5',
  toolbarCustomAccent: '#34c759',
  toolbarCustomSelection: '#34c759',
  toolbarActive: [...DEFAULT_TOOLBAR_ACTIVE],
  toolbarInactive: [...DEFAULT_TOOLBAR_INACTIVE],

  saveDirectory: '',
  fileNameTemplate: 'SideWeave_{{YYYY}}{{MM}}{{DD}}_{{HH}}{{mm}}{{ss}}',
  saveFormat: 'png',
  autoSave: false,
  autoCopy: true,
  finishAction: 'copy_and_save',
  detectWindow: true,
  defaultCornerRadius: 0,
  defaultShadow: false,
  scrollStep: 640,
  hdrCaptureMode: 'auto',
  watermarkText: '',

  annotationColor: '#ef4444',
  strokeWidth: 4,
  brushSize: 24,
  textSize: 18,

  pinCursorZoom: true,
  pinAutoOcr: false,
  pinDoubleClick: 'thumbnail',

  demoDefaultTool: 'fade_pen',
  demoFadeMs: 2400,
  demoToolbarPos: 'top',

  ocrEngine: 'system',
  ocrRegion: '',
  ocrSelectModel: 0,
  tencentOcrConfigured: false,
  systemPrefersLight: false,
  translateEngine: 'youdao',
  translateTargetLang: 'zh-CN',

  recordingFps: 15,
  recordingMaxHeight: 1080,
  recordingBitrateMbps: 16,
  recordingGifColors: 256,
  recordingWebpQuality: 90,
  recordingShowCursor: true,
  recordingDefaultPassthrough: true,
}

const call = <T>(method: string, payload: Record<string, unknown> = {}, timeoutMs = 12_000) =>
  pluginCall<T>(method, payload, timeoutMs)

export const getSettings = () => call<ScreenshotSettings>('get_settings')

export const setSettings = (
  settings: ScreenshotSettings,
  tencentCredentials?: TencentCredentialsUpdate,
) =>
  call<void>('set_settings', {
    ...settings,
    ...(tencentCredentials ? { tencentCredentials } : {}),
  })

export const startCapture = (mode: 'region' | 'scroll' = 'region') =>
  call<void>('start_capture', { mode })
export const startDemo = () => call<void>('start_demo')
export const quickPin = () => call<void>('quick_pin')
export const pinLast = () => call<void>('pin_last')
export const pinFile = (path: string) => call<void>('pin_file', { path })
export const copyFile = (path: string) => call<void>('copy_file', { path })

interface ImageChunkResponse {
  chunk: string
  chunkIndex: number
  totalChunks: number
  mime: string
  width: number
  height: number
}

export async function uploadTempDataUrl(dataUrl: string): Promise<string> {
  const commaIdx = dataUrl.indexOf(',')
  if (commaIdx < 0) {
    throw new Error('无效的图片 DataURL')
  }
  const header = dataUrl.slice(0, commaIdx).toLowerCase()
  const ext = header.includes('jpeg') || header.includes('jpg')
    ? 'jpg'
    : header.includes('webp')
      ? 'webp'
      : 'png'
  const b64 = dataUrl.slice(commaIdx + 1).replace(/\s+/g, '')
  const uploadId = `u_${Date.now()}_${Math.random().toString(36).slice(2, 10)}`
  // Must be a multiple of 4 so each chunk decodes independently to exact bytes
  const chunkSize = 120_000
  let tempPath = ''
  for (let offset = 0, idx = 0; offset < b64.length || idx === 0; offset += chunkSize, idx++) {
    const chunkBase64 = b64.slice(offset, offset + chunkSize)
    const res = await call<{ path: string }>(
      'upload_temp_chunk',
      { uploadId, chunkBase64, append: idx > 0, ext },
      20_000,
    )
    tempPath = res.path
  }
  return tempPath
}

export async function copyImageDataUrl(dataUrl: string): Promise<void> {
  if (dataUrl.length > 150_000) {
    const tempPath = await uploadTempDataUrl(dataUrl)
    await copyFile(tempPath)
    return
  }
  await call<void>('copy_image_data_url', { dataUrl }, 20_000)
}

export const startRecording = (fps: number) => call<void>('start_recording', { fps })
export const stopRecording = () => call<void>('stop_recording')
export const cancelRecording = () => call<void>('cancel_recording')
export const exportRecording = (format: 'webp' | 'gif' | 'mp4') =>
  call<void>('export_recording', { format })
export const clearRecording = () => call<void>('clear_recording')
export const cancelCapture = () => call<void>('cancel_capture')
export const getStatus = () => call<ScreenshotStatus>('get_status')
export const clearOcr = () => call<void>('clear_ocr')

export const chooseSaveDirectory = () =>
  call<string | null>('choose_save_directory', {}, 60_000)
export const openSaveDirectory = (path?: string) =>
  call<void>('open_save_directory', { path })

export const quickTranslate = (
  text: string,
  sourceLang: string,
  targetLang: string,
  provider: TranslateEngine,
) =>
  call<TranslateResponse>(
    'quick_translate',
    { text, sourceLang, targetLang, provider },
    25_000,
  )

export const getHistory = (params?: {
  kind?: string
  tag?: string
  query?: string
  includeDirScan?: boolean
}) => call<HistoryItem[]>('get_history', (params ?? {}) as Record<string, unknown>, 15_000)

export const addHistoryTag = (id: string, path: string, tag: string) =>
  call<string[]>('add_history_tag', { id, path, tag })

export const removeHistoryTag = (id: string, path: string, tag: string) =>
  call<string[]>('remove_history_tag', { id, path, tag })

export const deleteHistoryItem = (id: string, path: string, deleteFile = false) =>
  call<void>('delete_history_item', { id, path, deleteFile })

export const clearHistory = () => call<void>('clear_history')
export const clearThumbCache = () => call<void>('clear_thumb_cache')

export const openHistoryFile = (path: string, reveal = false) =>
  call<void>('open_history_file', { path, reveal })

export async function readImageDataUrl(
  path: string,
  maxDim = 0,
): Promise<ImageDataUrlResponse> {
  // Fast disk-cached single-frame path for thumbnails
  if (maxDim > 0 && maxDim <= 480) {
    return call<ImageDataUrlResponse>('read_image_data_url', { path, maxDim }, 20_000)
  }
  // Safe chunked IPC transfer for high-resolution Studio images (never exceeds 256 KB limit)
  const chunkSize = 140_000
  const first = await call<ImageChunkResponse>(
    'read_image_chunk',
    { path, maxDim, chunkIndex: 0, chunkSize },
    25_000,
  )
  const parts: string[] = [first.chunk]
  for (let i = 1; i < first.totalChunks; i++) {
    const next = await call<ImageChunkResponse>(
      'read_image_chunk',
      { path, maxDim, chunkIndex: i, chunkSize },
      25_000,
    )
    parts.push(next.chunk)
  }
  return {
    dataUrl: `data:${first.mime};base64,${parts.join('')}`,
    width: first.width,
    height: first.height,
  }
}

export const chooseImages = () => call<string[]>('choose_images', {}, 60_000)

export async function exportDocument(req: {
  pages: { source: string }[]
  format: 'pdf' | 'pptx' | 'png' | 'jpg' | 'webp'
  quality?: number
  outputPath?: string
  saveDirectory?: string
}): Promise<ExportDocumentResponse> {
  const safePages: { source: string }[] = []
  for (const page of req.pages) {
    if (page.source.startsWith('data:') && page.source.length > 150_000) {
      const tempPath = await uploadTempDataUrl(page.source)
      safePages.push({ source: tempPath })
    } else {
      safePages.push(page)
    }
  }
  return call<ExportDocumentResponse>(
    'export_document',
    { ...req, pages: safePages } as Record<string, unknown>,
    60_000,
  )
}
