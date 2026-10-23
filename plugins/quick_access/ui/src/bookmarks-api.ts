/** 书签插件前端封装：通过统一 Plugin SDK 调用入口访问插件。 */

import { pluginCall } from '../../../ui-sdk/runtime'

/** 书签条目（Rust Bookmark，字段 snake_case 直传） */
export interface Bookmark {
  id: string
  name: string
  url: string
  tags: string[]
  pinned: boolean
  in_preview: boolean
  /** 创建时间（Unix 秒） */
  created_at: number
}

/** 新建/编辑入参（Rust BookmarkInput 同构） */
export interface BookmarkInput {
  name: string
  url: string
  tags: string[]
  pinned: boolean
  in_preview: boolean
}

// ---------- 命令 ----------

export const bmList = () => pluginCall<Bookmark[]>('bookmarks.list')

export const bmAdd = (input: BookmarkInput) => pluginCall<Bookmark>('bookmarks.add', { input })

export const bmUpdate = (id: string, bm: BookmarkInput) =>
  pluginCall<Bookmark>('bookmarks.update', { id, input: bm })

export const bmRemove = (id: string) => pluginCall<void>('bookmarks.remove', { id })

/** 用默认浏览器打开书签网址 */
export const bmOpen = (id: string) => pluginCall<void>('bookmarks.open', { id })

/** 切换书签是否加入悬浮预览展示 */
export const bmTogglePreview = (id: string) =>
  pluginCall<{ id: string; in_preview: boolean }>('bookmarks.toggle_preview', { id })

/** 列表视图样式：card 卡片（默认）/ compact 单条 / table 表格 */
export type BookmarkViewStyle = 'card' | 'compact' | 'table'

export const bmGetView = () => pluginCall<BookmarkViewStyle>('bookmarks.get_view')

export const bmSetView = (view: BookmarkViewStyle) =>
  pluginCall<void>('bookmarks.set_view', { view })

