<script setup lang="ts">
/** 桌面悬浮时钟：大字时钟，闹钟前 ≤5 分钟切换大倒计时；
 * 根元素 data-tauri-drag-region 支持拖动，固定会启用宿主原生鼠标穿透。
 * 监听全局事件 time://alarm，并按设置闪动提醒。 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '../../../ui-sdk/runtime'
import {
  COUNTDOWN_THRESHOLD_SECS,
  timeNotify,
  TIME_EVENTS,
  formatCountdown,
  pad2,
  timeSnapshot,
  getTimeClockStyle,
  getInitialTimeClockStyle,
  CLOCK_STYLE_EVENTS,
  type ClockStyle
} from './api'
import type { AlarmFiredPayload, NextAlarm, TimeSnapshot } from './api'

const now = ref(new Date())
const alarms = ref<{ id: string; label: string; hour: number; minute: number }[]>([])
const next = ref<NextAlarm | null>(null)
const alarmFlashEnabled = ref(true)
const flashing = ref(false)
const initialStyle = getInitialTimeClockStyle()
const clickthrough = ref(!!initialStyle?.pinned)
const style = ref<ClockStyle>(initialStyle ?? { background_opacity: 0.85, content_opacity: 1, scale: 1, color: 'accent', skin: 'mini', mini_time_format: 'hm', mini_layout: 'horizontal', font_weight: 500, pomo_work_min: 25, pomo_break_min: 5, pinned: false })
const STYLE_RETRY_DELAYS = [150, 300, 600, 1_000, 2_000, 3_000]

let ticker = 0
let syncTimer = 0
let flashTimer = 0
let styleRetryTimer = 0
let styleLoadStopped = false
let unlistenAlarm: UnlistenFn | null = null
let unlistenSnapshot: UnlistenFn | null = null
let unlistenClickthrough: UnlistenFn | null = null
let unlistenVisibility: UnlistenFn | null = null
let pageVisible = !document.hidden

/** 低频同步快照：拿闹钟列表与下个闹钟（走秒走本地时钟，不每秒请求后端） */
async function refresh() {
  try {
    const snap = await timeSnapshot()
    alarms.value = snap.alarms
    next.value = snap.next
    alarmFlashEnabled.value = snap.alarm_clock_flash_enabled ?? true
  } catch {
    // 后端暂不可用时静默，本地时钟照常显示
  }
}

function flash() {
  flashing.value = true
  window.clearTimeout(flashTimer)
  flashTimer = window.setTimeout(() => {
    flashing.value = false
  }, 6000)
}

// ---------- 展示 ----------

const miniTimeParts = computed(() => {
  const parts = [pad2(now.value.getHours()), pad2(now.value.getMinutes())]
  if (style.value.mini_time_format === 'hms') parts.push(pad2(now.value.getSeconds()))
  return parts
})
const miniTimeText = computed(() => miniTimeParts.value.join(':'))

const nextAlarm = computed(() =>
  next.value ? alarms.value.find((a) => a.id === next.value?.alarm_id) : undefined
)
const remaining = computed(() => {
  if (!next.value) return null
  return Math.max(0, Math.round(next.value.at_secs - now.value.getTime() / 1000))
})
/** ≤5 分钟切换大倒计时态 */
const imminent = computed(
  () =>
    remaining.value !== null &&
    remaining.value <= COUNTDOWN_THRESHOLD_SECS
)
const finalMinute = computed(
  () => alarmFlashEnabled.value && remaining.value !== null && remaining.value <= 60
)
const countdownText = computed(() =>
  remaining.value === null ? '' : formatCountdown(remaining.value)
)
const alarmStatusText = computed(() => {
  if (remaining.value === null) return ''
  return formatCountdown(remaining.value)
})

// ---------- 样式 ----------

const skin = computed(() => style.value.skin || 'mini')
const skinClass = computed(() => `skin-${skin.value}`)

