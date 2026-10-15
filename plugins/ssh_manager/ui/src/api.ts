import { pluginCall } from '../../../ui-sdk/runtime'

export type SshAuthKind = 'password' | 'key'

export interface SshProfile {
  id: string
  name: string
  host: string
  port: number
  user: string
  auth: SshAuthKind
  key_path: string
  tags: string[]
}

export interface SshProfileInput extends SshProfile {}

export interface SshKeyInfo {
  name: string
  path: string
  imported_at: string
  size: number
}

export const sshList = () => pluginCall<SshProfile[]>('list')
export const sshSave = (profile: SshProfileInput) => pluginCall<SshProfile>('save', { profile })
export const sshDelete = (id: string) => pluginCall<void>('delete', { id })
export const sshOpenTerminal = (id: string) => pluginCall<void>('open_terminal', { id })
export const sshTest = (id: string) => pluginCall<number>('test', { id })
export const sshImportKey = () => pluginCall<SshKeyInfo | null>('import_key')
export const sshListKeys = () => pluginCall<SshKeyInfo[]>('list_keys')
export const sshDeleteKey = (name: string) => pluginCall<void>('delete_key', { name })
export const sshResolveKey = (keyPath: string) => pluginCall<string | null>('resolve_key', { key_path: keyPath })

export const DEFAULT_GROUP = '默认'
export function groupOf(profile: SshProfile): string {
  return profile.tags.map((tag) => tag.trim()).find(Boolean) ?? DEFAULT_GROUP
}

function quoteIfNeeded(value: string): string {
  return /\s/.test(value) ? `"${value.replace(/"/g, '\\"')}"` : value
}

export function buildSshCommand(profile: Pick<SshProfile, 'port' | 'auth' | 'key_path' | 'user' | 'host'>): string {
  const parts = ['ssh', '-p', String(profile.port)]
  const key = profile.key_path.trim()
  if (profile.auth === 'key' && key) parts.push('-i', quoteIfNeeded(key))
  parts.push(quoteIfNeeded(`${profile.user}@${profile.host}`))
  return parts.join(' ')
}

export function parseTags(text: string): string[] {
  return [...new Set(text.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean))]
}
