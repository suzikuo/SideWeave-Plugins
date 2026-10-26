import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

const file = new URL('../plugins/ai_usage/ui/src/composables/useAiUsage.ts', import.meta.url)
const { code } = transformSync(readFileSync(file, 'utf8'), { loader: 'ts', format: 'cjs' })
const display = { exports: {} }
const displayFile = new URL('../plugins/ai_usage/ui/src/quota-display.ts', import.meta.url)
runInNewContext(transformSync(readFileSync(displayFile, 'utf8'), { loader: 'ts', format: 'cjs' }).code,
  { module: display, exports: display.exports })
const flush = async () => { await setImmediate(); await vue.nextTick() }

async function mount(t) {
  const calls = []
  const intervals = new Map()
  const timers = new Map()
  let timerId = 0
  let lifecycle
  let state
  const responses = {
    dashboard: { accounts: [], report: { quotas: [], sources: [] }, last_ingest_ms: 1 },
    sync_data: { running: true, warning: null },
    get_sync_status: { running: false, warning: null },
    get_quotas: { quotas: [] },
    get_quota_notification_settings: { enabled: false, remaining_thresholds_percent: [50], subscriptions: {} },
    check_quota_notifications: { notifications_sent: 0 },
    requests: { rows: [], refresh_warning: null, total: 0, page: 1, page_size: 100, total_pages: 1, snapshot_max_id: 0 },
    sessions: { rows: [], refresh_warning: null }
  }
  const module = { exports: {} }
  runInNewContext(code, {
    module, exports: module.exports,
    require: (name) => {
      if (name === 'vue') return vue
      if (name === '../quota-display') return display.exports
      return {
        errorMessage: (error) => String(error),
        pluginCall: async (method, payload) => { calls.push({ method, payload }); return typeof responses[method] === 'function' ? responses[method](payload) : responses[method] },
        onPluginLifecycle: (handler) => { lifecycle = handler; handler(false); return () => { lifecycle = () => {} } }
      }
    },
    document: { visibilityState: 'visible' },
    window: {
      setInterval: (callback, delay) => { intervals.set(++timerId, { callback, delay }); return timerId },
      clearInterval: (id) => intervals.delete(id),
      setTimeout: (callback, delay) => { timers.set(++timerId, { callback, delay }); return timerId },
      clearTimeout: (id) => timers.delete(id)
    }
  })
  const renderer = vue.createRenderer({
    createElement: () => ({}), insert: () => {}, remove: () => {}, patchProp: () => {},
    parentNode: () => null, nextSibling: () => null, createText: () => ({}),
    createComment: () => ({}), setText: () => {}, setElementText: () => {}
  })
  const app = renderer.createApp({ setup() {
    state = module.exports.useAiUsage()
    return () => vue.h('div')
  } })
  app.mount({})
  t.after(() => app.unmount())
  await flush()
  return {
    state, calls, responses, intervals, timers,
    activate: (active) => lifecycle(active),
    async finishScan() {
      lifecycle(true)
      const poll = [...timers.entries()].find(([, timer]) => timer.delay === 1000)
      assert.ok(poll)
      timers.delete(poll[0])
      await poll[1].callback()
      await flush()
    },
    unmount: () => app.unmount()
  }
}

test('initial snapshot is available while the first scan is still running', async (t) => {
  const host = await mount(t)
  assert.ok(host.state.payload.value)
  assert.equal(host.state.loading.value, false)
  assert.equal(host.state.syncing.value, true)
  assert.equal(host.calls[0].method, 'dashboard')
  assert.ok(host.calls.findIndex((call) => call.method === 'sync_data') > 0)
  await host.finishScan()
  assert.equal(host.state.syncing.value, false)
  assert.equal(host.calls.filter((call) => call.method === 'dashboard').length, 2)
})

test('data and subscription refresh use separate endpoints and manual quota bypasses TTL', async (t) => {
  const host = await mount(t)
  await host.finishScan()
  host.calls.length = 0
  await host.state.refreshQuotas()
  assert.deepEqual(host.calls.map((call) => call.method), ['get_quotas'])
  assert.equal(host.calls[0].payload.force, true)
  assert.equal(host.calls[0].payload.refresh, true)
  host.calls.length = 0
  await host.state.refreshCurrent()
  assert.deepEqual(host.calls.map((call) => call.method), ['sync_data'])
})

test('60-second data and five-minute quota timers pause on inactive pages', async (t) => {
  const host = await mount(t)
  await host.finishScan()
  host.activate(false)
  host.calls.length = 0
  const dataTimer = [...host.intervals.values()].find((timer) => timer.delay === 60_000)
  const quotaTimer = [...host.intervals.values()].find((timer) => timer.delay === 300_000)
  assert.ok(dataTimer && quotaTimer)
  dataTimer.callback()
  quotaTimer.callback()
  await flush()
  assert.equal(host.calls.length, 0)
  host.activate(true)
  await flush()
  host.calls.length = 0
  quotaTimer.callback()
  await flush()
  assert.deepEqual(host.calls.map((call) => call.method), ['get_quotas'])
  assert.equal(host.calls[0].payload.force, true)
  host.unmount()
  assert.equal(host.intervals.size, 0)
  assert.equal(host.timers.size, 0)
})

