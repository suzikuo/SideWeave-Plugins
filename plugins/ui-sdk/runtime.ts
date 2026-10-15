import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export interface PluginBridge {
  call<T>(method: string, payload?: unknown, timeoutMs?: number): Promise<T>
  onEvent<T>(topic: string, handler: (payload: T) => void): Promise<() => void>
}

declare global {
  interface Window {
    __agileTilesPluginId?: string
    agileTiles?: {
      apiVersion: number
      plugin: PluginBridge
    }
  }
}

function requestId(): string {
  return globalThis.crypto?.randomUUID?.() ?? 'ui-' + Date.now() + '-' + Math.random()
}

function currentPluginId(): string {
  if (window.__agileTilesPluginId) return window.__agileTilesPluginId
  const match = window.location.pathname.match(/\/plugins\/([^/]+)/)
  if (match) return decodeURIComponent(match[1])
  const isPluginResourceOrigin =
    window.location.protocol === 'agile-plugin:' ||
    ((window.location.protocol === 'http:' || window.location.protocol === 'https:') &&
      window.location.hostname === 'agile-plugin.localhost')
  if (isPluginResourceOrigin) {
    const installedMatch = window.location.pathname.match(/^\/([^/]+)/)
    if (installedMatch) return decodeURIComponent(installedMatch[1])
  }
  return 'unknown'
}

function clonePluginPayload(payload: unknown): unknown {
  if (payload === undefined || payload === null) return payload
  try {
    const serialized = JSON.stringify(payload)
    return serialized === undefined ? null : JSON.parse(serialized)
  } catch {
    throw new Error('插件请求参数不可序列化')
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === 'string' && error.trim()) return error
  if (error && typeof error === 'object') {
    const candidate = error as { message?: unknown; error?: unknown }
    if (typeof candidate.message === 'string' && candidate.message.trim()) return candidate.message
    if (typeof candidate.error === 'string' && candidate.error.trim()) return candidate.error
  }
  return '未知错误'
}

function postMessageSafe(target: Window, message: unknown): void {
  try {
    const serialized = JSON.stringify(message)
    target.postMessage(serialized === undefined ? null : JSON.parse(serialized), '*')
  } catch {
    throw new Error('插件消息不可序列化')
  }
}

interface PluginResponse<T> {
  request_id: string
  plugin_id: string
  ok: boolean
  result: T
  error?: { code: string; message: string } | null
}

interface PluginEventMessage<T = unknown> {
  __agileTiles?: boolean
  type?: string
  plugin_id?: string
  event?: { topic?: string; payload?: T }
}

interface PluginThemeMessage {
  __agileTiles?: boolean
  type?: string
  plugin_id?: string
  theme?: {
    variables?: Record<string, string>
    theme?: string
    skin?: string
    colorScheme?: string
  }
}

interface PluginLifecycleMessage {
  __agileTiles?: boolean
  type?: string
  plugin_id?: string
  active?: boolean
}

type PluginLifecycleHandler = (active: boolean) => void
const lifecycleHandlers = new Set<PluginLifecycleHandler>()
let hostActive = window.parent === window
let pluginActive = hostActive && !document.hidden

function setPluginActive(active: boolean): void {
  if (pluginActive === active) return
  pluginActive = active
  for (const handler of [...lifecycleHandlers]) handler(active)
}

window.addEventListener('message', (event: MessageEvent<PluginLifecycleMessage>) => {
  const data = event.data
  if (
    event.source !== window.parent ||
    data?.__agileTiles !== true ||
    data.type !== 'lifecycle' ||
    data.plugin_id !== currentPluginId() ||
    typeof data.active !== 'boolean'
  ) return
  hostActive = data.active
  setPluginActive(hostActive && !document.hidden)
})

document.addEventListener('visibilitychange', () => setPluginActive(hostActive && !document.hidden))

/** Subscribe to whether this plugin UI is the visible, active detail page. */
export function onPluginLifecycle(handler: PluginLifecycleHandler): () => void {
  lifecycleHandlers.add(handler)
  handler(pluginActive)
  return () => lifecycleHandlers.delete(handler)
}

