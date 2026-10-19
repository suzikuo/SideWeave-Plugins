import { pluginCall } from '../../../ui-sdk/runtime'

export type TunnelState = 'stopped' | 'starting' | 'running' | 'error'

export interface ServiceRule { id: string; name: string; hostname: string; service: string; enabled: boolean; disable_chunked_encoding?: boolean }
export interface ServiceView extends ServiceRule { full_url: string; port: number; local_online: boolean }

export interface TunnelRecord {
  id: string; name: string; tunnel_id: string; domain: string; created_at: string
  services: ServiceRule[]
}

export interface TunnelDetailView {
  tunnel: TunnelRecord; status: TunnelState; conns: number; services: ServiceView[]
  error_message?: string | null; config_path: string; credentials_path: string
}

export interface CredentialScanItem { tunnel_id: string; path: string }

export const cfGetTunnel = () => pluginCall<TunnelDetailView | null>('get_tunnel')
export const cfSetupTunnel = (name: string, domain: string, tunnel_id: string, credentials_path?: string, credentials_content?: string) =>
  pluginCall<void>('setup_tunnel', { name, domain, tunnel_id, credentials_path, credentials_content })
export const cfScanCredentials = () => pluginCall<CredentialScanItem[]>('scan_credentials')
export const cfSaveServices = (services: ServiceRule[]) => pluginCall<void>('save_services', { services })
export const cfToggleService = (service_id: string, enabled: boolean) => pluginCall<void>('toggle_service', { service_id, enabled })
export const cfStartTunnel = () => pluginCall<void>('start')
export const cfStopTunnel = () => pluginCall<void>('stop')
export const cfRestartTunnel = () => pluginCall<void>('restart')
export const cfGetLogs = (limit?: number) => pluginCall<string[]>('get_logs', { limit })
export const cfClearLogs = () => pluginCall<void>('clear_logs')
export const cfResetTunnel = () => pluginCall<void>('reset_tunnel')
export const cfOpenUrl = (url: string) => pluginCall<void>('open_url', { url })
export const cfCopyText = (text: string) => pluginCall<void>('copy_text', { text })