const rootStyle = computed(() => {
  const s = style.value
  // 颜色：accent=跟随强调色；自定义 hex 写入 --clock-color 供 .time/.countdown 消费
  const clockColor = s.color === 'accent' ? 'var(--accent)' : s.color
  const rawClockColor = s.color === 'accent'
    ? getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
    : s.color
  const rgb = parseClockColor(rawClockColor)
  const alarmFlashColor = rgb && rgb[0] > rgb[1] * 1.2 && rgb[0] > rgb[2] * 1.2
    ? '#438dff'
    : '#ff3b30'
  const style_ = {
    '--clock-color': clockColor,
    '--alarm-flash-color': alarmFlashColor,
    '--clock-font-weight': String(s.font_weight ?? 500),
    '--clock-content-opacity': String(s.content_opacity ?? 1),
    background: skin.value === 'mini'
      ? 'transparent'
      : `color-mix(in srgb, var(--bg) ${Math.round((s.background_opacity ?? s.opacity ?? 0.85) * 100)}%, transparent)`,
    border: skin.value === 'mini' ? 'none' : '1px solid color-mix(in srgb, var(--border) 70%, transparent)',
    boxShadow: skin.value === 'mini' ? 'none' : '0 8px 26px rgba(0, 0, 0, 0.35)',
    borderRadius: skin.value === 'mini' ? '0' : '16px'
  }
  return style_
})

function parseClockColor(value: string): [number, number, number] | null {
  const hex = value.match(/^#?([\da-f]{3}|[\da-f]{6})$/i)?.[1]
  if (hex) {
    const fullHex = hex.length === 3 ? [...hex].map((digit) => digit + digit).join('') : hex
    return [0, 2, 4].map((index) => Number.parseInt(fullHex.slice(index, index + 2), 16)) as [number, number, number]
  }
  const rgb = value.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i)
  return rgb ? [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])] : null
}
const timeFont = computed(() => ({
  fontSize: `${46 * style.value.scale}px`
}))

function stopClockTicker() {
  window.clearTimeout(ticker)
  ticker = 0
}

function startClockTicker() {
  stopClockTicker()
  if (!pageVisible) return
  now.value = new Date()
  const tick = () => {
    if (!pageVisible) return
    const current = new Date()
    const updatesBySecond =
      skin.value === 'pomodoro' || style.value.mini_time_format === 'hms' || imminent.value
    const delay = updatesBySecond
      ? 1000 - current.getMilliseconds()
      : 60_000 - (current.getSeconds() * 1000 + current.getMilliseconds())
    ticker = window.setTimeout(() => {
      now.value = new Date()
      tick()
    }, delay)
  }
  tick()
}

function waitForStyleRetry(delay: number) {
  return new Promise<void>((resolve) => {
    styleRetryTimer = window.setTimeout(resolve, delay)
  })
}

async function loadClockStyle() {
  for (let attempt = 0; !styleLoadStopped; attempt += 1) {
    try {
      const loaded = await getTimeClockStyle()
      if (styleLoadStopped) return
      style.value = { ...style.value, ...loaded }
      clickthrough.value = !!style.value.pinned
      if (pageVisible) startClockTicker()
      return
    } catch {
      const delay = STYLE_RETRY_DELAYS[Math.min(attempt, STYLE_RETRY_DELAYS.length - 1)]
      await waitForStyleRetry(delay)
    }
  }
}

// ---------- 番茄钟（skin=pomodoro） ----------

const pomo = ref({ phase: 'work' as 'work' | 'break', remaining: 25 * 60, running: false, rounds: 0 })
let pomoTick = 0

const pomoDuration = computed(() => {
  const s = style.value
  return ((pomo.value.phase === 'work' ? s.pomo_work_min : s.pomo_break_min) || 25) * 60
})
const pomoText = computed(() => {
  const m = Math.floor(pomo.value.remaining / 60)
  const s = pomo.value.remaining % 60
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
})
const pomoPct = computed(() => {
  const total = Math.max(1, pomoDuration.value)
  return Math.round(((total - pomo.value.remaining) / total) * 100)
})

