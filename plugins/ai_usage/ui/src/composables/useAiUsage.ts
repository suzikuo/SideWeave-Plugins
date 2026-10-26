import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { errorMessage, onPluginLifecycle, pluginCall } from '../../../../ui-sdk/runtime'
import { resetCountdown, resetTimeText } from '../quota-display'

const DATA_REFRESH_MS = 60_000
const QUOTA_REFRESH_MS = 300_000

interface SyncStatus {
  running: boolean
  last_finished_ms: number | null
  warning: string | null
}

type PeriodKey = 'day' | 'week' | 'month' | 'year'
type PageKey = 'overview' | 'tools' | 'sources' | 'sessions' | 'requests' | 'quotas' | 'preview'
type PreviewWidget = 'tokens' | 'cost' | 'requests' | 'quotas' | 'tools' | 'split' | 'updated'
type PreviewDisplayMode = 'cards' | 'compact'

interface PreviewSettings {
  widgets: PreviewWidget[]
  display_mode: PreviewDisplayMode
}

interface QuotaNotificationSettings {
  // Legacy fields are migrated to per-subscription drafts when loaded.
  enabled: boolean
  remaining_thresholds_percent: number[]
  subscriptions: Record<string, QuotaSubscriptionNotificationRule>
  available_subscriptions?: Array<{ key: string; quota: Quota }>
}

interface QuotaSubscriptionNotificationRule {
  enabled: boolean
  reset_enabled: boolean
  remaining_thresholds_percent: number[]
}

interface QuotaSubscriptionNotificationDraft {
  enabled: boolean
  reset_enabled: boolean
  thresholdsText: string
}

interface PreviewData {
  requests: number
  total_tokens: number
  input_tokens: number
  output_tokens: number
  last_ingest_ms: number | null
  tools: Array<{ tool: string; total_tokens: number; requests: number }>
  quotas: Quota[]
  quota_estimates?: QuotaEstimate[]
  cost_usd: number
  credit_cost_usd: number
  unpriced_models: number
  settings: PreviewSettings
}

interface Counts {
  input: number
  cache_creation: number
  cache_read: number
  output: number
  reasoning: number
  credits: number
}

interface Summary {
  counts: Counts
  total_tokens: number
  cost: number
  credits: number
  credit_cost: number
  requests: number
  sessions: number
  cached_pct: number
  unpriced: Array<{ tool: string; model: string; total_tokens: number; requests: number }>
}

interface Item {
  key: string
  label: string
  counts: Counts
  total_tokens: number
  cost: number
  requests: number
  sessions: number
  priced: boolean
}

interface UsageWindow {
  key: string
  label: string
  summary: Summary
  prev: Summary
  delta_cost_pct: number
  delta_tokens_pct: number
  breakdown: {
    tools: Item[]
    models: Item[]
    projects: Item[]
  }
}

interface Quota {
  tool: string
  used_percent: number
  window_minutes: number
  resets_at_ms: number
  sampled_at_ms: number
  label: string | null
  id?: string
  origin: string
}

interface QuotaEstimate {
  key: string
  used_cost_usd: number | null
  total_cost_usd: number | null
  remaining_cost_usd: number | null
  reason: string | null
}

interface UsageReport {
  generated_at_ms: number
  utc_offset: string
  day: UsageWindow
  week: UsageWindow
  month: UsageWindow
  year: UsageWindow
  heatmap: Array<{ date: string; total_tokens: number; cost: number; requests: number }>
  hourly: Array<{ total_tokens: number; cost: number; requests: number }>
  quotas: Quota[]
  quotas_pending: boolean
  sources: Array<{ id: string; display: string; detected: boolean; events_ingested: number; hint: string | null }>
  pricing: { source: string; fetched_at_ms: number; stale: boolean; key_count: number }
  recent_sessions: Array<{
    tool: string
    session: string
    session_title: string | null
    project: string | null
    model: string
    first_ms: number
    last_ms: number
    total_tokens: number
    cost: number
    requests: number
  }>
  all_time: Summary
}

interface DashboardPayload {
  report: UsageReport
  quota_estimates?: QuotaEstimate[]
  indexed_events: number
  last_ingest_ms: number | null
  refresh_warning: string | null
  supported_tools: Array<{ id: string; display: string }>
  data_roots: Array<{ id: string; path: string }>
  accounts: Array<{ tool: string; selection: string; label: string; name: string | null; requests: number; sessions: number }>
}

interface RequestRow {
  tool: string
  timestamp_ms: number
  session: string
  session_title: string | null
  account_id: string | null
  duration_ms: number | null
  project: string | null
  model: string | null
  counts: Counts
  total_tokens: number
  cost_usd: number | null
  credits_cost_usd: number | null
  pricing_alias?: string | null
}

