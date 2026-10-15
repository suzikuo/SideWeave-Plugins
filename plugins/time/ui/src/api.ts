import { pluginCall } from '../../../ui-sdk/runtime'

/** 闹钟规则。预设只创建这些通用规则的初始值，之后仍可编辑。 */
export type AlarmRule =
  | { type: 'time'; hour: number; minute: number; weekdays: number[] }
  | { type: 'interval'; interval_minutes: number; next_fire_at_secs: number; message: string }

/** 闹钟持久化结构（Rust Alarm，字段 snake_case 直传） */
export interface Alarm {
  id: string
  label: string
  rule: AlarmRule
  enabled: boolean
}

/** 新增/编辑入参（id 由服务端生成/保留） */
export interface AlarmInput {
  label: string
  rule: AlarmRule
  enabled: boolean
}

/** 下一个将触发的闹钟 */
export interface NextAlarm {
  alarm_id: string
  /** 本地时区 Unix 时间戳（秒） */
  at_secs: number
}

/** 时钟快照 */
export interface TimeSnapshot {
  /** 本地时间 RFC3339（带时区偏移） */
  now_local: string
  hour: number
  minute: number
  second: number
  /** 0=周日..6=周六 */
  weekday: number
  alarms: Alarm[]
  next: NextAlarm | null
  notification_method: NotificationMethod
  notification_duration_secs: number
  alarm_clock_flash_enabled: boolean
}

export type NotificationMethod = 'native' | 'desktop' | 'sidebar'

// ---------- 命令 ----------

export const timeListAlarms = () => pluginCall<Alarm[]>('list_alarms')

export const timeAddAlarm = (alarm: AlarmInput) => pluginCall<Alarm>('add_alarm', { alarm })

export const timeUpdateAlarm = (id: string, alarm: AlarmInput) =>
  pluginCall<Alarm>('update_alarm', { id, alarm })

export const timeRemoveAlarm = (id: string) => pluginCall<void>('remove_alarm', { id })

export const timeSnapshot = () => pluginCall<TimeSnapshot>('snapshot')

export const timeSaveNotificationMethod = (method: NotificationMethod) =>
  pluginCall<NotificationMethod>('save_notification_method', { method })

export const timeSaveNotificationDuration = (duration_secs: number) =>
  pluginCall<number>('save_notification_duration', { duration_secs })

export const timeSetAlarmClockFlash = (enabled: boolean) =>
  pluginCall<boolean>('set_alarm_clock_flash', { enabled })

/** 悬浮钟显示/隐藏切换（已可见 → 隐藏；隐藏/不存在 → 显示并复位点击穿透） */
export const timeToggleClock = () => pluginCall<void>('toggle_clock')

/** 悬浮钟固定开关：锁定位置并启用原生鼠标穿透；取消固定需从详情页恢复交互。 */
export const timeSetPinned = (pinned: boolean) =>
  pluginCall<void>('set_pinned', { enabled: pinned })

// ---------- 事件 ----------

/** 事件名（Rust → 前端） */
export const TIME_EVENTS = {
  alarm: 'time://alarm',
  snapshot: 'time://clock-snapshot',
  visibility: 'time://clock-visibility',
  /** 悬浮钟固定状态（payload: bool，时钟窗口据此同步锁定样式） */
  clickthrough: 'time://clickthrough'
} as const

export interface AlarmFiredPayload {
  id: string
  label: string
  flash?: boolean
}

// ---------- 展示辅助 ----------

/** 倒计时强调阈值：5 分钟（秒） */
export const COUNTDOWN_THRESHOLD_SECS = 5 * 60

/** 0=周日..6=周六 */
export const WEEKDAY_LABELS = ['周日', '周一', '周二', '周三', '周四', '周五', '周六']

/** 周几 chip 的展示顺序：周一..周日 */
export const WEEKDAY_CHIP_ORDER = [1, 2, 3, 4, 5, 6, 0]

/** 重复规则文案 */
export function formatWeekdays(days: number[]): string {
  if (!days.length) return '仅一次'
  if (days.length === 7) return '每天'
  const sorted = [...days].sort((a, b) => a - b)
  if (sorted.join(',') === '1,2,3,4,5') return '工作日'
  if (sorted.join(',') === '0,6') return '周末'
  return sorted.map((d) => WEEKDAY_LABELS[d] ?? '').join(' ')
}