function pomoToggle() {
  pomo.value.running = !pomo.value.running
  pomoEnsureTimer()
}

function pomoReset() {
  pomo.value.running = false
  pomo.value.remaining = pomoDuration.value
  pomoEnsureTimer()
}

function pomoEnsureTimer() {
  window.clearInterval(pomoTick)
  pomoTick = 0
  if (!pomo.value.running || skin.value !== 'pomodoro') return
  pomoTick = window.setInterval(() => {
    if (!pomo.value.running || skin.value !== 'pomodoro') return
    pomo.value.remaining -= 1
    if (pomo.value.remaining > 0) return
    const finished = pomo.value.phase
    const next = finished === 'work' ? 'break' : 'work'
    if (next === 'work') pomo.value.rounds += 1
    pomo.value.phase = next
    pomo.value.remaining = pomoDuration.value
    void timeNotify(
      'Agile Tiles 番茄钟',
      finished === 'work' ? '专注结束，休息一下吧 ☕' : '休息结束，开始新的专注 🍅'
    )
  }, 1000)
}

watch(
  () => [style.value.pomo_work_min, style.value.pomo_break_min],
  () => {
    if (!pomo.value.running) pomo.value.remaining = pomoDuration.value
  }
)

onMounted(() => {
  pomoEnsureTimer()
  refresh()
  if (!initialStyle) void loadClockStyle()
  listen<ClockStyle>(CLOCK_STYLE_EVENTS.styleChanged, (e) => {
    style.value = { ...style.value, ...e.payload }
    clickthrough.value = !!style.value.pinned
    if (pageVisible) startClockTicker()
    pomoEnsureTimer()
  }).catch(() => {})
  // 穿透状态以后端广播为准：详情页"恢复交互"等外部路径变更时本地标记不再滞后
  listen<boolean>(TIME_EVENTS.clickthrough, (e) => {
    clickthrough.value = !!e.payload
    style.value = { ...style.value, pinned: clickthrough.value }
  })
    .then((fn) => {
      unlistenClickthrough = fn
    })
    .catch(() => {})
  listen<TimeSnapshot>(TIME_EVENTS.snapshot, (e) => {
    alarms.value = e.payload.alarms
    next.value = e.payload.next
    alarmFlashEnabled.value = e.payload.alarm_clock_flash_enabled ?? true
  })
    .then((fn) => {
      unlistenSnapshot = fn
    })
    .catch(() => {})
  listen<boolean>(TIME_EVENTS.visibility, (e) => {
    pageVisible = e.payload
    if (pageVisible) {
      startClockTicker()
      if (!syncTimer) syncTimer = window.setInterval(refresh, 30_000)
      void refresh()
    } else {
      stopClockTicker()
      window.clearInterval(syncTimer)
      syncTimer = 0
    }
  })
    .then((fn) => {
      unlistenVisibility = fn
    })
    .catch(() => {})
  window.addEventListener('visibilitychange', onDocumentVisibilityChange)
  if (pageVisible) {
    startClockTicker()
    syncTimer = window.setInterval(refresh, 30_000)
  }
  listen<AlarmFiredPayload>(TIME_EVENTS.alarm, (payload) => {
    if (payload.flash !== false) flash()
    refresh()
  })
    .then((fn) => {
      unlistenAlarm = fn
    })
    .catch(() => {
      // 事件能力缺失时静默降级
    })
})

onUnmounted(() => {
  window.clearInterval(pomoTick)
  stopClockTicker()
  window.clearInterval(syncTimer)
  window.clearTimeout(flashTimer)
  window.clearTimeout(styleRetryTimer)
  styleLoadStopped = true
  unlistenAlarm?.()
  unlistenSnapshot?.()
  unlistenClickthrough?.()
  unlistenVisibility?.()
  window.removeEventListener('visibilitychange', onDocumentVisibilityChange)
})

