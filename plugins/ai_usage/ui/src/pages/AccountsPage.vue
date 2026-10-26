<script setup lang="ts">
import { computed, inject, onMounted, ref } from 'vue'
import { pluginCall, errorMessage } from '../../../../ui-sdk/runtime'
import { aiUsageKey } from '../context'

interface Identity {
  id: number; tool: string; origin: string; account_id: string; source_label: string
  name: string | null; requests: number; sessions: number; last_used_ms: number
  group_id: string | null; quota_available: boolean
}
interface Group { id: string; tool: string; name: string; members: number[]; quota_identity_id: number | null }
interface Inventory { identities: Identity[]; groups: Group[]; unknown: Array<{ tool: string; requests: number }> }
const { toolName, formatTime, refreshAccountGroups } = inject(aiUsageKey)!
const data = ref<Inventory>({ identities: [], groups: [], unknown: [] })
const busy = ref(false), error = ref(''), message = ref(''), editing = ref(false), reviewing = ref(false)
const draft = ref<Group>({ id: '', tool: '', name: '', members: [], quota_identity_id: null })
const tools = computed(() => [...new Set(data.value.identities.map(i => i.tool))])
const candidates = computed(() => data.value.identities.filter(i => i.tool === draft.value.tool))
const selected = computed(() => candidates.value.filter(i => draft.value.members.includes(i.id)))
const canSave = computed(() => draft.value.name.trim() && selected.value.length && draft.value.members.length === selected.value.length)
function reset(group?: Group) {
  draft.value = group ? { ...group, members: [...group.members] } : { id: '', tool: tools.value[0] ?? '', name: '', members: [], quota_identity_id: null }
  reviewing.value = false; editing.value = true; error.value = ''; message.value = ''
}
function switchTool() { draft.value.members = []; draft.value.quota_identity_id = null; reviewing.value = false }
function memberName(id: number) {
  const i = data.value.identities.find(i => i.id === id)
  return i ? `${i.source_label} · ${i.name || i.account_id}` : `已不可用的来源 #${id}`
}
async function load() {
  busy.value = true; error.value = ''
  try { data.value = await pluginCall<Inventory>('get_account_groups') }
  catch (e) { error.value = errorMessage(e) }
  finally { busy.value = false }
}
async function save() {
  if (!canSave.value || busy.value) return
  busy.value = true; error.value = ''
  try {
    const quota = draft.value.members.includes(draft.value.quota_identity_id ?? -1) ? draft.value.quota_identity_id : null
    data.value = await pluginCall<Inventory>('save_account_group', { ...draft.value, quota_identity_id: quota })
    editing.value = false; reviewing.value = false; message.value = '关联已保存，原始账号与请求数据保持不变。'
    await refreshAccountGroups()
  } catch (e) { error.value = errorMessage(e) }
  finally { busy.value = false }
}
const deleting = ref<string | null>(null)
async function remove(group: Group) {
  busy.value = true; error.value = ''
  try {
    data.value = await pluginCall<Inventory>('delete_account_group', { id: group.id })
    deleting.value = null; editing.value = false; message.value = '统一账号已解散，原始记录仍可按来源账号查询。'
    await refreshAccountGroups()
  } catch (e) { error.value = errorMessage(e) }
  finally { busy.value = false }
}
onMounted(load)
</script>

