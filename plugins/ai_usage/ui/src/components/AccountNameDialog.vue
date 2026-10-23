<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { accountNameEditing, accountNameSaving, accountNameDraft, accountNameError, saveAccountName } = inject(aiUsageKey)!
</script>

<template>
  <div v-if="accountNameEditing" class="account-name-backdrop" role="presentation" @click.self="accountNameEditing = false">
    <section class="panel account-name-dialog" role="dialog" aria-modal="true" aria-labelledby="account-name-title">
      <div class="settings-heading"><div><div class="section-kicker">ACCOUNT / PROFILE</div><h3 id="account-name-title">编辑账号或配置名称</h3></div><button class="account-name-close" type="button" aria-label="关闭" @click="accountNameEditing = false">×</button></div>
      <p class="account-name-help">展示名只保存在本机，用于区分统计里的 账号 / 配置；清空后恢复显示账号 ID 后缀。</p>
      <label class="account-name-field"><span>展示名</span><input v-model="accountNameDraft" maxlength="48" placeholder="例如：工作账号" @keydown.enter.prevent="saveAccountName" /></label>
      <div v-if="accountNameError" class="account-name-error">{{ accountNameError }}</div>
      <div class="account-name-actions"><button class="text-button" type="button" :disabled="accountNameSaving" @click="accountNameEditing = false">取消</button><button class="primary-button" type="button" :disabled="accountNameSaving" @click="saveAccountName">{{ accountNameSaving ? '保存中…' : '保存名称' }}</button></div>
    </section>
  </div>
</template>
