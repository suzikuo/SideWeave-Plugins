import { pluginCall } from '../../../ui-sdk/runtime'

export type TunnelState = 'stopped' | 'starting' | 'running' | 'error'

export interface ServiceRule { id: string; name: string; hostname: string; service: string; enabled: boolean; disable_chunked_encoding?: boolean; in_preview: boolean }
export interface ServiceView extends ServiceRule { full_url: string; port: number; local_online: boolean }

export interface TunnelRecord {
  id: string; name: string; tunnel_id: string; domain: string; created_at: string
  services: ServiceRule[]
  preview_status: boolean
}

export interface TunnelDetailView {
  tunnel: TunnelRecord; status: TunnelState; conns: number; services: ServiceView[]
  error_message?: string | null; config_path: string; credentials_path: string
}

export interface CredentialScanItem { tunnel_id: string; path: string }

export const cfGetTunnel = () => pluginCall<TunnelDetailView | null>('tunnel.get_tunnel')
export const cfSetupTunnel = (name: string, domain: string, tunnel_id: string, credentials_path?: string, credentials_content?: string) =>
  pluginCall<void>('tunnel.setup_tunnel', { name, domain, tunnel_id, credentials_path, credentials_content })
export const cfScanCredentials = () => pluginCall<CredentialScanItem[]>('tunnel.scan_credentials')
export const cfSaveServices = (services: ServiceRule[]) => pluginCall<void>('tunnel.save_services', { services })
export const cfSetPreviewStatus = (enabled: boolean) => pluginCall<void>('tunnel.set_preview_status', { enabled })
export const cfToggleService = (service_id: string, enabled: boolean) => pluginCall<void>('tunnel.toggle_service', { service_id, enabled })
export const cfStartTunnel = () => pluginCall<void>('tunnel.start')
export const cfStopTunnel = () => pluginCall<void>('tunnel.stop')
export const cfRestartTunnel = () => pluginCall<void>('tunnel.restart')
export const cfGetLogs = (limit?: number) => pluginCall<string[]>('tunnel.get_logs', { limit })
export const cfClearLogs = () => pluginCall<void>('tunnel.clear_logs')
export const cfResetTunnel = () => pluginCall<void>('tunnel.reset_tunnel')
export const cfOpenUrl = (url: string) => pluginCall<void>('tunnel.open_url', { url })
export const cfCopyText = (text: string) => pluginCall<void>('tunnel.copy_text', { text })

