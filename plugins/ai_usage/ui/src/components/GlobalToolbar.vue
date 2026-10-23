<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { accounts, accountScope, selectedAccount, accountLabel, editAccountName, scope, payload, updatedText, loading, requestLoading, refreshCurrent } = inject(aiUsageKey)!
</script>

<template>
  <div class="top-actions global-toolbar">
    <div v-if="accounts.length" class="account-filter-control">
      <label class="scope-select account-select"><span>账号 / 配置</span><select v-model="accountScope">
        <option value="all">全部账号</option>
        <option v-for="account in accounts" :key="account.selection" :value="account.selection">{{ accountLabel(account) }} · {{ account.sessions }} 会话</option>
      </select></label>
      <button v-if="selectedAccount?.selection.startsWith('account:')" class="account-name-edit" type="button" title="编辑当前账号或配置名称" aria-label="编辑当前账号或配置名称" @click="editAccountName">✎</button>
    </div>
    <label class="scope-select"><span>来源</span><select v-model="scope">
      <option value="all">全部</option>
      <option value="local">本机</option>
      <option v-for="root in payload?.data_roots ?? []" :key="root.id" :value="`origin:${root.id}`">{{ root.path }}</option>
    </select></label>
    <span class="updated-label"><span class="sync-dot" :class="{ spinning: loading || requestLoading }"></span>{{ updatedText }}</span>
    <button class="refresh-button global-refresh" title="刷新数据" aria-label="刷新数据" :disabled="loading || requestLoading" @click="refreshCurrent">
      <span class="refresh-symbol" :class="{ spinning: loading || requestLoading }">↻</span>
    </button>
  </div>
</template>
