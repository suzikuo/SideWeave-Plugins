<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** Time plugin details: alarm management, floating clock controls, and next-alarm countdown. */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen, onPluginLifecycle, type UnlistenFn } from '../../../ui-sdk/runtime'
import Icon from '../../../ui-sdk/Icon.vue'
import Toast from '../../../ui-sdk/Toast.vue'
import RangeSlider from '../../../ui-sdk/RangeSlider.vue'
import {
  COUNTDOWN_THRESHOLD_SECS,
  WEEKDAY_CHIP_ORDER,
  WEEKDAY_LABELS,
  formatAlarmTime,
  formatCountdown,
  formatRemaining,
  formatWeekdays,
  timeAddAlarm,
  getTimeClockStyle,
  timeSaveNotificationDuration,
  timeSaveNotificationMethod,
  timeRemoveAlarm,
  timeSetAlarmClockFlash,
  saveTimeClockStyle,
  timeSetPinned,
  timeSnapshot,
  timeToggleClock,
  timeUpdateAlarm
} from './api'
import type { Alarm, AlarmInput, AlarmRule, ClockStyle, NextAlarm, NotificationMethod } from './api'

function toLocalDateTimeInput(date: Date): string {
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return local.toISOString().slice(0, 16)
}

function intervalStartFallback(minutes = 30): string {
  const startAtMinute = Math.ceil((Date.now() + minutes * 60_000) / 60_000)
  const date = new Date(startAtMinute * 60_000)
  return toLocalDateTimeInput(date)
}

function nextHourFallback(): string {
  const next = new Date()
  next.setMinutes(0, 0, 0)
  next.setHours(next.getHours() + 1)
  return toLocalDateTimeInput(next)
}

interface AlarmForm {
  label: string
  ruleType: 'time' | 'interval'
  time: string
  weekdays: number[]
  intervalMinutes: number
  nextFireAt: string
  message: string
  enabled: boolean
}

// ---------- 状态 ----------

const alarms = ref<Alarm[]>([])
const next = ref<NextAlarm | null>(null)
const now = ref(new Date())
const notificationMethod = ref<NotificationMethod>('sidebar')
const notificationDurationSecs = ref(12)
const alarmClockFlashEnabled = ref(true)

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
const firedLabel = ref('')

const showDialog = ref(false)
const editingId = ref<string | null>(null)
const form = ref<AlarmForm>(defaultForm())

let ticker = 0
let syncTimer = 0
let flashTimer = 0
let unlistenEvent: UnlistenFn | null = null
let unlistenLifecycle: (() => void) | null = null
let pluginActive = false
let pendingAlarmLabel: string | null = null

function defaultForm(ruleType: AlarmForm['ruleType'] = 'time'): AlarmForm {
  return {
    label: '',
    ruleType,
    time: '08:00',
    weekdays: [] as number[],
    intervalMinutes: 30,
    nextFireAt: intervalStartFallback(30),
    message: '时间到了，休息一下吧。',
    enabled: true
  }
}

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

function flashAlarm(label: string) {
  firedLabel.value = label || '闹钟'
  window.clearTimeout(flashTimer)
  flashTimer = window.setTimeout(() => { firedLabel.value = '' }, 8000)
}