interface RequestFilters {
  since_ms: number
  until_ms: number
  scope: string
  account: string
  tool: string
  search: string
}

interface SessionRow {
  tool: string
  session: string
  session_title: string | null
  account_id: string | null
  project: string | null
  model: string
  first_ms: number
  last_ms: number
  total_tokens: number
  cost: number
  requests: number
}

export function useAiUsage() {
  const pages: Array<{ id: PageKey; label: string; icon: string }> = [
    { id: 'overview', label: '总览', icon: '◫' },
    { id: 'tools', label: '工具排行', icon: '⌘' },
    { id: 'sources', label: '来源管理', icon: '⌂' },
    { id: 'sessions', label: '会话排行', icon: '▤' },
    { id: 'requests', label: '请求明细', icon: '≡' },
    { id: 'quotas', label: '订阅配额', icon: '◷' },
    { id: 'preview', label: '悬停预览', icon: '▧' }
  ]
  const previewWidgetOptions: Array<{ id: PreviewWidget; title: string; description: string }> = [
    { id: 'tokens', title: 'Token 总量', description: '显示最近 24 小时的输入和输出 Token' },
    { id: 'cost', title: '费用估算', description: '显示可匹配价格的模型费用估算' },
    { id: 'requests', title: '请求次数', description: '显示最近 24 小时的用量记录数' },
    { id: 'quotas', title: '订阅配额', description: '显示各工具的剩余额度、额度窗口和重置时间' },
    { id: 'tools', title: '工具排行', description: '显示 Token 用量最高的工具' },
    { id: 'split', title: '输入 / 输出', description: '用进度条对比输入和输出 Token' },
    { id: 'updated', title: '索引更新时间', description: '显示本机日志上次扫描时间' }
  ]
  const defaultPreviewSettings: PreviewSettings = {
    widgets: ['tokens', 'cost', 'requests', 'quotas', 'tools', 'split', 'updated'],
    display_mode: 'cards'
  }
  const periodOptions: Array<{ id: PeriodKey; label: string }> = [
    { id: 'day', label: '今天' },
    { id: 'week', label: '本周' },
    { id: 'month', label: '本月' },
    { id: 'year', label: '今年' }
  ]
  const trendOptions = [7, 30, 90, 365]

  const page = ref<PageKey>('overview')
  const scope = ref('all')
  const accountScope = ref('all')
  const accountNameEditing = ref(false)
  const accountNameSaving = ref(false)
  const accountNameDraft = ref('')
  const accountNameError = ref('')
  const rootPathDraft = ref('')
  const rootsSaving = ref(false)
  const rootsError = ref('')
  const period = ref<PeriodKey>('day')
  const trendDays = ref(30)
  const payload = ref<DashboardPayload | null>(null)
  const loading = ref(false)
  const error = ref('')
  const syncing = ref(false)
  const syncWarning = ref('')
  const quotaLoading = ref(false)
  const quotaError = ref('')
  const liveQuotas = ref<Quota[] | null>(null)
  const quotaEstimates = ref<QuotaEstimate[]>([])
  let uiActive = false
  let disposed = false
  let stopLifecycle: (() => void) | undefined
  let dataRefreshTimer: number | undefined
  let quotaRefreshTimer: number | undefined
  let syncPollTimer: number | undefined
  let quotaSettleTimer: number | undefined
  let lastQuotaRefresh = 0
  let dashboardRequest = 0
  let quotaRequest = 0
  const requestRows = ref<RequestRow[]>([])
  const requestLoading = ref(false)
  const requestError = ref('')
  const requestPage = ref(1)
  const requestPageSize = ref(100)
  const requestTotal = ref(0)
  const requestPageCount = ref(1)
  const requestSnapshotMaxId = ref<number | null>(null)
  let appliedRequestFilters: RequestFilters | null = null
  let requestQuery = 0
  const sessionRows = ref<SessionRow[]>([])
  const sessionLoading = ref(false)
  const sessionError = ref('')
  const requestTool = ref('')
  const requestSearch = ref('')
  const requestSince = ref(defaultDate(-7))
  const requestUntil = ref(defaultDate(0))
  const previewSettings = ref<PreviewSettings>({ ...defaultPreviewSettings })
  const previewSettingsLoading = ref(false)
  const previewSettingsSaving = ref(false)
  const previewSettingsMessage = ref('')
  const previewSettingsError = ref('')
  const previewSample = ref<PreviewData | null>(null)
  const quotaNotificationRules = ref<Record<string, QuotaSubscriptionNotificationDraft>>({})
  const availableQuotaSubscriptions = ref<Array<{ key: string; quota: Quota }>>([])
  const legacyQuotaNotificationEnabled = ref(false)
  const legacyQuotaNotificationThresholds = ref<number[]>([50, 10, 0])
  const quotaNotificationSettingsLoaded = ref(false)
  const quotaNotificationSaving = ref(false)
  const quotaNotificationLoading = ref(false)
  const quotaNotificationMessage = ref('')
  const quotaNotificationError = ref('')
  let quotaNotificationTimer: number | undefined

  const report = computed(() => payload.value?.report ?? null)
  const selectedWindow = computed<UsageWindow | null>(() => report.value?.[period.value] ?? null)
  const summary = computed(() => selectedWindow.value?.summary ?? null)
  const toolRows = computed(() => selectedWindow.value?.breakdown.tools ?? [])
  const historyRows = computed(() => report.value?.heatmap.slice(-trendDays.value) ?? [])
  const trendMax = computed(() => Math.max(1, ...historyRows.value.map((cell) => cell.total_tokens)))
  const hourMax = computed(() => Math.max(1, ...(report.value?.hourly.map((cell) => cell.total_tokens) ?? [])))
  const recentSessions = computed(() => [...(report.value?.recent_sessions ?? [])].sort((a, b) => b.cost - a.cost))
  const requestTools = computed(() => report.value?.sources ?? [])
  const accounts = computed(() => payload.value?.accounts ?? [])
  const selectedAccount = computed(() => accounts.value.find((account) => account.selection === accountScope.value) ?? null)
  const quotaRows = computed(() => {
    const cached = report.value?.quotas ?? []
    if (liveQuotas.value === null) return cached
    const live = liveQuotas.value
    return [...cached.filter((row) => row.origin !== 'probe' && !(row.origin === 'log'
      && live.some((quota) => quota.tool === row.tool && quota.window_minutes === row.window_minutes))), ...live]
  })
  const quotaNotificationSources = computed(() => {
    const current = new Map(quotaRows.value
      .filter((quota) => quota.origin === 'probe')
      .map((quota) => [quotaNotificationIdentity(quota), quota] as const))
    const available = availableQuotaSubscriptions.value.length
      ? availableQuotaSubscriptions.value
      : quotaRows.value
          .filter((quota) => quota.origin === 'probe')
          .map((quota) => ({ key: quotaNotificationIdentity(quota), quota }))
    return available.map(({ key, quota }) => ({ key, quota: current.get(key) ?? quota }))
  })
  const quotaNotificationRows = computed(() => quotaNotificationSources.value
    .map(({ key, quota }) => ({ key, quota, rule: quotaNotificationRules.value[key] }))
    .filter((row) => row.rule !== undefined) as Array<{
      key: string
      quota: Quota
      rule: QuotaSubscriptionNotificationDraft
    }>)
  const anyQuotaNotificationEnabled = computed(() => legacyQuotaNotificationEnabled.value
    || Object.values(quotaNotificationRules.value).some((rule) => rule.enabled || rule.reset_enabled))
  const detectedCount = computed(() => report.value?.sources.filter((source) => source.detected).length ?? 0)
  const sourceCatalog = computed(() => payload.value?.supported_tools.map((tool) => {
    const status = report.value?.sources.find((source) => source.id === tool.id)
    return { ...tool, detected: status?.detected ?? false, hint: status?.hint ?? null }
  }) ?? [])
  const updatedText = computed(() => {
    const stamp = payload.value?.last_ingest_ms ?? report.value?.generated_at_ms
    if (!stamp) return '等待首次扫描'
    return `更新于 ${formatTime(stamp)}`
  })
  const tokenShareTotal = computed(() => Math.max(1, ...toolRows.value.map((tool) => tool.total_tokens)))

  function defaultDate(offset: number): string {
    const date = new Date()
    date.setDate(date.getDate() + offset)
    const year = date.getFullYear()
    const month = String(date.getMonth() + 1).padStart(2, '0')
    const day = String(date.getDate()).padStart(2, '0')
    return `${year}-${month}-${day}`
  }

  function dateStart(value: string): number {
    return new Date(`${value}T00:00:00`).getTime()
  }

  function dateEnd(value: string): number {
    return dateStart(value) + 86_400_000 - 1
  }

  function sessionPeriodBounds(): { since_ms: number; until_ms: number } {
    const now = new Date()
    const start = new Date(now)
    start.setHours(0, 0, 0, 0)
    if (period.value === 'week') {
      start.setDate(start.getDate() - ((start.getDay() + 6) % 7))
    } else if (period.value === 'month') {
      start.setDate(1)
    } else if (period.value === 'year') {
      start.setMonth(0, 1)
    }
    return { since_ms: start.getTime(), until_ms: now.getTime() }
  }

  function formatTokens(value: number, compact = true): string {
    if (!Number.isFinite(value)) return '0'
    if (!compact) return Math.round(value).toLocaleString('zh-CN')
    if (Math.abs(value) >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`
    if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
    if (Math.abs(value) >= 10_000) return `${(value / 1_000).toFixed(1)}K`
    return Math.round(value).toLocaleString('zh-CN')
  }

  function formatMoney(value: number, priced = true): string {
    if (!priced) return '未计价'
    if (Math.abs(value) < 0.01 && value !== 0) return '<$0.01'
    return new Intl.NumberFormat('zh-CN', { style: 'currency', currency: 'USD', maximumFractionDigits: 2 }).format(value)
  }

  function formatRequestCost(value: number): string {
    if (!Number.isFinite(value)) return '—'
    return `$${Math.max(0, value).toFixed(6)}`
  }

  function formatDuration(value: number | null): string {
    if (value === null || !Number.isFinite(value) || value < 0) return '—'
    if (value < 1_000) return `${Math.round(value)} 毫秒`
    const seconds = value / 1_000
    if (seconds < 60) return `${seconds.toFixed(1)} 秒`
    const minutes = Math.floor(seconds / 60)
    const remainder = Math.round(seconds % 60)
    if (minutes < 60) return remainder ? `${minutes} 分 ${remainder} 秒` : `${minutes} 分钟`
    const hours = Math.floor(minutes / 60)
    const remainingMinutes = minutes % 60
    return remainingMinutes ? `${hours} 小时 ${remainingMinutes} 分` : `${hours} 小时`
  }

  function tokenBreakdownTitle(row: RequestRow): string {
    return [
      `输入：${formatTokens(row.counts.input, false)}`,
      `缓存写入：${formatTokens(row.counts.cache_creation, false)}`,
      `缓存读取：${formatTokens(row.counts.cache_read, false)}`,
      `输出：${formatTokens(row.counts.output, false)}`,
      `其中推理：${formatTokens(row.counts.reasoning, false)}`,
      `总 Token：${formatTokens(row.total_tokens, false)}`,
      `Credits：${formatTokens(row.counts.credits, false)}`
    ].join('\n')
  }

  function formatTime(value: number): string {
    return new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(value)
  }

  function formatDate(value: number): string {
    return new Intl.DateTimeFormat('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit' }).format(value)
  }

  function percent(value: number): string {
    return `${Math.max(0, Math.min(100, value)).toFixed(value >= 10 ? 0 : 1)}%`
  }

  function toolName(id: string): string {
    return report.value?.sources.find((source) => source.id === id)?.display ?? id.replaceAll('_', ' ')
  }

  function toolInitial(id: string): string {
    return toolName(id).trim().slice(0, 1).toUpperCase() || 'A'
  }

  function quotaWindow(quota: Quota): string {
    if (quota.label) return quota.label
    const minutes = quota.window_minutes
    if (minutes >= 10_080) return `${Math.round(minutes / 10_080)} 周窗口`
    if (minutes >= 1_440) return `${Math.round(minutes / 1_440)} 天窗口`
    return `${Math.round(minutes / 60)} 小时窗口`
  }

  function hasPreviewWidget(id: PreviewWidget): boolean {
    return previewSettings.value.widgets.includes(id)
  }

  function togglePreviewWidget(id: PreviewWidget) {
    const next = new Set(previewSettings.value.widgets)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    previewSettings.value.widgets = previewWidgetOptions
      .map((option) => option.id)
      .filter((widget) => next.has(widget))
    previewSettingsMessage.value = ''
  }

  async function loadPreviewSettings() {
    previewSettingsLoading.value = true
    previewSettingsError.value = ''
    try {
      const sample = await pluginCall<PreviewData>('get_preview')
      const saved = sample.settings
      previewSettings.value = {
        widgets: previewWidgetOptions.map((option) => option.id).filter((id) => saved.widgets?.includes(id)),
        display_mode: saved.display_mode === 'compact' ? 'compact' : 'cards'
      }
      previewSample.value = sample
    } catch (reason) {
      previewSettingsError.value = errorMessage(reason)
    } finally {
      previewSettingsLoading.value = false
    }
  }

  async function savePreviewSettings() {
    previewSettingsSaving.value = true
    previewSettingsMessage.value = ''
    previewSettingsError.value = ''
    try {
      const saved = await pluginCall<PreviewSettings>('set_preview_settings', previewSettings.value)
      previewSettings.value = saved
      previewSettingsMessage.value = '悬停预览设置已保存'
    } catch (reason) {
      previewSettingsError.value = errorMessage(reason)
    } finally {
      previewSettingsSaving.value = false
    }
  }

  function resetPreviewSettings() {
    previewSettings.value = { ...defaultPreviewSettings, widgets: [...defaultPreviewSettings.widgets] }
    previewSettingsMessage.value = ''
  }

  function quotaOrigin(quota: Quota): string {
    if (quota.origin === 'budget') return '自定义预算'
    if (quota.origin === 'log') return '本地日志'
    return '实时读取'
  }

  function accountLabel(account: DashboardPayload['accounts'][number]): string {
    return toolName(account.tool) + ' · ' + (account.name || account.label)
  }
  function editAccountName() {
    if (!selectedAccount.value || !selectedAccount.value.selection.startsWith('account:')) return
    accountNameDraft.value = selectedAccount.value.name ?? ''
    accountNameError.value = ''
    accountNameEditing.value = true
  }

  async function saveAccountName() {
    const account = selectedAccount.value
    if (!account || !account.selection.startsWith('account:')) return
    accountNameSaving.value = true
    accountNameError.value = ''
    try {
      const result = await pluginCall<{ name: string | null }>('set_account_name', {
        selection: account.selection,
        name: accountNameDraft.value
      }, 60_000)
      account.name = result.name
      accountNameEditing.value = false
    } catch (reason) {
      accountNameError.value = errorMessage(reason)
    } finally {
      accountNameSaving.value = false
    }
  }

  function resetLabel(timestamp: number): string {
    return resetCountdown(timestamp)
  }

  function quotaEstimate(quota: Quota) {
    return quotaEstimates.value.find((estimate) => estimate.key === quotaNotificationIdentity(quota))
  }

  function previewQuotaEstimate(quota: Quota) {
    return previewSample.value?.quota_estimates?.find((estimate) => estimate.key === quotaNotificationIdentity(quota))
  }

  function quotaNotificationIdentity(quota: Quota): string {
    return `${quota.tool}|${quota.window_minutes}|${quota.id ?? quota.label ?? 'default'}`
  }

  function ensureQuotaNotificationRules() {
    if (!quotaNotificationSettingsLoaded.value) return
    const next = { ...quotaNotificationRules.value }
    let changed = false
    for (const { key } of quotaNotificationSources.value) {
      if (next[key]) continue
      const thresholds = legacyQuotaNotificationEnabled.value
        ? legacyQuotaNotificationThresholds.value
        : [50, 10, 0]
      next[key] = {
        enabled: legacyQuotaNotificationEnabled.value,
        reset_enabled: false,
        thresholdsText: thresholds.join(',')
      }
      changed = true
    }
    if (changed) quotaNotificationRules.value = next
  }

  watch(
    [quotaNotificationSources, legacyQuotaNotificationEnabled, legacyQuotaNotificationThresholds, quotaNotificationSettingsLoaded],
    ensureQuotaNotificationRules,
    { immediate: true, flush: 'sync' }
  )

  function checkQuotaNotifications(): Promise<{ notifications_sent: number }> {
    return pluginCall<{ notifications_sent: number }>('check_quota_notifications', {}, 8_000)
  }

  function syncQuotaNotificationPolling() {
    if (quotaNotificationTimer !== undefined) {
      window.clearInterval(quotaNotificationTimer)
      quotaNotificationTimer = undefined
    }
    if (disposed || !anyQuotaNotificationEnabled.value) return
    quotaNotificationTimer = window.setInterval(() => {
      if (uiActive && document.visibilityState === 'visible') void checkQuotaNotifications().catch(() => {})
    }, 60_000)
  }

  async function loadQuotaNotificationSettings() {
    quotaNotificationLoading.value = true
    quotaNotificationError.value = ''
    try {
      const settings = await pluginCall<QuotaNotificationSettings>('get_quota_notification_settings')
      availableQuotaSubscriptions.value = settings.available_subscriptions ?? []
      quotaNotificationRules.value = Object.fromEntries(
        Object.entries(settings.subscriptions ?? {}).map(([key, rule]) => [key, {
          enabled: rule.enabled,
          reset_enabled: rule.reset_enabled ?? false,
          thresholdsText: rule.remaining_thresholds_percent.join(',')
        }])
      )
      legacyQuotaNotificationEnabled.value = settings.enabled
      legacyQuotaNotificationThresholds.value = settings.remaining_thresholds_percent
      ensureQuotaNotificationRules()
      syncQuotaNotificationPolling()
    } catch (reason) {
      quotaNotificationError.value = errorMessage(reason)
    } finally {
      quotaNotificationSettingsLoaded.value = true
      ensureQuotaNotificationRules()
      quotaNotificationLoading.value = false
    }
  }

  async function saveQuotaNotificationSettings() {
    ensureQuotaNotificationRules()
    const subscriptions: Record<string, QuotaSubscriptionNotificationRule> = {}
    for (const [key, rule] of Object.entries(quotaNotificationRules.value)) {
      const parts = rule.thresholdsText.split(',').map((value) => value.trim())
      const thresholds = parts.map(Number)
      if (
        parts.length < 1 || parts.length > 10 ||
        parts.some((value) => !/^\d+$/.test(value)) ||
        thresholds.some((value) => !Number.isInteger(value) || value < 0 || value > 100)
      ) {
        quotaNotificationError.value = '\u8bf7\u4e3a\u6bcf\u4e2a\u8ba2\u9605\u586b\u5199 1 \u5230 10 \u4e2a 0 \u5230 100 \u4e4b\u95f4\u7684\u6574\u6570\u9608\u503c\uff0c\u7528\u9017\u53f7\u5206\u9694\u3002'
        quotaNotificationMessage.value = ''
        return
      }
      subscriptions[key] = {
        enabled: rule.enabled,
        reset_enabled: rule.reset_enabled,
        remaining_thresholds_percent: [...new Set(thresholds)].sort((a, b) => b - a)
      }
    }
    quotaNotificationSaving.value = true
    quotaNotificationMessage.value = ''
    quotaNotificationError.value = ''
    try {
      const settings = await pluginCall<QuotaNotificationSettings>('set_quota_notification_settings', {
        subscriptions
      })
      quotaNotificationRules.value = Object.fromEntries(
        Object.entries(settings.subscriptions).map(([key, rule]) => [key, {
          enabled: rule.enabled,
          reset_enabled: rule.reset_enabled ?? false,
          thresholdsText: rule.remaining_thresholds_percent.join(',')
        }])
      )
      legacyQuotaNotificationEnabled.value = settings.enabled
      legacyQuotaNotificationThresholds.value = settings.remaining_thresholds_percent
      ensureQuotaNotificationRules()
      syncQuotaNotificationPolling()
      quotaNotificationMessage.value = '\u5df2\u4fdd\u5b58\u5404\u8ba2\u9605\u901a\u77e5\u8bbe\u7f6e'
      if (anyQuotaNotificationEnabled.value) {
        try {
          const result = await checkQuotaNotifications()
          if (result.notifications_sent > 0) quotaNotificationMessage.value = '\u5df2\u4fdd\u5b58\uff0c\u5f53\u524d\u8ba2\u9605\u989d\u5ea6\u5df2\u89e6\u53d1\u63d0\u9192'
        } catch (reason) {
          quotaNotificationError.value = `\u8bbe\u7f6e\u5df2\u4fdd\u5b58\uff0c\u4f46\u5bbf\u4e3b\u901a\u77e5\u5931\u8d25\uff1a${errorMessage(reason)}`
        }
      }
    } catch (reason) {
      quotaNotificationError.value = errorMessage(reason)
    } finally {
      quotaNotificationSaving.value = false
    }
  }

  function readDashboard(): Promise<void> {
    const request = ++dashboardRequest
    const selection = { scope: scope.value, account: accountScope.value }
    loading.value = true
    error.value = ''
    return pluginCall<DashboardPayload>('dashboard', selection, 60_000)
      .then((value) => {
        if (disposed || request !== dashboardRequest || selection.scope !== scope.value || selection.account !== accountScope.value) return
        payload.value = value
        quotaEstimates.value = value.quota_estimates ?? []
        if (accountScope.value !== 'all' && !value.accounts.some((option) => option.selection === accountScope.value)) {
          accountScope.value = 'all'
        }
      })
      .catch((reason) => {
        if (request === dashboardRequest) error.value = errorMessage(reason)
      })
      .finally(() => {
        if (request === dashboardRequest) loading.value = false
      })
  }

  async function finishDataSync(status: SyncStatus) {
    syncing.value = false
    syncWarning.value = status.warning ?? ''
    await readDashboard()
    if (page.value === 'requests' && !requestLoading.value) await fetchRequests(requestPage.value === 1)
    if (page.value === 'sessions') await fetchSessions()
    if (page.value === 'preview') await loadPreviewSettings()
  }

  async function pollDataSync() {
    syncPollTimer = undefined
    if (disposed || !uiActive) return
    try {
      const status = await pluginCall<SyncStatus>('get_sync_status')
      if (disposed) return
      if (status.running) syncPollTimer = window.setTimeout(() => void pollDataSync(), 1_000)
      else await finishDataSync(status)
    } catch (reason) {
      syncing.value = false
      syncWarning.value = errorMessage(reason)
    }
  }

  async function refresh(force = true): Promise<void> {
    if (syncing.value || disposed) return
    syncing.value = true
    syncWarning.value = ''
    try {
      const status = await pluginCall<SyncStatus>('sync_data', { force })
      if (disposed) return
      if (status.running) {
        syncPollTimer = window.setTimeout(() => void pollDataSync(), 1_000)
      } else await finishDataSync(status)
    } catch (reason) {
      syncing.value = false
      syncWarning.value = errorMessage(reason)
    }
  }

  async function readQuotas(refresh: boolean, force = false) {
    const request = ++quotaRequest
    const selection = { scope: scope.value, account: accountScope.value }
    quotaLoading.value = true
    quotaError.value = ''
    try {
      const result = await pluginCall<{ quotas: Quota[]; quota_estimates?: QuotaEstimate[] }>('get_quotas', { ...selection, refresh, force }, 8_000)
      if (disposed || request !== quotaRequest || selection.scope !== scope.value || selection.account !== accountScope.value) return
      liveQuotas.value = result.quotas
      quotaEstimates.value = result.quota_estimates ?? []
      availableQuotaSubscriptions.value = result.quotas.filter((quota) => quota.origin === 'probe')
        .map((quota) => ({ key: quotaNotificationIdentity(quota), quota }))
    } catch (reason) {
      if (request === quotaRequest) quotaError.value = errorMessage(reason)
    } finally {
      if (request === quotaRequest) quotaLoading.value = false
    }
  }

  async function refreshQuotas(force = true) {
    if (quotaLoading.value || disposed) return
    if (quotaSettleTimer !== undefined) window.clearTimeout(quotaSettleTimer)
    lastQuotaRefresh = Date.now()
    await readQuotas(true, force)
    // Slow providers finish into their cache after the two-second response
    // budget. Pick those answers up without starting another network probe.
    if (!disposed) quotaSettleTimer = window.setTimeout(() => {
      if (uiActive && !quotaLoading.value) void readQuotas(false)
    }, 30_000)
  }

  async function fetchRequests(reset = true, nextPage = requestPage.value) {
    if ((reset || !appliedRequestFilters) && (!requestSince.value || !requestUntil.value)) {
      requestError.value = '请选择开始日期和结束日期'
      return
    }
    const query = ++requestQuery
    const freshQuery = reset || !appliedRequestFilters
    const filters: RequestFilters = freshQuery ? {
        since_ms: dateStart(requestSince.value),
        until_ms: dateEnd(requestUntil.value),
        scope: scope.value,
        account: accountScope.value,
        tool: requestTool.value,
        search: requestSearch.value.trim()
      } : { ...appliedRequestFilters! }
    if (freshQuery) nextPage = 1
    const snapshotMaxId = freshQuery ? null : requestSnapshotMaxId.value
    requestLoading.value = true
    requestError.value = ''
    try {
      const result = await pluginCall<{
        rows: RequestRow[]
        refresh_warning: string | null
        total: number
        page: number
        page_size: number
        total_pages: number
        snapshot_max_id: number
      }>('requests', {
        ...filters,
        page: nextPage,
        limit: requestPageSize.value,
        ...(snapshotMaxId === null ? {} : { snapshot_max_id: snapshotMaxId })
      }, 60_000)
      if (disposed || query !== requestQuery) return
      appliedRequestFilters = filters
      requestRows.value = result.rows
      requestPage.value = result.page
      requestTotal.value = result.total
      requestPageCount.value = result.total_pages
      requestSnapshotMaxId.value = result.snapshot_max_id
      if (result.refresh_warning) requestError.value = result.refresh_warning
    } catch (reason) {
      if (query === requestQuery) requestError.value = errorMessage(reason)
    } finally {
      if (query === requestQuery) requestLoading.value = false
    }
  }

  function goRequestPage(next: number) {
    if (requestLoading.value || !Number.isInteger(next) || next < 1 || next > requestPageCount.value) return
    return fetchRequests(false, next)
  }

  function resetRequestPagination() {
    requestQuery += 1
    requestPage.value = 1
    requestTotal.value = 0
    requestPageCount.value = 1
    requestSnapshotMaxId.value = null
    appliedRequestFilters = null
    requestRows.value = []
    requestLoading.value = false
  }

  async function fetchSessions() {
    sessionLoading.value = true
    sessionError.value = ''
    try {
      const result = await pluginCall<{
        rows: SessionRow[]
        refresh_warning: string | null
      }>('sessions', {
        ...sessionPeriodBounds(),
        scope: scope.value,
        account: accountScope.value,
        limit: 200
      }, 60_000)
      sessionRows.value = result.rows
      if (result.refresh_warning) sessionError.value = result.refresh_warning
    } catch (reason) {
      sessionError.value = errorMessage(reason)
    } finally {
      sessionLoading.value = false
    }
  }

  async function saveDataRoots(next: Array<{ id: string; path: string }>) {
    rootsSaving.value = true
    rootsError.value = ''
    try {
      const roots = await pluginCall<Array<{ id: string; path: string }>>('set_data_roots', { roots: next }, 60_000)
      if (payload.value) payload.value.data_roots = roots
      const previousScope = scope.value
      if (scope.value.startsWith('origin:') && !roots.some((root) => `origin:${root.id}` === scope.value)) scope.value = 'all'
      if (scope.value === previousScope) await refresh()
    } catch (reason) {
      rootsError.value = errorMessage(reason)
    } finally {
      rootsSaving.value = false
    }
  }

  function addDataRoot() {
    const path = rootPathDraft.value.trim()
    if (!path) return
    const roots = [...(payload.value?.data_roots ?? []), { id: '', path }]
    rootPathDraft.value = ''
    void saveDataRoots(roots)
  }

  function removeDataRoot(id: string) {
    void saveDataRoots((payload.value?.data_roots ?? []).filter((root) => root.id !== id))
  }

  function selectPage(next: PageKey) {
    page.value = next
    if (next === 'requests') void fetchRequests(false)
    if (next === 'sessions') void fetchSessions()
    if (next === 'preview') void loadPreviewSettings()
  }

  function refreshCurrent() {
    return refresh()
  }

  watch([requestTool], () => {
    if (page.value === 'requests') void fetchRequests()
    else resetRequestPagination()
  })

  watch(requestPageSize, () => {
    resetRequestPagination()
    if (page.value === 'requests') void fetchRequests()
  })

  watch(period, () => {
    if (page.value === 'sessions') void fetchSessions()
  })

  watch([scope, accountScope], async () => {
    resetRequestPagination()
    liveQuotas.value = null
    quotaEstimates.value = []
    if (payload.value) await readDashboard()
    void readQuotas(false)
    if (page.value === 'requests') await fetchRequests()
    if (page.value === 'sessions') await fetchSessions()
  })

  onMounted(() => {
    stopLifecycle = onPluginLifecycle((active) => {
      uiActive = active
      if (!active) return
      if (syncing.value && syncPollTimer === undefined) void pollDataSync()
      else if (payload.value) void refresh(false)
      if (payload.value && Date.now() - lastQuotaRefresh >= QUOTA_REFRESH_MS) void refreshQuotas(false)
    })
    void readDashboard().then(() => {
      if (disposed) return
      void refresh(false)
      void refreshQuotas(false)
    })
    void loadQuotaNotificationSettings()
    dataRefreshTimer = window.setInterval(() => {
      if (uiActive && !loading.value) void refresh(true)
    }, DATA_REFRESH_MS)
    quotaRefreshTimer = window.setInterval(() => {
      if (uiActive) void refreshQuotas(true)
    }, QUOTA_REFRESH_MS)
  })
  onUnmounted(() => {
    disposed = true
    stopLifecycle?.()
    if (quotaNotificationTimer !== undefined) window.clearInterval(quotaNotificationTimer)
    if (dataRefreshTimer !== undefined) window.clearInterval(dataRefreshTimer)
    if (quotaRefreshTimer !== undefined) window.clearInterval(quotaRefreshTimer)
    if (syncPollTimer !== undefined) window.clearTimeout(syncPollTimer)
    if (quotaSettleTimer !== undefined) window.clearTimeout(quotaSettleTimer)
  })
  return {
    pages, previewWidgetOptions, defaultPreviewSettings, periodOptions, trendOptions,
    page, scope, accountScope, accountNameEditing, accountNameSaving, accountNameDraft,
    accountNameError, rootPathDraft, rootsSaving, rootsError, period, trendDays,
    payload, loading, error, syncing, syncWarning, quotaLoading, quotaError, refreshQuotas, requestRows, requestLoading, requestError, sessionRows,
    sessionLoading, sessionError, requestTool, requestSearch, requestSince, requestUntil,
    requestPage, requestPageSize, requestTotal, requestPageCount, goRequestPage,
    previewSettings, previewSettingsLoading, previewSettingsSaving, previewSettingsMessage,
    previewSettingsError, previewSample, report, selectedWindow, summary, toolRows,
    quotaNotificationRows, quotaNotificationSaving,
    quotaNotificationLoading, quotaNotificationMessage, quotaNotificationError,
    historyRows, trendMax, hourMax, recentSessions, requestTools, accounts,
    selectedAccount, quotaRows, detectedCount, sourceCatalog, updatedText, tokenShareTotal,
    defaultDate, dateStart, dateEnd, sessionPeriodBounds, formatTokens, formatMoney,
    formatRequestCost, formatDuration, tokenBreakdownTitle, formatTime, formatDate, percent,
    toolName, toolInitial, quotaWindow, hasPreviewWidget, togglePreviewWidget,
    loadPreviewSettings, savePreviewSettings, resetPreviewSettings, quotaOrigin,
    accountLabel, editAccountName, saveAccountName, resetLabel, resetTimeText, quotaEstimate, previewQuotaEstimate, refresh, fetchRequests,
    fetchSessions, saveDataRoots, addDataRoot, removeDataRoot, selectPage, refreshCurrent,
    saveQuotaNotificationSettings,
  }
}
