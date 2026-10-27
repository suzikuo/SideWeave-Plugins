<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { quotaRows, toolInitial, toolName, quotaWindow, quotaOrigin, percent, resetLabel, resetTimeText, quotaEstimate, formatMoney, formatTime, refreshQuotas, quotaLoading, quotaError, quotaNotificationRows, quotaNotificationSaving, quotaNotificationLoading, quotaNotificationMessage, quotaNotificationError, saveQuotaNotificationSettings } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>订阅配额</h2><p>插件启用时后台每 5 分钟独立刷新；统一账号使用选定的登录来源，手动刷新不扫描用量日志。</p></div><button class="refresh-button" :disabled="quotaLoading" @click="refreshQuotas()">{{ quotaLoading ? '正在读取订阅…' : '刷新订阅' }}</button></div>
    <div v-if="quotaError" class="notice-banner" role="alert">{{ quotaError }}</div>
    <section class="panel quota-alert-settings">
      <div class="quota-alert-heading">
        <div><div class="section-kicker">SUBSCRIPTION ALERT</div><h3>订阅提醒</h3><p>剩余额度与到点重置可独立开启。应用运行且插件启用时，重置提醒在后台检查，关闭详情页仍有效。</p></div>
        <button class="primary-button" :disabled="quotaNotificationSaving || quotaNotificationLoading" @click="saveQuotaNotificationSettings"><span v-if="quotaNotificationSaving">&#x4FDD;&#x5B58;&#x4E2D;&#x2026;</span><span v-else>&#x4FDD;&#x5B58;&#x8BBE;&#x7F6E;</span></button>
      </div>
      <div v-if="quotaNotificationRows.length" class="quota-alert-list">
        <article v-for="row in quotaNotificationRows" :key="row.key" class="quota-alert-row">
          <div class="quota-alert-subscription">
            <span class="tool-avatar" :class="`avatar-${row.quota.tool}`">{{ toolInitial(row.quota.tool) }}</span>
            <div><strong>{{ toolName(row.quota.tool) }} &#xB7; {{ quotaWindow(row.quota) }}</strong><small>&#x5269;&#x4F59; {{ percent(100 - row.quota.used_percent) }} &#xB7; {{ resetLabel(row.quota.resets_at_ms) }}</small></div>
          </div>
          <div class="quota-alert-rule">
            <label class="quota-alert-toggle"><input v-model="row.rule.enabled" type="checkbox" :disabled="quotaNotificationLoading || quotaNotificationSaving" /><span>额度提醒</span></label>
            <label class="quota-alert-threshold"><span>&#x5269;&#x4F59;&#x9608;&#x503C; (%)</span><input v-model="row.rule.thresholdsText" type="text" inputmode="decimal" placeholder="90,50,10,0" :disabled="!row.rule.enabled || quotaNotificationLoading || quotaNotificationSaving" /></label>
            <label class="quota-alert-toggle" title="按工具提供的重置时间提醒；未知时间时不会发送"><input v-model="row.rule.reset_enabled" type="checkbox" :disabled="quotaNotificationLoading || quotaNotificationSaving" /><span>重置时提醒</span></label>
          </div>
        </article>
      </div>
      <div v-else class="settings-note"><span v-if="quotaNotificationLoading">&#x8BFB;&#x53D6;&#x8BBE;&#x7F6E;&#x4E2D;&#x2026;</span><span v-else>&#x5F53;&#x524D;&#x6CA1;&#x6709;&#x53EF;&#x914D;&#x7F6E;&#x7684;&#x5B9E;&#x65F6;&#x8BA2;&#x9605;&#x989D;&#x5EA6;&#x3002;</span></div>
      <div class="quota-alert-footer">
        <span v-if="quotaNotificationMessage" class="save-confirmation">{{ quotaNotificationMessage }}</span>
        <span v-else class="settings-note">额度档位和重置时间每个窗口各提醒一次；重置时间未知时不会发送到点提醒。</span>
        <span v-if="quotaNotificationError" class="quota-alert-error">{{ quotaNotificationError }}</span>
      </div>
    </section>
    <div v-if="quotaRows.length" class="quota-grid">
      <article v-for="quota in quotaRows" :key="`${quota.tool}-${quota.id ?? quota.label}`" class="panel quota-card">
        <div class="quota-card-head"><span class="tool-avatar large" :class="`avatar-${quota.tool}`">{{ toolInitial(quota.tool) }}</span><div><strong>{{ toolName(quota.tool) }}</strong><small>{{ quotaWindow(quota) }}</small></div><span class="quota-live"><i></i> {{ quotaOrigin(quota) }}</span></div>
        <div class="quota-value-row"><strong>剩余 {{ percent(100 - quota.used_percent) }}</strong></div>
        <div class="progress-track large-track"><i :class="{ warning: quota.used_percent >= 75, danger: quota.used_percent >= 92 }" :style="{ width: `${Math.max(0, Math.min(100, 100 - quota.used_percent))}%` }"></i></div>
        <div class="quota-card-foot quota-reset-details"><strong>{{ resetTimeText(quota.resets_at_ms) }}</strong><span>{{ resetLabel(quota.resets_at_ms) }}</span></div>
        <div class="quota-estimate-box">
          <template v-if="quotaEstimate(quota)?.total_cost_usd != null">
            <div><span>预估满额可用金额</span><strong>≈ {{ formatMoney(quotaEstimate(quota)!.total_cost_usd!) }}</strong></div>
            <small>本窗口已用 {{ formatMoney(quotaEstimate(quota)!.used_cost_usd!) }} · 预估剩余 {{ formatMoney(quotaEstimate(quota)!.remaining_cost_usd!) }}</small>
          </template>
          <span v-else>{{ quotaEstimate(quota)?.reason ?? '等待窗口费用数据' }}</span>
          <small v-if="quotaEstimate(quota)?.coverage_note">{{ quotaEstimate(quota)!.coverage_note }}</small>
          <small v-if="quotaEstimate(quota)?.speed_note">{{ quotaEstimate(quota)!.speed_note }}</small>
        </div>
        <div class="quota-sampled">最近读取 {{ formatTime(quota.sampled_at_ms) }}</div>
      </article>
    </div>
    <div v-else class="panel empty-state quota-empty"><span>◷</span><strong>{{ quotaLoading ? '正在读取订阅额度…' : '没有可展示的实时额度' }}</strong><small>这不影响本地 Token、费用估算和会话统计。部分工具不会向本地客户端提供订阅窗口数据。</small><button class="refresh-button" :disabled="quotaLoading" @click="refreshQuotas()">重新读取订阅</button></div>
    <p class="quota-estimate-note">满额金额 ≈ 窗口内已记录的 API 等值费用 ÷ 已用额度比例。仅为当前账号的近似推算，并非订阅价格或账单；其他设备的未记录用量、模型切换会影响结果。已用不足 1% 或价格缺失时暂不估算。</p>
  </section>
</template>
