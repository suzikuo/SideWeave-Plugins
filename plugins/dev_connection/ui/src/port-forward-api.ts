import { pluginCall } from '../../../ui-sdk/runtime'

export interface GwRuleView {
  id: string
  name: string
  listen_port: number
  target_host: string
  target_port: number
  enabled: boolean
  in_preview: boolean
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
  in_preview: boolean
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

export interface ConnectionLogView {
  id: number
  started_at_ms: number
  elapsed_ms: number
  finished: boolean
  rule_name: string
  source_addr: string
  listen_addr: string
  target_endpoint: string
  actual_target_addr: string | null
  protocol: 'HTTP' | 'TLS' | 'TCP' | string
  method: string | null
  host: string | null
  path: string | null
  request_url: string | null
  bytes_from_client: number
  bytes_from_target: number
  status: string
  error: string | null
}

export const gwList = () => pluginCall<GwRuleView[]>('ports.list')

export const gwSave = (rule: GwRuleInput) =>
  pluginCall<GwRuleView>('ports.save', { rule })

export const gwDelete = (id: string) => pluginCall<void>('ports.delete', { id })

export const gwSetRunning = (id: string, running: boolean) =>
  pluginCall<GwRuleView>('ports.set_running', { id, running })

export const gwListeners = (port?: number) =>
  pluginCall<ListenerInfo[]>('ports.listeners', port ? { port } : {})

export const gwStopListener = (listener: Pick<ListenerInfo, 'port' | 'pid' | 'path'>) =>
  pluginCall<void>('ports.stop_listener', {
    listener: { port: listener.port, pid: listener.pid, path: listener.path ?? '' }
  })

export const gwConnectionLogs = () => pluginCall<ConnectionLogView[]>('ports.connection_logs')

export const gwClearConnectionLogs = () => pluginCall<void>('ports.clear_connection_logs')

