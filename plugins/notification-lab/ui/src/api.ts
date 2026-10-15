import { pluginCall } from '../../../ui-sdk/runtime'

export interface NotificationPayload {
  title: string
  body: string
  icon?: string
  sound?: string
}

export function sendTestNotification(payload: NotificationPayload) {
  return pluginCall<{ sent: boolean }>('notify', payload)
}

export type DesktopNotificationKind = 'info' | 'success' | 'warning' | 'alarm'

export interface DesktopNotificationPayload {
  title: string
  body: string
  kind: DesktopNotificationKind
  durationMs: number
}

export function sendHostDesktopNotification(payload: DesktopNotificationPayload) {
  return pluginCall<{ shown: boolean }>('show_desktop', payload)
}

export function sendHostSidebarNotification(payload: DesktopNotificationPayload) {
  return pluginCall<{ shown: boolean }>('show_sidebar', payload)
}
