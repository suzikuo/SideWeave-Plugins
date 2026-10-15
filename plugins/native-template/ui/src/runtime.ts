export interface PluginBridge {
  call<T>(method: string, payload?: unknown, timeoutMs?: number): Promise<T>
  onEvent<T>(topic: string, handler: (payload: T) => void): Promise<() => void>
}

declare global {
  interface Window {
    agileTiles?: {
      apiVersion: number
      plugin: PluginBridge
    }
  }
}

export function pluginCall<T>(method: string, payload: unknown = {}, timeoutMs = 8_000): Promise<T> {
  const bridge = window.agileTiles?.plugin
  if (!bridge) {
    return Promise.reject(new Error('SideWeave plugin bridge is unavailable'))
  }
  return bridge.call<T>(method, payload, timeoutMs)
}

export function pluginOnEvent<T>(topic: string, handler: (payload: T) => void): Promise<() => void> {
  const bridge = window.agileTiles?.plugin
  if (!bridge) {
    return Promise.reject(new Error('SideWeave plugin bridge is unavailable'))
  }
  return bridge.onEvent(topic, handler)
}

export {}