function onDocumentVisibilityChange() {
  pageVisible = !document.hidden
  if (!pageVisible) {
    stopClockTicker()
    window.clearInterval(syncTimer)
    syncTimer = 0
    return
  }
  startClockTicker()
  if (!syncTimer) syncTimer = window.setInterval(refresh, 30_000)
  void refresh()
}
</script>

<template>
    <div
      class="clock-root"
      :data-tauri-drag-region="clickthrough ? undefined : ''"
      :class="[skinClass, { flashing, imminent, locked: clickthrough }]"
      :style="rootStyle"
      @contextmenu.prevent
    >
      <div class="main" :data-tauri-drag-region="clickthrough ? undefined : ''">
      <template v-if="skin === 'pomodoro' && !imminent">
        <div class="pomo" :data-tauri-drag-region="clickthrough ? undefined : ''">
          <div class="pomo-phase" :data-tauri-drag-region="clickthrough ? undefined : ''">
            {{ pomo.phase === 'work' ? '专注中' : '休息中' }} · 第 {{ pomo.rounds + 1 }} 轮
          </div>
          <div class="pomo-time" :data-tauri-drag-region="clickthrough ? undefined : ''" :style="timeFont">{{ pomoText }}</div>
          <div class="pomo-bar" :data-tauri-drag-region="clickthrough ? undefined : ''">
            <div class="pomo-fill" :style="{ width: pomoPct + '%' }" />
          </div>
          <div class="pomo-btns">
            <button class="pomo-btn" @click.stop="pomoToggle">
              {{ pomo.running ? '暂停' : '开始' }}
            </button>
            <button class="pomo-btn" @click.stop="pomoReset">重置</button>
          </div>
        </div>
      </template>
      <template v-else-if="skin === 'mini'">
        <div
          class="mini-display"
          :style="{ fontSize: (style.mini_layout === 'vertical' ? 38 : 46) * style.scale + 'px' }"
          :data-tauri-drag-region="clickthrough ? undefined : ''"
        >
          <div class="mini-time" :class="{ 'alarm-pulse': finalMinute }">
            <div v-if="style.mini_layout === 'vertical'" class="mini-vertical">
              <span>{{ miniTimeParts[0] }}</span>
              <span class="separator">:</span>
              <span>{{ miniTimeParts[1] }}</span>
              <template v-if="style.mini_time_format === 'hms'">
                <span class="separator">:</span>
                <span>{{ miniTimeParts[2] }}</span>
              </template>
            </div>
            <div v-else class="time">{{ miniTimeText }}</div>
          </div>
          <div v-if="imminent" class="alarm-status">{{ alarmStatusText }}</div>
        </div>
      </template>
      <template v-else>
        <div class="countdown" :style="{ fontSize: (56 * style.scale) + 'px' }">{{ countdownText }}</div>
        <div class="date warn">{{ nextAlarm?.label || '闹钟' }} 即将响起</div>
      </template>
      </div>

  </div>
</template>

<style scoped>
.clock-root {
  position: relative;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  border-radius: 16px;
  border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  background: color-mix(in srgb, var(--bg) 84%, transparent);
  box-shadow: 0 8px 26px rgba(0, 0, 0, 0.35);
  overflow: hidden;
  user-select: none;
}

.mini-display {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.04em;
  width: 100%;
  max-width: 100%;
}

.mini-time.alarm-pulse .time,
.mini-time.alarm-pulse .mini-vertical {
  animation: mini-time-pulse 2.4s steps(1, end) infinite;
}

