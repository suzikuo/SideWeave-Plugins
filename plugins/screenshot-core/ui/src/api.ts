import { pluginCall } from '../../../ui-sdk/runtime'

export interface ScreenshotSettings {
  hotkey: string
  saveDirectory: string
  autoCopy: boolean
  scrollStep: number
  watermarkText: string
  annotationColor: string
  textSize: number
  ocrRegion: string
  ocrSelectModel: number
  tencentOcrConfigured: boolean
  recordingMp4Fps: number
  recordingMp4MaxHeight: number
  recordingMp4BitrateMbps: number
  recordingGifFps: number
  recordingWebpFps: number
}

export interface ScreenshotStatus {
  active: boolean
  lastPath: string | null
  lastSize: { width: number; height: number } | null
  ocrText: string | null
  pinned: boolean
  error: string | null
  recording: RecordingStatus
}

export interface RecordingStatus {
  phase: 'idle' | 'selecting' | 'recording' | 'processing' | 'preview' | 'exporting' | 'cancelled' | 'cancelling' | 'error'
  paused: boolean
  fps: number
  frameCount: number
  elapsedMs: number
  maxDurationMs: number
  message: string | null
  previewPath: string | null
  lastPath: string | null
}

const call = <T>(method: string, payload: Record<string, unknown> = {}) =>
  pluginCall<T>(method, payload)

export const getSettings = () => call<ScreenshotSettings>('get_settings')
export interface TencentCredentialsUpdate {
  secretId: string
  secretKey: string
  token: string
  clear: boolean
}

export const setSettings = (
  settings: ScreenshotSettings,
  tencentCredentials: TencentCredentialsUpdate,
) => call<void>('set_settings', {
  ...settings,
  tencentCredentials,
})
export const startCapture = (mode: 'region' | 'scroll' = 'region') =>
  call<void>('start_capture', { mode })
export const startRecording = (fps: number) => call<void>('start_recording', { fps })
export const stopRecording = () => call<void>('stop_recording')
export const cancelRecording = () => call<void>('cancel_recording')
export const exportRecording = (format: 'webp' | 'gif' | 'mp4') =>
  call<void>('export_recording', { format })
export const clearRecording = () => call<void>('clear_recording')
export const cancelCapture = () => call<void>('cancel_capture')
export const getStatus = () => call<ScreenshotStatus>('get_status')
export const pinLast = () => call<void>('pin_last')
export const clearOcr = () => call<void>('clear_ocr')
export const chooseSaveDirectory = () => call<string | null>('choose_save_directory')
export const openSaveDirectory = (path?: string) => call<void>('open_save_directory', { path })