<template>
  <section class="page-content account-management">
    <div class="page-intro"><div><h2>账号管理</h2><p>将同一工具在 Windows、WSL 和其他机器上的账号关联，统一统计用量。</p></div><button class="primary-button" :disabled="busy || !tools.length" @click="reset()">新建统一账号</button></div>
    <div v-if="error" class="notice-banner" role="alert">{{ error }}</div>
    <div v-if="message" class="notice-banner" role="status">{{ message }}</div>
    <section v-if="editing" class="panel account-editor">
      <h3>{{ draft.id ? '编辑统一账号' : '新建统一账号' }}</h3>
      <template v-if="!reviewing">
        <label>工具<select v-model="draft.tool" :disabled="busy || !!draft.id" @change="switchTool"><option v-for="tool in tools" :key="tool" :value="tool">{{ toolName(tool) }}</option></select></label>
        <label>统一名称<input v-model="draft.name" maxlength="80" :disabled="busy" placeholder="例如：我的 Codex" /></label>
        <div class="account-identity-list">
          <label v-for="identity in candidates" :key="identity.id" class="account-identity">
            <input v-model="draft.members" type="checkbox" :value="identity.id" :disabled="busy || (!!identity.group_id && identity.group_id !== draft.id)" />
            <span><strong>{{ identity.source_label }} · {{ identity.name || '未命名账号' }}</strong><code>{{ identity.account_id }}</code><small>{{ identity.requests.toLocaleString() }} 条请求 · {{ identity.sessions }} 个会话 · {{ identity.last_used_ms ? formatTime(identity.last_used_ms) : '暂无保留期内用量' }}{{ identity.group_id && identity.group_id !== draft.id ? ' · 已加入其他统一账号' : '' }}</small></span>
          </label>
        </div>
        <label>配额查询来源<select v-model="draft.quota_identity_id" :disabled="busy"><option :value="null">暂不绑定订阅</option><option v-for="identity in selected" :key="identity.id" :value="identity.id" :disabled="!identity.quota_available">{{ identity.source_label }} · {{ identity.name || identity.account_id }}{{ identity.quota_available ? '' : '（当前无法验证登录）' }}</option></select></label>
        <p class="subtle">可关联所有工具的用量；目前统一账号订阅查询支持可验证的 Codex 登录来源。其他机器的记录可参与汇总，查询仍使用你选定的登录来源。</p>
      </template>
      <template v-else>
        <h4>确认关联：{{ toolName(draft.tool) }} · {{ draft.name }}</h4>
        <ul><li v-for="identity in selected" :key="identity.id">{{ identity.source_label }} · {{ identity.name || identity.account_id }}</li></ul>
        <p>配额来源：{{ draft.quota_identity_id && draft.members.includes(draft.quota_identity_id) ? memberName(draft.quota_identity_id) : '未绑定' }}</p>
        <p>仅合并已识别账号的查询范围，未知历史不纳入。你可以随时移除成员或解散。</p>
      </template>
      <div class="account-actions"><button class="refresh-button" :disabled="busy" @click="editing = false">取消</button><button v-if="reviewing" class="refresh-button" :disabled="busy" @click="reviewing = false">返回修改</button><button class="primary-button" :disabled="busy || !canSave" @click="reviewing ? save() : reviewing = true">{{ busy ? '保存中…' : reviewing ? '确认保存关联' : '核对关联成员' }}</button></div>
    </section>
    <div v-if="busy && !data.identities.length" class="panel loading-table">正在读取账号来源…</div>
    <section v-for="group in data.groups" :key="group.id" class="panel account-editor">
      <div class="page-intro"><div><h3>{{ group.name }}</h3><p>{{ toolName(group.tool) }} · {{ group.members.length }} 个来源</p></div><button class="refresh-button" :disabled="busy" @click="reset(group)">编辑关联</button></div>
      <ul><li v-for="id in group.members" :key="id">{{ memberName(id) }}</li></ul>
      <p>配额来源：{{ group.quota_identity_id ? memberName(group.quota_identity_id) : '未绑定订阅，仅统一用量统计' }}</p>
      <div class="account-actions"><template v-if="deleting === group.id"><span>确认解散“{{ group.name }}”？历史数据会保留。</span><button class="refresh-button" :disabled="busy" @click="deleting = null">取消</button><button class="refresh-button" :disabled="busy" @click="remove(group)">确认解散</button></template><button v-else class="refresh-button" :disabled="busy" @click="deleting = group.id">解散统一账号</button></div>
    </section>
    <section v-for="tool in tools" :key="tool" class="panel account-editor">
      <h3>{{ toolName(tool) }} · 已识别来源账号</h3>
      <article v-for="identity in data.identities.filter(i => i.tool === tool)" :key="identity.id" class="account-identity"><span><strong>{{ identity.source_label }} · {{ identity.name || '未命名账号' }}</strong><code>{{ identity.account_id }}</code><small>{{ identity.requests.toLocaleString() }} 条请求 · 最近 {{ identity.last_used_ms ? formatTime(identity.last_used_ms) : '暂无用量' }} · {{ identity.group_id ? '已关联' : '尚未关联' }}</small></span></article>
    </section>
    <section class="panel account-editor"><h3>未知账号历史</h3><p>缺少账号标识的请求保持原样，不允许加入统一账号，也不计入订阅满额预估。</p><p v-for="item in data.unknown" :key="item.tool">{{ toolName(item.tool) }} · {{ item.requests.toLocaleString() }} 条未关联请求</p><p v-if="!data.unknown.length">暂无未知账号请求。</p></section>
    <div v-if="!busy && !tools.length" class="panel empty-state"><strong>暂无已识别账号</strong><small>数据同步后，日志中具有明确账号标识的来源会出现在这里。</small></div>
  </section>
</template>

<style scoped>
.account-editor { padding: 18px; display: grid; gap: 14px; }
.account-editor h3, .account-editor h4, .account-editor p { margin: 0; }
.account-editor p, .account-editor li { font-size: 12px; line-height: 1.7; color: var(--text-secondary); overflow-wrap: anywhere; }
.account-editor > label { display: grid; gap: 7px; font-size: 12px; }
.account-editor input:not([type="checkbox"]), .account-editor select { width: 100%; padding: 9px; background: var(--surface); color: var(--text); border: 1px solid var(--border); border-radius: 7px; }
.account-identity-list { display: grid; gap: 8px; }
.account-identity { display: flex; align-items: start; gap: 10px; padding: 12px; border: 1px solid var(--border); border-radius: 8px; }
.account-identity > input { margin-top: 3px; }
.account-identity > span { display: grid; gap: 6px; min-width: 0; }
.account-identity strong { font-size: 12px; overflow-wrap: anywhere; }
.account-identity code, .account-identity small { font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
.account-actions { display: flex; justify-content: end; align-items: center; flex-wrap: wrap; gap: 8px; font-size: 12px; }
</style>