export function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

export function formatAlarmTime(a: { hour: number; minute: number }): string {
  return `${pad2(a.hour)}:${pad2(a.minute)}`
}

/** 剩余秒数 → "1天 2小时" / "2小时 3分" / "3分 20秒" */
export function formatRemaining(secs: number): string {
  const s = Math.max(0, Math.floor(secs))
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  if (d > 0) return `${d}天 ${h}小时`
  if (h > 0) return `${h}小时 ${m}分`
  return `${m}分 ${sec}秒`
}

/** 剩余秒数 → "MM:SS"（≤5 分钟倒计时态） */
export function formatCountdown(secs: number): string {
  const s = Math.max(0, Math.floor(secs))
  return `${pad2(Math.floor(s / 60))}:${pad2(s % 60)}`
}

// ---------- 悬浮钟样式 ----------

export interface ClockStyle {
  background_opacity: number
  content_opacity: number
  scale: number
  color: string // "accent" | "#hex"
  pos_x?: number | null
  pos_y?: number | null
  /** 悬浮钟是否显示中（Rust 维护，重启自动恢复；前端保存样式时无需关心） */
  clock_visible?: boolean
  /** 悬浮钟皮肤：mini / pomodoro */
  skin: 'mini' | 'pomodoro'
  /** 极简时钟显示格式：hm / hms */
  mini_time_format: 'hm' | 'hms'
  /** 极简时钟排列：horizontal / vertical */
  mini_layout: 'horizontal' | 'vertical'
  /** 悬浮钟数字字重：400 / 500 / 700 */
  font_weight: 400 | 500 | 700
  /** 番茄钟专注时长（分钟） */
  pomo_work_min: number
  /** 番茄钟休息时长（分钟） */
  pomo_break_min: number
  /** 悬浮窗是否锁定点击与拖拽，重启后恢复。 */
  pinned: boolean
}

function isClockStyle(value: unknown): value is ClockStyle {
  if (!value || typeof value !== 'object') return false
  const style = value as Partial<ClockStyle>
  const backgroundOpacity = style.background_opacity
  return (
    typeof backgroundOpacity === 'number' &&
    Number.isFinite(backgroundOpacity) &&
    (style.content_opacity === undefined ||
      (typeof style.content_opacity === 'number' && Number.isFinite(style.content_opacity))) &&
    typeof style.scale === 'number' &&
    Number.isFinite(style.scale) &&
    typeof style.color === 'string' &&
    typeof style.skin === 'string' &&
    typeof style.pomo_work_min === 'number' &&
    typeof style.pomo_break_min === 'number' &&
    typeof style.pinned === 'boolean'
  )
}

export function getInitialTimeClockStyle(): ClockStyle | null {
  try {
    const encoded = new URLSearchParams(window.location.search).get('style')
    if (!encoded) return null
    const value = JSON.parse(encoded)
    if (!isClockStyle(value)) return null
    const style = value as ClockStyle
    const fontWeight = (value as { font_weight?: number }).font_weight
    return {
      ...style,
      background_opacity: style.background_opacity,
      content_opacity: style.content_opacity ?? 1,
      skin: style.skin === 'pomodoro' ? 'pomodoro' : 'mini',
      mini_time_format: style.mini_time_format === 'hms' ? 'hms' : 'hm',
      mini_layout: style.mini_layout === 'vertical' ? 'vertical' : 'horizontal',
      font_weight: fontWeight === 400 ? 400 : fontWeight === 600 || fontWeight === 700 ? 700 : 500
    }
  } catch {
    return null
  }
}

export async function getTimeClockStyle() {
  return pluginCall<ClockStyle>('get_clock_style')
}

export async function saveTimeClockStyle(style: ClockStyle) {
  await pluginCall<void>('save_clock_style', { style })
}

export const CLOCK_STYLE_EVENTS = { styleChanged: 'time://clock-style' } as const

/** 收紧/扩容悬浮钟窗口（逻辑像素）。不传 pos：窗口中心保持不动，且钳制在所在显示器内；
 * 传 pos（物理像素 [x, y]）：改为绝对定位并记忆位置——用于右键菜单关闭后精确还原。 */
/** Send a host-managed desktop notification, for example after a Pomodoro phase change. */
export const timeNotify = (title: string, body: string) =>
  pluginCall<void>('notify', { title, body })
