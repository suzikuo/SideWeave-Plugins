import { pluginCall } from '../../../ui-sdk/runtime'

export type TunnelState = 'stopped' | 'starting' | 'running' | 'error'

export interface ServiceRule {
  id: string
  name: string
  hostname: string
  service: string
  enabled: boolean
}

export interface ServiceView {
  id: string
  name: string
  hostname: string
  full_url: string
  service: string
  port: number
  local_online: boolean
  enabled: boolean
}

export interface FileShareConfig {
  enabled: boolean
  directory: string
  hostname?: string
  token?: string
  port?: number
  lan_enabled?: boolean
  allowed_ips?: string[]
  upload_enabled?: boolean
  upload_directory?: string
  max_upload_mb?: number
  max_total_upload_mb?: number
}

export interface FileShareView {
  enabled: boolean
  directory: string
  local_url: string
  local_running: boolean
  port: number
  lan_enabled: boolean
  lan_running: boolean
  lan_host: string
  lan_share_url: string
  cloudflare_share_url: string
  upload_enabled: boolean
  upload_directory: string
  max_upload_mb: number
  max_total_upload_mb: number
  allowed_ips: string[]
  access_logs: FileShareAccessLog[]
}

export interface FileShareAccessLog {
  request_id: string
  timestamp: string
  client_ip: string
  method: string
  path: string
  protocol: string
  host: string
  status_code: number
  status_text: string
  request_content_length: number | null
  content_type: string
  user_agent: string
  referer: string
  response_content_type: string
  error: string
  expected_bytes: number | null
  received_bytes: number
  sent_bytes: number
  duration_ms: number
}

export interface TunnelRecord {
  id: string
  name: string
  tunnel_id: string
  domain: string
  created_at: string
  services: ServiceRule[]
  file_share?: FileShareConfig | null
}

export interface TunnelDetailView {
  tunnel: TunnelRecord
  status: TunnelState
  conns: number
  services: ServiceView[]
  file_share?: FileShareView | null
  error_message?: string | null
  config_path: string
  credentials_path: string
}

export interface CredentialScanItem {
  tunnel_id: string
  path: string
}

export const cfGetTunnel = () => pluginCall<TunnelDetailView | null>('get_tunnel')

export const cfSetupTunnel = (
  name: string,
  domain: string,
  tunnel_id: string,
  credentials_path?: string,
  credentials_content?: string
) =>
  pluginCall<void>('setup_tunnel', {
    name,
    domain,
    tunnel_id,
    credentials_path,
    credentials_content
  })

export const cfScanCredentials = () =>
  pluginCall<CredentialScanItem[]>('scan_credentials')

export const cfSaveServices = (services: ServiceRule[]) =>
  pluginCall<void>('save_services', { services })

export const cfSaveFileShare = (
  config: {
    directory: string
    hostname: string
    port: number
    lan_enabled: boolean
    allowed_ips: string
    upload_enabled: boolean
    upload_directory: string
    max_upload_mb: number
    max_total_upload_mb: number
  },
  rotate_token = false
) => pluginCall<void>('save_file_share', { ...config, rotate_token })

export const cfPickFileShareDirectory = (kind: 'share' | 'upload') =>
  pluginCall<string | null>('pick_file_share_directory', { kind })

export const cfStartFileShare = () => pluginCall<void>('start_file_share')
export const cfStopFileShare = () => pluginCall<void>('stop_file_share')

export const cfToggleService = (service_id: string, enabled: boolean) =>
  pluginCall<void>('toggle_service', { service_id, enabled })

export const cfStartTunnel = () => pluginCall<void>('start')
export const cfStopTunnel = () => pluginCall<void>('stop')
export const cfRestartTunnel = () => pluginCall<void>('restart')

export const cfGetLogs = (limit?: number) =>
  pluginCall<string[]>('get_logs', { limit })

export const cfClearLogs = () => pluginCall<void>('clear_logs')
export const cfResetTunnel = () => pluginCall<void>('reset_tunnel')
export const cfOpenUrl = (url: string) => pluginCall<void>('open_url', { url })
export const cfCopyText = (text: string) => pluginCall<void>('copy_text', { text })
