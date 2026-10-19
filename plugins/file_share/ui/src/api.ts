import { pluginCall } from '../../../ui-sdk/runtime'

export interface FileShareConfig {
  id: string
  name: string
  enabled: boolean
  view_directory: string
  receive_directory: string
  token?: string
  port: number
  tunnel_hostname: string
  lan_enabled: boolean
  allowed_ips: string[]
  allow_view: boolean
  allow_upload: boolean
  require_login: boolean
  media_preview_limit_kbps: number
  max_upload_mb: number
  max_total_upload_mb: number
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

export interface FileShareView extends Omit<FileShareConfig, 'token'> {
  token: string
  local_url: string
  local_running: boolean
  cloudflare_url: string
  lan_running: boolean
  lan_host: string
  lan_share_url: string
  access_logs: FileShareAccessLog[]
}

export interface FileShareDetail {
  file_shares: FileShareView[]
  users: string[]
}

export const getFileShares = () => pluginCall<FileShareDetail>('get_file_shares')
export const pickDirectory = (kind: 'view' | 'receive') => pluginCall<string | null>('pick_directory', { kind })
export const suggestPort = () => pluginCall<number>('suggest_port')
export const saveFileShare = (config: FileShareConfig) => pluginCall<{ id: string }>('save_file_share', { config })
export const deleteFileShare = (id: string) => pluginCall<void>('delete_file_share', { id })
export const startFileShare = (id: string) => pluginCall<void>('start_file_share', { id })
export const stopFileShare = (id: string) => pluginCall<void>('stop_file_share', { id })
export const rotateToken = (id: string) => pluginCall<{ id: string; token: string }>('rotate_file_share_token', { id })
export const clearAccessLogs = (id: string) => pluginCall<void>('clear_access_logs', { id })
export const addUser = (username: string, password: string) => pluginCall<void>('add_user', { username, password })
export const setUserPassword = (username: string, password: string) => pluginCall<void>('set_user_password', { username, password })
export const deleteUser = (username: string) => pluginCall<void>('delete_user', { username })
export const openUrl = (url: string) => pluginCall<void>('open_url', { url })
export const copyText = (text: string) => pluginCall<void>('copy_text', { text })
