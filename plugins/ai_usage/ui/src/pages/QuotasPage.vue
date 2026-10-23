<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { quotaRows, toolInitial, toolName, quotaWindow, quotaOrigin, percent, resetLabel, formatTime, refresh, quotaNotificationRows, quotaNotificationSaving, quotaNotificationLoading, quotaNotificationMessage, quotaNotificationError, saveQuotaNotificationSettings } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>订阅配额</h2><p>额度数据来自工具接口、本地日志或你设定的预算。</p></div><span class="subtle-pill">{{ quotaRows.length }} 个额度窗口</span></div>
    <section class="panel quota-alert-settings">
      <div class="quota-alert-heading">
        <div><div class="section-kicker">THRESHOLD ALERT</div><h3>&#x9608;&#x503C;&#x6D88;&#x606F;&#x901A;&#x77E5;</h3><p>&#x6BCF;&#x4E2A;&#x5B9E;&#x65F6;&#x8BA2;&#x9605;&#x53EF;&#x5355;&#x72EC;&#x542F;&#x7528;&#x5E76;&#x8BBE;&#x7F6E;&#x5269;&#x4F59;&#x9608;&#x503C;&#x6863;&#x4F4D;&#x3002;</p></div>
        <button class="primary-button" :disabled="quotaNotificationSaving || quotaNotificationLoading" @click="saveQuotaNotificationSettings"><span v-if="quotaNotificationSaving">&#x4FDD;&#x5B58;&#x4E2D;&#x2026;</span><span v-else>&#x4FDD;&#x5B58;&#x8BBE;&#x7F6E;</span></button>
      </div>
      <div v-if="quotaNotificationRows.length" class="quota-alert-list">
        <article v-for="row in quotaNotificationRows" :key="row.key" class="quota-alert-row">
          <div class="quota-alert-subscription">
            <span class="tool-avatar" :class="`avatar-${row.quota.tool}`">{{ toolInitial(row.quota.tool) }}</span>
            <div><strong>{{ toolName(row.quota.tool) }} &#xB7; {{ quotaWindow(row.quota) }}</strong><small>&#x5269;&#x4F59; {{ percent(100 - row.quota.used_percent) }} &#xB7; {{ resetLabel(row.quota.resets_at_ms) }}</small></div>
          </div>
          <div class="quota-alert-rule">
            <label class="quota-alert-toggle"><input v-model="row.rule.enabled" type="checkbox" :disabled="quotaNotificationLoading || quotaNotificationSaving" /><span>&#x542F;&#x7528;</span></label>
            <label class="quota-alert-threshold"><span>&#x5269;&#x4F59;&#x9608;&#x503C; (%)</span><input v-model="row.rule.thresholdsText" type="text" inputmode="decimal" placeholder="90,50,10,0" :disabled="!row.rule.enabled || quotaNotificationLoading || quotaNotificationSaving" /></label>
          </div>
        </article>
      </div>
      <div v-else class="settings-note"><span v-if="quotaNotificationLoading">&#x8BFB;&#x53D6;&#x8BBE;&#x7F6E;&#x4E2D;&#x2026;</span><span v-else>&#x5F53;&#x524D;&#x6CA1;&#x6709;&#x53EF;&#x914D;&#x7F6E;&#x7684;&#x5B9E;&#x65F6;&#x8BA2;&#x9605;&#x989D;&#x5EA6;&#x3002;</span></div>
      <div class="quota-alert-footer">
        <span v-if="quotaNotificationMessage" class="save-confirmation">{{ quotaNotificationMessage }}</span>
        <span v-else class="settings-note">&#x6BCF;&#x4E2A;&#x8BA2;&#x9605;&#x7A97;&#x53E3;&#x5185;&#xFF0C;&#x6BCF;&#x4E2A;&#x6863;&#x4F4D;&#x53EA;&#x63D0;&#x9192;&#x4E00;&#x6B21;&#x3002;</span>
        <span v-if="quotaNotificationError" class="quota-alert-error">{{ quotaNotificationError }}</span>
      </div>
    </section>
    <div v-if="quotaRows.length" class="quota-grid">
      <article v-for="quota in quotaRows" :key="`${quota.tool}-${quota.id ?? quota.label}`" class="panel quota-card">
        <div class="quota-card-head"><span class="tool-avatar large" :class="`avatar-${quota.tool}`">{{ toolInitial(quota.tool) }}</span><div><strong>{{ toolName(quota.tool) }}</strong><small>{{ quotaWindow(quota) }}</small></div><span class="quota-live"><i></i> {{ quotaOrigin(quota) }}</span></div>
        <div class="quota-value-row"><strong>剩余 {{ percent(100 - quota.used_percent) }}</strong></div>
        <div class="progress-track large-track"><i :class="{ warning: quota.used_percent >= 75, danger: quota.used_percent >= 92 }" :style="{ width: `${Math.max(0, Math.min(100, 100 - quota.used_percent))}%` }"></i></div>
        <div class="quota-card-foot"><span>{{ resetLabel(quota.resets_at_ms) }}</span></div>
        <div class="quota-sampled">最近读取 {{ formatTime(quota.sampled_at_ms) }}</div>
      </article>
    </div>
    <div v-else class="panel empty-state quota-empty"><span>◷</span><strong>没有可展示的实时额度</strong><small>这不影响本地 Token、费用估算和会话统计。部分工具不会向本地客户端提供订阅窗口数据。</small><button class="refresh-button" @click="refresh">重新读取</button></div>
  </section>
</template>