test('changing a source or account queries snapshots without starting another scan', async (t) => {
  const host = await mount(t)
  await host.finishScan()
  host.calls.length = 0
  host.state.scope.value = 'local'
  await flush()
  assert.deepEqual(host.calls.map((call) => call.method), ['dashboard', 'get_quotas'])
  assert.equal(host.calls[1].payload.refresh, false)
})

test('reset times show full local dates and unknown or expired times are explicit', () => {
  const now = Date.UTC(2026, 9, 26, 8)
  assert.match(display.exports.resetTimeText(now), /2026/)
  assert.match(display.exports.resetTimeText(now), /GMT/)
  assert.match(display.exports.resetTimeText(0), /未提供/)
  assert.match(display.exports.resetCountdown(0, now), /未知/)
  assert.match(display.exports.resetCountdown(now - 1, now), /已到/)
  assert.match(display.exports.resetCountdown(now + 1, now), /1 分钟/)
})

test('reset reminders can be saved independently of remaining-quota thresholds', async (t) => {
  const host = await mount(t)
  const quota = { tool: 'codex', window_minutes: 300, label: '5 小时', origin: 'probe', used_percent: 50, resets_at_ms: Date.now() + 60_000 }
  host.responses.get_quotas = { quotas: [quota] }
  host.responses.set_quota_notification_settings = (payload) => ({ ...payload, enabled: false, remaining_thresholds_percent: [50] })
  await host.state.refreshQuotas()
  const row = host.state.quotaNotificationRows.value[0]
  assert.ok(row)
  row.rule.enabled = false
  row.rule.reset_enabled = true
  await host.state.saveQuotaNotificationSettings()
  const saved = host.calls.find((call) => call.method === 'set_quota_notification_settings')
  assert.equal(saved.payload.subscriptions[row.key].reset_enabled, true)
  assert.equal(saved.payload.subscriptions[row.key].enabled, false)
  assert.equal(host.state.quotaNotificationRows.value[0].rule.reset_enabled, true)
})

function requestPageResult(payload, session = 'row') {
  return { rows: [{ session }], refresh_warning: null, total: 425,
    page: payload.page, page_size: payload.limit, total_pages: Math.ceil(425 / payload.limit), snapshot_max_id: 500 }
}

test('request navigation preserves applied filters and refresh starts a new result set', async (t) => {
  const host = await mount(t)
  host.responses.requests = requestPageResult
  await host.state.fetchRequests()
  assert.equal(host.state.requestPage.value, 1)
  assert.equal(host.state.requestTotal.value, 425)
  assert.equal(host.state.requestPageCount.value, 5)
  host.state.requestSearch.value = 'unapplied draft'
  await host.state.goRequestPage(2)
  const navigation = host.calls.filter((call) => call.method === 'requests').at(-1).payload
  assert.equal(navigation.page, 2)
  assert.equal(navigation.limit, 100)
  assert.equal(navigation.snapshot_max_id, 500)
  assert.equal(navigation.search, '')
  assert.equal(host.state.requestPage.value, 2)
  await host.state.fetchRequests()
  const refreshed = host.calls.filter((call) => call.method === 'requests').at(-1).payload
  assert.equal(refreshed.page, 1)
  assert.equal(refreshed.search, 'unapplied draft')
  assert.equal(Object.hasOwn(refreshed, 'snapshot_max_id'), false)
})

test('page size and source changes reset request pagination', async (t) => {
  const host = await mount(t)
  host.responses.requests = requestPageResult
  await host.state.fetchRequests()
  await host.state.goRequestPage(3)
  host.state.page.value = 'requests'
  host.state.requestPageSize.value = 50
  await flush()
  assert.equal(host.state.requestPage.value, 1)
  assert.equal(host.state.requestPageCount.value, 9)
  await host.state.goRequestPage(4)
  host.state.scope.value = 'local'
  await flush()
  const query = host.calls.filter((call) => call.method === 'requests').at(-1).payload
  assert.equal(host.state.requestPage.value, 1)
  assert.equal(query.scope, 'local')
  assert.equal(Object.hasOwn(query, 'snapshot_max_id'), false)
})

test('late or failed request queries cannot overwrite a newer page or mislabel old rows', async (t) => {
  const host = await mount(t)
  let releaseOld
  host.responses.requests = () => new Promise((resolve) => { releaseOld = resolve })
  const oldQuery = host.state.fetchRequests()
  host.state.requestSearch.value = 'latest'
  host.responses.requests = (payload) => requestPageResult(payload, 'latest')
  await host.state.fetchRequests()
  releaseOld({ ...requestPageResult({ page: 4, limit: 100 }, 'stale') })
  await oldQuery
  assert.equal(host.state.requestPage.value, 1)
  assert.equal(host.state.requestRows.value[0].session, 'latest')
  host.responses.requests = () => { throw new Error('read failed') }
  await host.state.goRequestPage(2)
  assert.equal(host.state.requestPage.value, 1)
  assert.equal(host.state.requestRows.value[0].session, 'latest')
  assert.match(host.state.requestError.value, /read failed/)
  assert.equal(host.state.requestLoading.value, false)
})
