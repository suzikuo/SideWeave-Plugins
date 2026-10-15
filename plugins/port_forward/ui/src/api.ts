import { pluginCall } from '../../../ui-sdk/runtime'

export interface GwRuleView {
  id: string
  name: string
  listen_port: number
  target_host: string
  target_port: number
  enabled: boolean
  running: boolean
  conns: number
}

export interface GwRuleInput {
  id: string
  name: string
  listen_port: number
  target_host: string
  target_port: number
  enabled: boolean
}

export interface ListenerInfo {
  protocol: 'TCP' | 'UDP'
  address: string
  port: number
  pid: number
  name: string
  path: string | null
  can_terminate: boolean
  blocked_reason: string | null
}

export const gwList = () => pluginCall<GwRuleView[]>('list')

export const gwSave = (rule: GwRuleInput) =>
  pluginCall<GwRuleView>('save', { rule })

export const gwDelete = (id: string) => pluginCall<void>('delete', { id })

export const gwSetRunning = (id: string, running: boolean) =>
  pluginCall<GwRuleView>('set_running', { id, running })

export const gwListeners = (port?: number) =>
  pluginCall<ListenerInfo[]>('listeners', port ? { port } : {})

export const gwStopListener = (listener: Pick<ListenerInfo, 'port' | 'pid' | 'path'>) =>
  pluginCall<void>('stop_listener', {
    listener: { port: listener.port, pid: listener.pid, path: listener.path ?? '' }
  })