@keyframes mini-time-pulse {
  0%, 100% { color: var(--clock-color, var(--text)); }
  50% { color: var(--alarm-flash-color, #ff3b30); }
}

.alarm-status {
  max-width: 100%;
  overflow: hidden;
  color: #ffbf69;
  font-family: "Segoe UI Variable", "Segoe UI", sans-serif;
  font-size: 0.35em;
  font-weight: 500;
  line-height: 1.1;
  letter-spacing: 0.035em;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.main {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  opacity: var(--clock-content-opacity, 1);
}

.time {
  font-size: inherit;
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Bahnschrift", "Segoe UI", sans-serif;
  font-weight: var(--clock-font-weight, 500);
  line-height: 1.02;
  letter-spacing: 0.045em;
  font-variant-numeric: tabular-nums;
  color: var(--clock-color, var(--text));
  white-space: nowrap;
}
.date {
  font-size: 12px;
  color: var(--text-muted);
}

/* 倒计时态 */
.countdown {
  font-size: 56px;
  font-weight: var(--clock-font-weight, 500);
  line-height: 1.02;
  letter-spacing: 2px;
  font-variant-numeric: tabular-nums;
  color: var(--clock-color, var(--accent));
  white-space: nowrap;
  filter: brightness(1.35);
  text-shadow: 0 0 18px color-mix(in srgb, var(--clock-color, var(--accent)) 70%, transparent);
  animation: countdown-pulse 1s ease-in-out infinite;
}
.date.warn {
  font-size: 12.5px;
  color: var(--text);
}
@keyframes countdown-pulse {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.06);
  }
}

/* 闹钟响铃闪动 */
.clock-root.flashing {
  border-color: var(--accent);
  animation: ring 1s ease-in-out infinite;
}
@keyframes ring {
  0%,
  100% {
    box-shadow: 0 0 0 0 transparent;
  }
  50% {
    box-shadow: 0 0 26px 4px color-mix(in srgb, var(--accent) 65%, transparent);
  }
}

.clock-root.locked,
.clock-root.locked * {
  -webkit-app-region: no-drag !important;
}

.skin-mini .main { gap: 0; align-items: center; width: 100%; }
.skin-mini .time {
  font-weight: var(--clock-font-weight, 500);
  text-shadow: 0 2px 10px rgba(0, 0, 0, 0.55);
}

:global(html),
:global(body),
:global(#app) {
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: transparent !important;
}

.pomo {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  width: 100%;
}
.pomo-phase {
  font-size: 11px;
  opacity: 0.7;
  letter-spacing: 1px;
}
.pomo-time {
  line-height: 1.1;
  font-variant-numeric: tabular-nums;
  font-weight: var(--clock-font-weight, 500);
  color: var(--clock-color, var(--text));
}
.pomo-bar {
  width: 80%;
  height: 5px;
  border-radius: 999px;
  background: color-mix(in srgb, currentColor 18%, transparent);
  overflow: hidden;
}
.pomo-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--clock-color, var(--accent));
  transition: width 0.4s linear;
}
.pomo-btns {
  display: flex;
  gap: 8px;
  margin-top: 2px;
}
.pomo-btn {
  padding: 2px 14px;
  border-radius: 7px;
  border: 1px solid color-mix(in srgb, currentColor 35%, transparent);
  background: transparent;
  color: inherit;
  font-size: 11px;
  cursor: pointer;
  -webkit-app-region: no-drag;
}
.pomo-btn:hover {
  background: color-mix(in srgb, currentColor 14%, transparent);
}

.mini-vertical {
  display: flex;
  flex-direction: column;
  align-items: center;
  color: var(--clock-color, var(--text));
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Bahnschrift", "Segoe UI", sans-serif;
  font-weight: var(--clock-font-weight, 500);
  font-variant-numeric: tabular-nums;
  line-height: 0.92;
  letter-spacing: 0.045em;
}

.mini-vertical .separator {
  font-size: 0.48em;
  line-height: 0.6;
  opacity: 0.65;
}
</style>
