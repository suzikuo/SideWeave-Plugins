/** 剪贴板历史插件前端封装（通过统一 Plugin SDK 调用入口） */

import { pluginCall } from '../../../ui-sdk/runtime'

export interface ClipItem {
  id: string
  text: string
  /** 记录时间（本地 RFC3339） */
  ts: string
  /** 收藏置顶：不会被容量淘汰 */
  pinned: boolean
}

export interface ClipboardSettings {
  enabled: boolean
  max: number
}

const call = <T>(method: string, payload: Record<string, unknown> = {}) =>
  pluginCall<T>(method, payload)

export const clipboardList = () => call<ClipItem[]>('list')

export const clipboardGetSettings = () => call<ClipboardSettings>('get_settings')

export const clipboardSetEnabled = (enabled: boolean) =>
  call<void>('set_enabled', { enabled })

export const clipboardSetMax = (max: number) => call<void>('set_max', { max })

/** 复制回剪贴板，并移动到历史顶部 */
export const clipboardCopyItem = (id: string) => call<void>('copy_item', { id })

export const clipboardDeleteItem = (id: string) => call<void>('delete_item', { id })

export const clipboardClear = () => call<void>('clear')

/** 收藏/取消收藏：收藏项置顶且不会被容量淘汰 */
export const clipboardPinToggle = (id: string) => call<void>('pin_toggle', { id })
