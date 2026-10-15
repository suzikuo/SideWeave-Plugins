import { pluginCall } from '../../../ui-sdk/runtime'

/** 任务清单条目（完成任务保留，done_at 记录完成时间） */
export interface TodoItem {
  id: string
  text: string
  done: boolean
  done_at: number | null
}

export type NoteKind = 'text' | 'todo'

/** 便签皮肤（窗口表现形式） */
export const NOTE_SKINS = [
  { id: 'classic', label: '经典' },
  { id: 'lined', label: '横线' },
  { id: 'grid', label: '方格' },
  { id: 'minimal', label: '极简' },
  { id: 'terminal', label: '终端' },
  { id: 'neon', label: '霓虹' }
] as const

export type NoteSkin = (typeof NOTE_SKINS)[number]['id']

export interface Note {
  id: string
  kind: NoteKind
  /** 便签皮肤 id */
  skin: string
  /** 文本便签正文 */
  text: string
  /** 任务清单条目 */
  items: TodoItem[]
  color: string
  opacity: number
  pos_x: number | null
  pos_y: number | null
  /** 窗口尺寸记忆（逻辑像素） */
  width: number | null
  height: number | null
  visible: boolean
  /** 删除 → 进历史（保留可恢复） */
  deleted: boolean
  deleted_at: number | null
}

export const noteList = () => pluginCall<Note[]>('list')

export const noteCreate = (kind: NoteKind = 'text') => pluginCall<Note>('create', { kind })

/** 删除：空白便签直接真删除；有内容的移入历史（可恢复）。返回 true=已彻底删除 */
export const noteDelete = (id: string) => pluginCall<boolean>('delete', { id })

export const noteRestore = (id: string) => pluginCall<void>('restore', { id })

/** 彻底删除（仅历史中的便签，不可恢复） */
export const notePurge = (id: string) => pluginCall<void>('purge', { id })

export const noteClearHistory = () => pluginCall<void>('clear_history')

export const noteToggleVisible = (id: string) =>
  pluginCall<void>('toggle_visible', { id })

export const noteUpdateText = (id: string, text: string) =>
  pluginCall<void>('update_text', { id, text })

export const noteSetStyle = (id: string, color: string, opacity: number) =>
  pluginCall<void>('set_style', { id, color, opacity })

export const noteSetSkin = (id: string, skin: string) =>
  pluginCall<void>('set_skin', { id, skin })

export const noteGet = (id: string) => pluginCall<Note>('get', { id })

export const noteAddItem = (id: string, text: string) =>
  pluginCall<void>('add_item', { id, text })

export const noteSetItemDone = (id: string, itemId: string, done: boolean) =>
  pluginCall<void>('set_item_done', { id, item_id: itemId, done })

export const noteEditItem = (id: string, itemId: string, text: string) =>
  pluginCall<void>('edit_item', { id, item_id: itemId, text })

export const noteRemoveItem = (id: string, itemId: string) =>
  pluginCall<void>('remove_item', { id, item_id: itemId })