function applyHostTheme(theme: PluginThemeMessage['theme']): void {
  if (!theme) return
  for (const [name, value] of Object.entries(theme.variables ?? {})) {
    if (name.startsWith('--')) document.documentElement.style.setProperty(name, value)
  }
  if (theme.theme) document.documentElement.dataset.theme = theme.theme
  if (theme.skin) document.documentElement.dataset.skin = theme.skin
  if (theme.colorScheme) document.documentElement.style.colorScheme = theme.colorScheme
}

if (window.parent !== window) document.documentElement.dataset.agileTilesEmbedded = 'true'
if (window.parent === window) document.body.style.background = 'transparent'

function onEmbeddedEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape' && event.key !== 'Esc') return
  if (event.defaultPrevented || window.parent === window) return
  postMessageSafe(window.parent,
    {
      __agileTiles: true,
      type: 'close-detail',
      plugin_id: currentPluginId()
    }
  )
}

if (window.parent !== window) window.addEventListener('keydown', onEmbeddedEscape)

window.addEventListener('message', (event: MessageEvent<PluginThemeMessage>) => {
  const data = event.data
  if (
    event.source !== window.parent ||
    data?.__agileTiles !== true ||
    data.type !== 'theme' ||
    data.plugin_id !== currentPluginId()
  ) return
  applyHostTheme(data.theme)
})

export function pluginCall<T>(method: string, payload: unknown = {}, timeoutMs = 8_000): Promise<T> {
  const safePayload = clonePluginPayload(payload)
  const bridge = window.agileTiles?.plugin
  if (bridge) return bridge.call<T>(method, safePayload, timeoutMs)

  if (window.parent === window) {
    const request = {
      request_id: requestId(),
      plugin_id: currentPluginId(),
      kind: 'call',
      method,
      payload: safePayload,
      timeout_ms: timeoutMs
    }
    return invoke<PluginResponse<T>>('plugin_call', {
      request
    }).then((response) => {
      if (!response.ok) {
        throw new Error(response.error?.message ?? `插件调用失败: ${method}`)
      }
      return response.result
    })
  }

  return new Promise<T>((resolve, reject) => {
    const id = requestId()
    const pluginId = currentPluginId()
    const timer = window.setTimeout(() => {
      window.removeEventListener('message', onMessage)
      reject(new Error('Agile Tiles UI bridge timeout'))
    }, 8000)
    function onMessage(event: MessageEvent) {
      const data = event.data as {
        __agileTiles?: boolean
        id?: string
        plugin_id?: string
        result?: T
        error?: string
      }
      if (
        event.source !== window.parent ||
        data?.__agileTiles !== true ||
        data.plugin_id !== pluginId ||
        data.id !== id
      ) return
      window.clearTimeout(timer)
      window.removeEventListener('message', onMessage)
      if (data.error) reject(new Error(data.error))
      else resolve(data.result as T)
    }
    window.addEventListener('message', onMessage)
    postMessageSafe(window.parent,
      {
        __agileTiles: true,
        id,
        plugin_id: pluginId,
        action: 'call',
        payload: { method, payload: safePayload, timeout_ms: timeoutMs }
      }
    )
  })
}

export function pluginOnEvent<T>(topic: string, handler: (payload: T) => void): Promise<() => void> {
  const pluginId = currentPluginId()
  if (window.parent !== window) {
    const onMessage = (event: MessageEvent<PluginEventMessage<T>>) => {
      const data = event.data
      if (
        event.source !== window.parent ||
        data?.__agileTiles !== true ||
        data.type !== 'event' ||
        data.plugin_id !== pluginId ||
        data.event?.topic !== topic
      ) return
      if (!pluginActive) return
      handler(data.event.payload as T)
    }
    window.addEventListener('message', onMessage)
    return Promise.resolve(() => window.removeEventListener('message', onMessage))
  }
  return listen<{ plugin_id: string; topic: string; payload?: T }>('plugin://event', (event) => {
    if (
      pluginActive &&
      event.payload.plugin_id === pluginId &&
      event.payload.topic === topic
    ) {
      handler(event.payload.payload as T)
    }
  })
}

export { getCurrentWindow } from '@tauri-apps/api/window'
export { listen }
export type { UnlistenFn } from '@tauri-apps/api/event'

export {}