async function refresh() {
  try {
    const snap = await timeSnapshot()
    alarms.value = snap.alarms
    next.value = snap.next
    notificationMethod.value = snap.notification_method ?? 'sidebar'
    notificationDurationSecs.value = snap.notification_duration_secs ?? 12
    alarmClockFlashEnabled.value = snap.alarm_clock_flash_enabled ?? true
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function changeNotificationMethod(method: NotificationMethod) {
  try {
    await timeSaveNotificationMethod(method)
    notificationMethod.value = method
    const labels: Record<NotificationMethod, string> = {
      native: 'Windows 原生通知',
      desktop: '宿主桌面卡片',
      sidebar: '侧边栏气泡'
    }
    showToast(`通知方式已切换为${labels[method]}`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function changeNotificationDuration(durationSecs: number) {
  if (!Number.isInteger(durationSecs) || durationSecs < 3 || durationSecs > 60) return
  try {
    notificationDurationSecs.value = await timeSaveNotificationDuration(durationSecs)
    showToast(`宿主通知将显示 ${durationSecs} 秒`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function setAlarmClockFlash(enabled: boolean) {
  try {
    alarmClockFlashEnabled.value = await timeSetAlarmClockFlash(enabled)
    showToast(enabled ? '已开启闹钟悬浮窗闪烁' : '已关闭闹钟悬浮窗闪烁', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function setPluginActive(active: boolean) {
  pluginActive = active
  window.clearInterval(ticker)
  window.clearInterval(syncTimer)
  ticker = 0
  syncTimer = 0
  if (!active) return

  now.value = new Date()
  void refresh()
  void loadClockStyle()
  ticker = window.setInterval(() => {
    now.value = new Date()
  }, 1000)
  syncTimer = window.setInterval(refresh, 30_000)
  if (pendingAlarmLabel) {
    flashAlarm(pendingAlarmLabel)
    pendingAlarmLabel = null
  }
}

// ---------- 时钟与倒计时 ----------

const nextAlarm = computed(() =>
  next.value ? alarms.value.find((a) => a.id === next.value?.alarm_id) : undefined
)
const nextAlarmTimeLabel = computed(() => nextAlarm.value ? alarmTimeLabel(nextAlarm.value) : '--:--')

/** 距下个闹钟剩余秒数（本地走秒实时计算，无需每秒请求后端） */
const remaining = computed(() => {
  if (!next.value) return null
  return Math.max(0, Math.round(next.value.at_secs - now.value.getTime() / 1000))
})

/** ≤5 分钟进入醒目倒计时态 */
const imminent = computed(
  () => remaining.value !== null && remaining.value <= COUNTDOWN_THRESHOLD_SECS
)
const countdownText = computed(() =>
  remaining.value === null ? '' : formatCountdown(remaining.value)
)
const upcomingText = computed(() =>
  remaining.value === null ? '' : formatRemaining(remaining.value)
)

// ---------- 闹钟 CRUD ----------

const sortedAlarms = computed(() =>
  [...alarms.value].sort((a, b) => alarmSortKey(a) - alarmSortKey(b))
)

function alarmSortKey(alarm: Alarm): number {
  if (!alarm.enabled) return Number.MAX_SAFE_INTEGER
  if (alarm.rule.type === 'interval') return alarm.rule.next_fire_at_secs
  const current = new Date()
  const base = new Date(current)
  base.setSeconds(0, 0)
  base.setHours(alarm.rule.hour, alarm.rule.minute, 0, 0)
  for (let offset = 0; offset <= 7; offset += 1) {
    const candidate = new Date(base)
    candidate.setDate(base.getDate() + offset)
    const weekdays = alarm.rule.weekdays
    if ((weekdays.length === 0 || weekdays.includes(candidate.getDay())) && candidate >= current) {
      return Math.floor(candidate.getTime() / 1000)
    }
  }
  return Number.MAX_SAFE_INTEGER
}

function alarmTimeLabel(alarm: Alarm): string {
  if (alarm.rule.type === 'time') return formatAlarmTime(alarm.rule)
  const at = new Date(alarm.rule.next_fire_at_secs * 1000)
  return formatAlarmTime({ hour: at.getHours(), minute: at.getMinutes() })
}

function alarmRuleLabel(alarm: Alarm): string {
  if (alarm.rule.type === 'time') return formatWeekdays(alarm.rule.weekdays)
  const minutes = alarm.rule.interval_minutes
  if (minutes < 60) return `每 ${minutes} 分钟`
  if (minutes < 1440 && minutes % 60 === 0) return `每 ${minutes / 60} 小时`
  if (minutes === 1440) return '每 24 小时'
  return `每 ${minutes} 分钟`
}

function openAdd(preset: 'hourly' | 'interval' | 'time' = 'time') {
  editingId.value = null
  const nextForm = defaultForm(preset === 'time' ? 'time' : 'interval')
  if (preset === 'hourly') {
    nextForm.label = '整点报时'
    nextForm.intervalMinutes = 60
    nextForm.nextFireAt = nextHourFallback()
    nextForm.message = '现在是 {time}。'
  } else if (preset === 'interval') {
    nextForm.label = '间隔提醒'
  }
  form.value = nextForm
  showDialog.value = true
}

function openEdit(a: Alarm) {
  editingId.value = a.id
  const nextForm = defaultForm(a.rule.type)
  nextForm.label = a.label
  nextForm.enabled = a.enabled
  if (a.rule.type === 'time') {
    nextForm.time = formatAlarmTime(a.rule)
    nextForm.weekdays = [...a.rule.weekdays]
  } else {
    nextForm.intervalMinutes = a.rule.interval_minutes
    nextForm.nextFireAt = a.rule.next_fire_at_secs > Math.floor(Date.now() / 1000)
      ? toLocalDateTimeInput(new Date(a.rule.next_fire_at_secs * 1000))
      : intervalStartFallback(a.rule.interval_minutes)
    nextForm.message = a.rule.message
  }
  form.value = nextForm
  showDialog.value = true
}

function setRepeatPreset(preset: 'once' | 'daily' | 'weekdays' | 'weekend') {
  const values: Record<typeof preset, number[]> = {
    once: [],
    daily: [0, 1, 2, 3, 4, 5, 6],
    weekdays: [1, 2, 3, 4, 5],
    weekend: [0, 6]
  }
  form.value.weekdays = [...values[preset]]
}

function toggleWeekday(d: number) {
  const list = form.value.weekdays
  const i = list.indexOf(d)
  if (i >= 0) list.splice(i, 1)
  else list.push(d)
}

async function save() {
  let rule: AlarmRule
  if (form.value.ruleType === 'time') {
    const [h, m] = form.value.time.split(':').map(Number)
    if (!Number.isFinite(h) || !Number.isFinite(m) || h! > 23 || m! > 59) {
      showToast('请选择有效的闹钟时间', 'error')
      return
    }
    rule = {
      type: 'time',
      hour: h!,
      minute: m!,
      weekdays: [...new Set(form.value.weekdays)].sort((a, b) => a - b)
    }
  } else {
    const minutes = Math.floor(Number(form.value.intervalMinutes))
    if (!Number.isFinite(minutes) || minutes < 1 || minutes > 10_080) {
      showToast('间隔需设置为 1 到 10080 分钟（7 天）', 'error')
      return
    }
    const nextFireAt = new Date(form.value.nextFireAt).getTime()
    if (!Number.isFinite(nextFireAt) || (form.value.enabled && nextFireAt <= Date.now())) {
      showToast('启用规则时，请选择一个未来的首次提醒时间', 'error')
      return
    }
    rule = {
      type: 'interval',
      interval_minutes: minutes,
      next_fire_at_secs: Math.floor(nextFireAt / 1000),
      message: form.value.message.trim() || '时间到了，休息一下吧。'
    }
  }
  const input: AlarmInput = {
    label: form.value.label.trim() || (rule.type === 'interval' ? '规则闹钟' : '闹钟'),
    rule,
    enabled: form.value.enabled
  }
  try {
    if (editingId.value) {
      await timeUpdateAlarm(editingId.value, input)
      showToast('已更新', 'success')
    } else {
      await timeAddAlarm(input)
      showToast('已添加', 'success')
    }
    showDialog.value = false
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function toggleEnabled(a: Alarm) {
  const rule: AlarmRule = a.rule.type === 'interval' && !a.enabled &&
    a.rule.next_fire_at_secs <= Math.floor(Date.now() / 1000)
    ? { ...a.rule, next_fire_at_secs: Math.floor(Date.now() / 1000) + a.rule.interval_minutes * 60 }
    : a.rule
  try {
    await timeUpdateAlarm(a.id, {
      label: a.label,
      rule,
      enabled: !a.enabled
    })
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function remove(a: Alarm) {
  try {
    await timeRemoveAlarm(a.id)
    await refresh()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 悬浮钟 ----------

async function toggleDesktopClock() {
  try {
    await timeToggleClock()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function toggleClockPinned() {
  const pinned = !clockStyle.value.pinned
  try {
    await timeSetPinned(pinned)
    clockStyle.value = { ...clockStyle.value, pinned }
    showToast(pinned ? '悬浮钟已固定并穿透' : '已恢复悬浮钟交互', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 悬浮钟样式 ----------

const CLOCK_SKINS = [
  { id: 'mini', label: '极简', desc: '自选时间格式与排列方式' },
  { id: 'pomodoro', label: '番茄钟', desc: '专注/休息循环 + 通知' }
] as const

function clampNum(v: string, min: number, max: number, fallback: number): number {
  const n = Math.floor(Number(v))
  if (!Number.isFinite(n)) return fallback
  return Math.min(max, Math.max(min, n))
}

const clockStyle = ref<ClockStyle>({ background_opacity: 0.85, content_opacity: 1, scale: 1, color: 'accent', skin: 'mini', mini_time_format: 'hm', mini_layout: 'horizontal', font_weight: 500, pomo_work_min: 25, pomo_break_min: 5, pinned: false })
const CLOCK_FONT_WEIGHTS = [
  { value: 400, label: '细' },
  { value: 500, label: '中' },
  { value: 700, label: '粗' }
] as const
const colorInput = ref('#ffffff')
let clockStyleRevision = 0
let clockStyleSaveQueue: Promise<void> = Promise.resolve()

async function loadClockStyle() {
  const revisionBeforeLoad = clockStyleRevision
  try {
    const loaded = await getTimeClockStyle()
    if (clockStyleRevision !== revisionBeforeLoad) return
    clockStyle.value = loaded
    if (loaded.color !== 'accent') colorInput.value = loaded.color
  } catch {
    // 走默认样式
  }
}

async function applyClockStyle(patch: Partial<ClockStyle> = {}) {
  const next: ClockStyle = { ...clockStyle.value, ...patch }
  const revision = ++clockStyleRevision
  clockStyle.value = next
  const save = clockStyleSaveQueue.then(() => saveTimeClockStyle(next))
  clockStyleSaveQueue = save.catch(() => {})
  try {
    await save
    if (revision === clockStyleRevision) showToast('悬浮钟样式已应用', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function setClockColor(c: string) {
  colorInput.value = c
  void applyClockStyle({ color: c })
}

// ---------- 生命周期 ----------

onMounted(async () => {
  unlistenLifecycle = onPluginLifecycle(setPluginActive)
  try {
    unlistenEvent = await listen<{ plugin_id?: string; topic?: string; payload?: { label?: string } }>('plugin://event', (event) => {
      if (event.payload.plugin_id !== 'time') return
      if (event.payload.topic === 'time.clock_style') {
        if (pluginActive) void loadClockStyle()
        return
      }
      if (event.payload.topic === 'time.alarm') {
        const label = event.payload.payload?.label ?? '闹钟'
        if (pluginActive) {
          flashAlarm(label)
          void refresh()
        } else {
          pendingAlarmLabel = label
        }
        return
      }
    })
  } catch {
    // 事件权限缺失时仍保留低频刷新
  }
})

onUnmounted(() => {
  window.clearInterval(ticker)
  window.clearInterval(syncTimer)
  window.clearTimeout(flashTimer)
  unlistenEvent?.()
  unlistenLifecycle?.()
})
</script>

<template>
  <div class="time-view">
    <!-- 闹钟触发横幅 -->
    <div v-if="firedLabel" class="alarm-banner">
      <Icon name="clock" :size="16" />
      <span>闹钟：{{ firedLabel }} —— 时间到！</span>
    </div>

    <!-- 大时钟面板 -->
    <section class="clock-panel" :class="{ imminent }">
      <div v-if="next" class="next-line">
        <template v-if="imminent">
          <span class="countdown">{{ countdownText }}</span>
          <span class="countdown-label">{{ nextAlarm?.label || '闹钟' }} 即将响起</span>
        </template>
        <template v-else>
          <Icon name="clock" :size="14" />
          <span>
            下个闹钟 {{ nextAlarmTimeLabel }} · 还有
            {{ upcomingText }}
          </span>
        </template>
      </div>
      <div v-else class="next-line muted">暂无启用中的闹钟</div>
    </section>

    <!-- 工具栏 -->
    <div class="toolbar">
      <span class="spacer" />
      <Button variant="outline" size="sm" class="btn" title="显示 / 隐藏桌面悬浮时钟" @click="toggleDesktopClock">
        <Icon name="clock" :size="14" /> 悬浮钟
      </Button>
      <Button variant="outline" size="sm"
        class="btn"
        :title="clockStyle.pinned ? '取消固定并恢复交互' : '固定悬浮钟并启用鼠标穿透'"
        @click="toggleClockPinned"
      >
        {{ clockStyle.pinned ? '取消固定' : '固定' }}
      </Button>
    </div>

    <!-- 悬浮钟样式 -->
    <section class="clock-style">
      <h3>悬浮钟样式</h3>
      <div class="cs-row">
        <span class="cs-label">皮肤</span>
        <div class="segmented">
          <button
            v-for="sk in CLOCK_SKINS"
            :key="sk.id"
            :class="{ on: (clockStyle.skin || 'mini') === sk.id }"
            :title="sk.desc"
            @click="applyClockStyle({ skin: sk.id })"
          >
            {{ sk.label }}
          </button>
        </div>
      </div>
      <template v-if="(clockStyle.skin || 'mini') === 'mini'">
        <div class="cs-row">
          <span class="cs-label">时间格式</span>
          <div class="segmented" role="group" aria-label="极简时钟时间格式">
            <button
              :class="{ on: (clockStyle.mini_time_format || 'hm') === 'hm' }"
              :aria-pressed="(clockStyle.mini_time_format || 'hm') === 'hm'"
              @click="applyClockStyle({ mini_time_format: 'hm' })"
            >时分</button>
            <button
              :class="{ on: (clockStyle.mini_time_format || 'hm') === 'hms' }"
              :aria-pressed="(clockStyle.mini_time_format || 'hm') === 'hms'"
              @click="applyClockStyle({ mini_time_format: 'hms' })"
            >时分秒</button>
          </div>
        </div>
        <div class="cs-row">
          <span class="cs-label">排列方式</span>
          <div class="segmented" role="group" aria-label="极简时钟排列方式">
            <button
              :class="{ on: (clockStyle.mini_layout || 'horizontal') === 'horizontal' }"
              :aria-pressed="(clockStyle.mini_layout || 'horizontal') === 'horizontal'"
              @click="applyClockStyle({ mini_layout: 'horizontal' })"
            >横向</button>
            <button
              :class="{ on: (clockStyle.mini_layout || 'horizontal') === 'vertical' }"
              :aria-pressed="(clockStyle.mini_layout || 'horizontal') === 'vertical'"
              @click="applyClockStyle({ mini_layout: 'vertical' })"
            >纵向</button>
          </div>
        </div>
      </template>
      <div v-if="(clockStyle.skin || 'mini') === 'pomodoro'" class="cs-row">
        <span class="cs-label">番茄钟</span>
        <div class="pomo-durs">
          <label class="pomo-dur">
            专注
            <input
              type="number"
              min="5"
              max="90"
              :value="clockStyle.pomo_work_min"
              @change="applyClockStyle({ pomo_work_min: clampNum(($event.target as HTMLInputElement).value, 5, 90, 25) })"
            />
            分
          </label>
          <label class="pomo-dur">
            休息
            <input
              type="number"
              min="1"
              max="30"
              :value="clockStyle.pomo_break_min"
              @change="applyClockStyle({ pomo_break_min: clampNum(($event.target as HTMLInputElement).value, 1, 30, 5) })"
            />
            分
          </label>
        </div>
      </div>
      <div class="cs-row">
        <span class="cs-label">数字字重</span>
        <div class="segmented" role="group" aria-label="悬浮钟数字字重">
          <button
            v-for="weight in CLOCK_FONT_WEIGHTS"
            :key="weight.value"
            :class="{ on: (clockStyle.font_weight ?? 500) === weight.value }"
            :aria-pressed="(clockStyle.font_weight ?? 500) === weight.value"
            :title="`${weight.label}（${weight.value}）`"
            @click="applyClockStyle({ font_weight: weight.value })"
          >
            {{ weight.label }}
          </button>
        </div>
      </div>
      <div v-if="(clockStyle.skin || 'mini') === 'pomodoro'" class="cs-row">
        <span class="cs-label">背景不透明度</span>
        <RangeSlider
          :model-value="clockStyle.background_opacity"
          :min="0"
          :max="1"
          :step="0.05"
          :format="(v: number) => `${Math.round(v * 100)}%`"
          aria-label="悬浮钟背景不透明度"
          @commit="(v: number) => applyClockStyle({ background_opacity: v })"
        />
      </div>
      <div class="cs-row">
        <span class="cs-label">主体不透明度</span>
        <RangeSlider
          :model-value="clockStyle.content_opacity"
          :min="0"
          :max="1"
          :step="0.05"
          :format="(v: number) => `${Math.round(v * 100)}%`"
          aria-label="悬浮钟主体不透明度"
          @commit="(v: number) => applyClockStyle({ content_opacity: v })"
        />
      </div>
      <div class="cs-row">
        <span class="cs-label">大小</span>
        <RangeSlider
          :model-value="clockStyle.scale"
          :min="0.3"
          :max="2"
          :step="0.1"
          :format="(v: number) => `${Math.round(v * 100)}%`"
          aria-label="悬浮钟大小"
          @commit="(v: number) => applyClockStyle({ scale: v })"
        />
      </div>
      <div class="cs-row">
        <span class="cs-label">颜色</span>
        <div class="cs-colors">
          <button
            class="swatch"
            :class="{ on: clockStyle.color === 'accent' }"
            title="跟随强调色"
            @click="setClockColor('accent')"
          >
            A
          </button>
          <input
            type="color"
            class="swatch custom"
            :value="clockStyle.color === 'accent' ? colorInput : clockStyle.color"
            aria-label="自定义悬浮钟颜色"
            @input="setClockColor(($event.target as HTMLInputElement).value)"
          />
        </div>
      </div>
      <p class="cs-hint">背景不透明度只影响皮肤底板，主体可单独调节；极简皮肤没有底板。拖动悬浮钟会自动记住位置。</p>
    </section>

    <!-- 全局通知设置：应用于时间插件发出的所有通知 -->
    <section class="clock-style notification-settings">
      <h3>全局通知设置</h3>
      <div class="cs-row">
        <span class="cs-label">通知方式</span>
        <div class="segmented" role="group" aria-label="时间插件通知方式">
          <button
            :class="{ on: notificationMethod === 'native' }"
            :aria-pressed="notificationMethod === 'native'"
            @click="changeNotificationMethod('native')"
          >Windows 通知</button>
          <button
            :class="{ on: notificationMethod === 'desktop' }"
            :aria-pressed="notificationMethod === 'desktop'"
            @click="changeNotificationMethod('desktop')"
          >桌面卡片</button>
          <button
            :class="{ on: notificationMethod === 'sidebar' }"
            :aria-pressed="notificationMethod === 'sidebar'"
            @click="changeNotificationMethod('sidebar')"
          >侧边栏气泡</button>
        </div>
      </div>
      <div class="cs-row">
        <span class="cs-label">全局通知时长</span>
        <select
          class="duration-select"
          :value="notificationDurationSecs"
          aria-label="全局通知时长"
          @change="changeNotificationDuration(Number(($event.target as HTMLSelectElement).value))"
        >
          <option v-for="seconds in [3, 5, 8, 10, 12, 15, 20, 30, 45, 60]" :key="seconds" :value="seconds">{{ seconds }} 秒</option>
        </select>
      </div>
      <div class="cs-row">
        <span class="cs-label">悬浮钟闪烁提醒</span>
        <label class="switch" :title="alarmClockFlashEnabled ? '关闭悬浮钟闪烁' : '开启悬浮钟闪烁'">
          <input
            type="checkbox"
            :checked="alarmClockFlashEnabled"
            @change="setAlarmClockFlash(($event.target as HTMLInputElement).checked)"
          />
          <span class="slider" />
        </label>
      </div>
      <p class="cs-hint">悬浮钟会在闹钟前最后一分钟和触发时闪烁。通知方式与时长适用于所有闹钟规则和番茄钟；Windows 通知时长由系统管理。</p>
    </section>

    <!-- 闹钟模块：预设只填充通用规则，不创建独立功能 -->
    <section class="clock-style alarm-settings">
      <h3>闹钟</h3>
      <p class="cs-hint">从空白规则开始，或用预设快速创建；保存后都可以继续编辑。</p>
      <div class="alarm-presets">
        <button class="preset-card" @click="openAdd('hourly')">
          <span class="preset-title"><Icon name="clock" :size="15" /> 整点提醒</span>
          <span class="preset-description">从下一个整点开始，每 60 分钟提醒</span>
        </button>
        <button class="preset-card" @click="openAdd('interval')">
          <span class="preset-title"><Icon name="clock" :size="15" /> 间隔提醒</span>
          <span class="preset-description">自选间隔、开始时间和提醒内容</span>
        </button>
      </div>

      <div class="alarm-list-heading">
        <h4 class="alarm-list-title">所有闹钟</h4>
        <Button variant="default" size="sm" class="btn primary" @click="openAdd"><Icon name="plus" :size="14" /> 新建闹钟</Button>
      </div>
      <div class="list" role="table">
        <div class="row head" role="row">
          <span class="c-time">下次</span>
          <span class="c-repeat">规则</span>
          <span class="c-label">标签</span>
          <span class="c-state">启用</span>
          <span class="c-actions">操作</span>
        </div>
        <div v-for="a in sortedAlarms" :key="a.id" class="row" role="row" :class="{ off: !a.enabled }">
          <span class="c-time strong">{{ alarmTimeLabel(a) }}</span>
          <span class="c-repeat">{{ alarmRuleLabel(a) }}</span>
          <span class="c-label" :title="a.label">{{ a.label || '—' }}</span>
          <span class="c-state">
            <label class="switch" :title="a.enabled ? '停用' : '启用'">
              <input type="checkbox" :checked="a.enabled" @change="toggleEnabled(a)" />
              <span class="slider" />
            </label>
          </span>
          <span class="c-actions">
            <Button variant="outline" size="sm" class="icon-btn" title="编辑" @click="openEdit(a)">
              <Icon name="pencil" :size="14" />
            </Button>
            <Button variant="destructive" size="sm" class="icon-btn danger" title="删除" @click="remove(a)">
              <Icon name="trash" :size="14" />
            </Button>
          </span>
        </div>
        <div v-if="sortedAlarms.length === 0" class="empty-row">
          还没有闹钟。试试上方的预设，或新建一个闹钟。
        </div>
      </div>
    </section>

    <!-- 新建 / 编辑对话框 -->
    <Teleport to="body">
      <div v-if="showDialog" class="mask" @click.self="showDialog = false">
        <div class="dialog">
          <h2>{{ editingId ? '编辑闹钟' : '新建闹钟' }}</h2>

          <div class="field">
            <span>规则类型</span>
            <div class="segmented rule-type-switch">
              <button :class="{ on: form.ruleType === 'time' }" @click="form.ruleType = 'time'">指定时间</button>
              <button :class="{ on: form.ruleType === 'interval' }" @click="form.ruleType = 'interval'">间隔重复</button>
            </div>
          </div>

          <template v-if="form.ruleType === 'time'">
            <label class="field">
              <span>触发时间 *</span>
              <input v-model="form.time" type="time" required />
            </label>

            <div class="field">
              <span>重复日期</span>
              <div class="repeat-presets">
                <button type="button" class="chip" @click="setRepeatPreset('once')">仅一次</button>
                <button type="button" class="chip" @click="setRepeatPreset('daily')">每天</button>
                <button type="button" class="chip" @click="setRepeatPreset('weekdays')">工作日</button>
                <button type="button" class="chip" @click="setRepeatPreset('weekend')">周末</button>
              </div>
              <div class="chips">
                <button
                  v-for="d in WEEKDAY_CHIP_ORDER"
                  :key="d"
                  type="button"
                  class="chip"
                  :class="{ on: form.weekdays.includes(d) }"
                  @click="toggleWeekday(d)"
                >
                  {{ WEEKDAY_LABELS[d] }}
                </button>
              </div>
            </div>
            <p class="form-hint">不选择日期表示只触发一次；也可以组合选择任意星期。</p>
          </template>

          <template v-else>
            <label class="field">
              <span>每隔多少分钟 *</span>
              <input v-model.number="form.intervalMinutes" type="number" min="1" max="10080" required />
              <small>可设置 1 分钟到 7 天。第一次提醒按下方开始时间，之后按间隔重复。</small>
            </label>
            <label class="field">
              <span>首次提醒时间 *</span>
              <input v-model="form.nextFireAt" type="datetime-local" required />
            </label>
            <label class="field">
              <span>提醒内容</span>
              <input v-model="form.message" type="text" maxlength="100" placeholder="时间到了，休息一下吧。" />
              <small>可用 {time} 插入触发时的当前时间。</small>
            </label>
          </template>

          <label class="field">
            <span>标签</span>
            <input v-model="form.label" type="text" placeholder="例如：起床 / 会议提醒" maxlength="100" />
          </label>

          <label class="check">
            <input v-model="form.enabled" type="checkbox" />
            <span>启用此闹钟</span>
          </label>

          <footer class="dialog-foot">
            <Button variant="outline" size="sm" class="btn" @click="showDialog = false">取消</Button>
            <Button variant="default" size="sm" class="btn primary" @click="save">保存</Button>
          </footer>
        </div>
      </div>
    </Teleport>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.time-view {
  display: flex;
  flex-direction: column;
  min-height: 100%;
  height: auto;
  gap: 12px;
}

/* ---------- 闹钟触发横幅 ---------- */
.alarm-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 14px;
  border-radius: var(--radius, 12px);
  border: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--text);
  font-size: 13px;
  animation: banner-pulse 1.2s ease-in-out infinite;
}
@keyframes banner-pulse {
  0%,
  100% {
    box-shadow: 0 0 0 0 transparent;
  }
  50% {
    box-shadow: 0 0 18px 2px color-mix(in srgb, var(--accent) 45%, transparent);
  }
}

/* ---------- 时钟面板 ---------- */
.clock-panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex: none;
  gap: 6px;
  padding: 16px;
  border-radius: 16px;
  border: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.clock-panel.imminent {
  border-color: color-mix(in srgb, var(--accent) 60%, transparent);
  background: color-mix(in srgb, var(--accent) 10%, var(--surface-subtle) 60%);
}

.next-line {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  font-size: 12.5px;
  color: var(--text-muted);
}
.next-line.muted {
  opacity: 0.8;
}
.next-line .countdown {
  font-size: 30px;
  font-weight: 800;
  line-height: 1;
  letter-spacing: 1.5px;
  font-variant-numeric: tabular-nums;
  color: var(--accent);
  filter: brightness(1.35);
  text-shadow: 0 0 16px color-mix(in srgb, var(--accent) 70%, transparent);
  animation: countdown-pulse 1s ease-in-out infinite;
}
.next-line .countdown-label {
  font-size: 12.5px;
  color: var(--text);
}
@keyframes countdown-pulse {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.08);
  }
}

/* ---------- 工具栏 ---------- */
.toolbar {
  display: flex;
  align-items: center;
  flex: none;
  gap: 8px;
  flex-wrap: wrap;
}
.spacer {
  flex: 1;
}

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 12.5px;
}
.btn:hover {
  border-color: var(--border-strong);
}
.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.btn.primary:hover {
  filter: brightness(1.1);
}

/* ---------- 闹钟列表 ---------- */
.list {
  flex: none;
  min-height: 0;
  overflow: visible;
  border: 1px solid var(--border);
  border-radius: var(--radius, 12px);
  background: color-mix(in srgb, var(--surface-subtle) 55%, transparent);
}
.alarm-list-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: 4px 0 0;
  padding-top: 10px;
  border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
}
.alarm-presets {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: 8px;
}
.preset-card {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--surface-subtle);
  color: var(--text);
  text-align: left;
  cursor: pointer;
}
.preset-card:hover {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 8%, var(--surface-subtle));
}
.preset-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12.5px;
  font-weight: 600;
}
.preset-description {
  color: var(--text-muted);
  font-size: 11px;
}

.row {
  display: grid;
  grid-template-columns: 84px 1fr 1.4fr 64px 92px;
  align-items: center;
  min-height: 46px;
  padding: 0 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 45%, transparent);
  font-size: 12.5px;
}
.row:last-child {
  border-bottom: none;
}
.row:not(.head):hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}
.row.off .c-time,
.row.off .c-repeat,
.row.off .c-label {
  opacity: 0.45;
}

.row.head {
  position: sticky;
  top: 0;
  min-height: 36px;
  background: var(--surface-container);
  color: var(--text-muted);
  font-weight: 600;
  z-index: 1;
}

.c-time,
.c-repeat,
.c-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding-right: 10px;
}
.strong {
  font-size: 16px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.c-state {
  display: flex;
  justify-content: center;
}
.c-actions {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
}
.icon-btn:hover {
  background: var(--surface-container);
  color: var(--accent);
}
.icon-btn.danger:hover {
  color: var(--danger);
}

.empty-row {
  padding: 32px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: 12.5px;
}

/* ---------- 开关 ---------- */
.switch {
  position: relative;
  display: inline-block;
  width: 34px;
  height: 19px;
  cursor: pointer;
}
.switch input {
  display: none;
}
.switch .slider {
  position: absolute;
  inset: 0;
  border-radius: 999px;
  background: var(--border);
  transition: background 0.18s;
}
.switch .slider::before {
  content: '';
  position: absolute;
  top: 2.5px;
  left: 3px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.18s;
}
.switch input:checked + .slider {
  background: var(--accent);
}
.switch input:checked + .slider::before {
  transform: translateX(14px);
}

/* ---------- 对话框 ---------- */
.mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.dialog {
  width: min(420px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow: auto;
  border-radius: 16px;
  border: 1px solid var(--border);
  background: var(--surface);
  box-shadow: var(--shadow);
  padding: 20px;
}
.dialog h2 {
  margin: 0 0 16px;
  font-size: 15px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-bottom: 12px;
  font-size: 12px;
  color: var(--text-muted);
}
.field input {
  padding: 7px 10px;
  border-radius: 9px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
  color-scheme: light dark;
  outline: none;
}
.field input:focus {
  border-color: var(--accent);
}
.field small,
.form-hint {
  margin: 0;
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.45;
}
.repeat-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 6px;
}
.rule-type-switch {
  align-self: flex-start;
}
.form-hint {
  margin: -4px 0 12px;
}

.chips {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}
.chip {
  padding: 5px 11px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text-muted);
  font-size: 12px;
}
.chip:hover {
  border-color: var(--border-strong);
}
.chip.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 22%, transparent);
  color: var(--text);
}

.check {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  font-size: 12.5px;
  color: var(--text);
}
.check input {
  accent-color: var(--accent);
}

.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}
/* 悬浮钟样式设置 */
.clock-style {
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  border-radius: var(--radius, 12px);
  padding: 12px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 12px;
}
.clock-style h3 {
  margin: 0;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-muted);
}
.cs-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.cs-label {
  font-size: 12.5px;
  color: var(--text);
}
.segmented {
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--border) 40%, transparent);
}
.segmented button {
  padding: 4px 14px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
}
.segmented button.on {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
}
.cs-colors {
  display: flex;
  align-items: center;
  gap: 8px;
}
.swatch {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  border: 2px solid var(--border);
  background: var(--accent);
  color: #fff;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}
.swatch.on {
  border-color: var(--text);
}
.swatch.custom {
  width: 28px;
  height: 28px;
  border: 1px dashed var(--border-strong);
  background: transparent;
  padding: 0;
}
.cs-hint {
  margin: 0;
  font-size: 11px;
  color: var(--text-muted);
}
.alarm-settings .check {
  margin: 0;
}
.duration-select {
  min-width: 100px;
  padding: 6px 28px 6px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface-subtle);
  color: var(--text);
  font: inherit;
  font-size: 12px;
}
.alarm-list-title {
  margin: 0;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 600;
}
.pomo-durs {
  display: flex;
  align-items: center;
  gap: 14px;
}
.pomo-dur {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
}
.pomo-dur input {
  width: 56px;
  padding: 5px 8px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
}

</style>
