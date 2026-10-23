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

export const pickExecutable = () => pluginCall<string | null>('apps.pick_executable')
export const alList = () => pluginCall<AppEntry[]>('apps.list')
export const alAdd = (entry: AppEntryInput) => pluginCall<AppEntry>('apps.add', { entry })
export const alUpdate = (id: string, entry: AppEntryInput) =>
  pluginCall<AppEntry>('apps.update', { id, entry })
export const alRemove = (id: string) => pluginCall<void>('apps.remove', { id })
export const alLaunch = (id: string) => pluginCall<void>('apps.launch', { id })
export const alChromePreset = () => pluginCall<AppEntryInput | null>('apps.chrome_preset')
export const alIcon = (path: string) => pluginCall<string | null>('apps.icon', { path })
export const alTogglePreview = (id: string) =>
  pluginCall<{ id: string; in_preview: boolean }>('apps.toggle_preview', { id })

