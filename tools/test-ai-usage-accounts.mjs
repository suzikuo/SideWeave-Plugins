import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import { parse, compileScript } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

const source = readFileSync(new URL('../plugins/ai_usage/ui/src/pages/AccountsPage.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const script = compileScript(descriptor, { id: 'accounts-page-test' })
const { code } = transformSync(script.content, { loader: 'ts', format: 'cjs' })
const flush = async () => { await setImmediate(); await vue.nextTick() }

async function mount(t) {
  const key = Symbol('account-context')
  const calls = []
  let refreshes = 0
  const inventory = {
    identities: [
      { id: 1, tool: 'codex', origin: '', account_id: 'same-account', source_label: 'Windows', name: null, requests: 10, sessions: 2, last_used_ms: 1, group_id: null, quota_available: true },
      { id: 2, tool: 'codex', origin: 'wsl', account_id: 'same-account', source_label: 'WSL', name: null, requests: 20, sessions: 3, last_used_ms: 2, group_id: null, quota_available: true },
      { id: 3, tool: 'claude', origin: '', account_id: 'profile-b', source_label: 'Windows', name: null, requests: 5, sessions: 1, last_used_ms: 3, group_id: null, quota_available: false }
    ],
    groups: [], unknown: [{ tool: 'codex', requests: 42 }]
  }
  let failSave = false
  const module = { exports: {} }
  runInNewContext(code, {
    module, exports: module.exports,
    require: name => name === 'vue' ? vue : name === '../context' ? { aiUsageKey: key } : {
      errorMessage: e => String(e),
      pluginCall: async (method, payload) => {
        calls.push({ method, payload })
        if (method === 'save_account_group') {
          if (failSave) throw new Error('Member already belongs to another group')
          inventory.groups = [{ ...payload, id: payload.id || 'group-1', members: [...payload.members] }]
        }
        if (method === 'delete_account_group') inventory.groups = []
        return JSON.parse(JSON.stringify(inventory))
      }
    }
  })
  const component = module.exports.default
  component.render = () => null
  const renderer = vue.createRenderer({
    createElement: () => ({}), insert: () => {}, remove: () => {}, patchProp: () => {},
    parentNode: () => null, nextSibling: () => null, createText: () => ({}),
    createComment: () => ({}), setText: () => {}, setElementText: () => {}
  })
  const app = renderer.createApp({ setup() {
    vue.provide(key, { toolName: id => id, formatTime: n => String(n), refreshAccountGroups: async () => { refreshes++ } })
    return () => vue.h(component)
  } })
  app.mount({})
  t.after(() => app.unmount())
  await flush()
  const state = app._instance.subTree.component.setupState
  return { state, calls, inventory, refreshes: () => refreshes, failSave: () => { failSave = true } }
}

test('account page loads source identities and keeps unknown history separate', async t => {
  const host = await mount(t)
  assert.equal(host.calls[0].method, 'get_account_groups')
  assert.equal(host.state.data.identities.length, 3)
  assert.equal(host.state.data.unknown[0].requests, 42)
  host.state.reset()
  assert.equal(host.state.candidates.length, 2)
  assert.equal(host.state.selected.length, 0)
  assert.equal(Boolean(host.state.canSave), false)
})

test('saving a unified account sends source-specific members and refreshes reports', async t => {
  const host = await mount(t)
  host.state.reset()
  host.state.draft.name = 'My Codex'
  host.state.draft.members = [1, 2]
  host.state.draft.quota_identity_id = 1
  host.state.reviewing = true
  assert.equal(Boolean(host.state.canSave), true)
  await host.state.save()
  const save = host.calls.find(call => call.method === 'save_account_group')
  assert.deepEqual([...save.payload.members], [1, 2])
  assert.equal(save.payload.quota_identity_id, 1)
  assert.equal(save.payload.tool, 'codex')
  assert.equal(host.refreshes(), 1)
  assert.equal(host.state.editing, false)
  assert.equal(host.state.data.unknown[0].requests, 42)
})

test('editing uses a copy, removing the quota member clears its binding, and dissolve refreshes', async t => {
  const host = await mount(t)
  const group = { id: 'g', name: 'Existing', tool: 'codex', members: [1, 2], quota_identity_id: 1 }
  host.state.reset(group)
  host.state.draft.members = [2]
  assert.deepEqual(group.members, [1, 2])
  await host.state.save()
  const save = host.calls.find(call => call.method === 'save_account_group')
  assert.equal(save.payload.quota_identity_id, null)
  assert.deepEqual([...save.payload.members], [2])
  await host.state.remove(host.state.data.groups[0])
  assert.equal(host.state.data.groups.length, 0)
  assert.equal(host.refreshes(), 2)
  assert.equal(host.state.data.identities.length, 3)
})

test('cross-tool candidates cannot save and server conflicts preserve the editor', async t => {
  const host = await mount(t)
  host.state.reset()
  host.state.draft.name = 'Invalid'
  host.state.draft.members = [3]
  await host.state.save()
  assert.equal(host.calls.filter(call => call.method === 'save_account_group').length, 0)
  host.state.draft.members = [1]
  host.failSave()
  await host.state.save()
  assert.match(host.state.error, /already belongs/)
  assert.equal(host.state.editing, true)
  assert.equal(host.state.busy, false)
  assert.equal(host.refreshes(), 0)
})
