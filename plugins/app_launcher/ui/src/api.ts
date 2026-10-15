import { pluginCall } from '../../../ui-sdk/runtime'

export interface AppEntry {
  id: string
  name: string
  app_type: 'application' | 'chrome'
  exe_path: string
  arguments: string
  use_ahk: boolean
  ahk_path: string
  window_x: number
  window_y: number
  window_width: number
  window_height: number
  always_on_top: boolean
  in_preview: boolean
}

export type AppEntryInput = Omit<AppEntry, 'id'>

export const pickExecutable = () => pluginCall<string | null>('pick_executable')
export const alList = () => pluginCall<AppEntry[]>('list')
export const alAdd = (entry: AppEntryInput) => pluginCall<AppEntry>('add', { entry })
export const alUpdate = (id: string, entry: AppEntryInput) =>
  pluginCall<AppEntry>('update', { id, entry })
export const alRemove = (id: string) => pluginCall<void>('remove', { id })
export const alLaunch = (id: string) => pluginCall<void>('launch', { id })
export const alChromePreset = () => pluginCall<AppEntryInput | null>('chrome_preset')
export const alIcon = (path: string) => pluginCall<string | null>('icon', { path })
export const alTogglePreview = (id: string) =>
  pluginCall<{ id: string; in_preview: boolean }>('toggle_preview', { id })
