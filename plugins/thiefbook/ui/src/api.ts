import { pluginCall } from '../../../ui-sdk/runtime'

export interface ThiefBook {
  id: string
  name: string
  path: string
  offset: number
  total_chars: number
  page_history?: number[]
}

export interface ThiefStyle {
  font_size: number
  opacity: number
  width: number
  lines: number
  color: string
  pinned: boolean
  hotkey_toggle: string
  hotkey_prev: string
  hotkey_next: string
  auto_page_secs: number
  hide_on_desktop: boolean
}

export interface ThiefConfig {
  books: ThiefBook[]
  style: ThiefStyle
  current_book_id?: string | null
  reader_position?: { x: number; y: number } | null
  reader_visible?: boolean
}

export interface ThiefReadResult {
  text: string
  total: number
  offset: number
}

export interface ThiefFindResult {
  offset: number | null
  total: number
}

export const thiefGetConfig = () => pluginCall<ThiefConfig>('get_config')
export const thiefPickBook = () => pluginCall<ThiefBook | null>('pick_book')
export const thiefRemoveBook = (id: string) => pluginCall<void>('remove_book', { id })
export const thiefRead = (bookId: string, offset: number, limit: number) => pluginCall<ThiefReadResult>('read', { book_id: bookId, offset, limit })
export const thiefFind = (bookId: string, query: string, fromOffset: number, direction: -1 | 1) => pluginCall<ThiefFindResult>('find', { book_id: bookId, query, from_offset: fromOffset, direction })
export const thiefSaveProgress = (bookId: string, offset: number, pageHistory: number[]) => pluginCall<void>('save_progress', { book_id: bookId, offset, page_history: pageHistory })
export const thiefJumpTo = (bookId: string, offset: number) => pluginCall<void>('jump_to', { book_id: bookId, offset })
export const thiefSaveReaderPosition = (x: number, y: number) => pluginCall<void>('save_reader_position', { x, y })
export const thiefRememberReaderHidden = (x: number, y: number) => pluginCall<void>('reader_hidden', { x, y })
export const thiefNotifyProgress = (bookId: string, offset: number) => pluginCall<void>('notify_progress', { book_id: bookId, offset })
export const thiefOpenBook = (bookId: string) => pluginCall<void>('open_book', { book_id: bookId })
export const thiefToggleReader = () => pluginCall<void>('toggle_reader')
export const thiefSaveStyle = (style: ThiefStyle) => pluginCall<void>('save_style', { style })
export const thiefSetPinned = (pinned: boolean) => pluginCall<void>('set_pinned', { pinned })
export const thiefRenameBook = (id: string, name: string) => pluginCall<ThiefBook>('rename_book', { id, name })

export const THIEF_EVENTS = {
  styleChanged: 'thief://style',
  openBook: 'thief://open',
  jump: 'thief://jump',
  progressChanged: 'thief://progress',
  page: 'thief://page'
} as const

export const STYLE_BOUNDS = {
  font_size: { min: 10, max: 32, step: 1 },
  opacity: { min: 0, max: 1, step: 0.05 },
  width: { min: 240, max: 1600, step: 10 },
  lines: { min: 1, max: 20, step: 1 },
  auto_page_secs: { min: 0, max: 120, step: 5 },
} as const

export const THIEF_STYLE_DEFAULT: ThiefStyle = {
  font_size: 14,
  opacity: 0.35,
  width: 420,
  lines: 3,
  color: '#e8e8ea',
  pinned: false,
  hotkey_toggle: '',
  hotkey_prev: '',
  hotkey_next: '',
  auto_page_secs: 0,
  hide_on_desktop: true
}

export const CURRENT_BOOK_KEY = 'thiefbook.current'
export function formatProgress(offset: number, total: number): string {
  if (!total || total <= 0) return '0%'
  return `${((Math.min(offset, total) / total) * 100).toFixed(1)}%`
}
