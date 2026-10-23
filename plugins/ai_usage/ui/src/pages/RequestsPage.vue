<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { requestSince, requestUntil, requestTool, requestTools, requestSearch, fetchRequests, requestError, requestLoading, requestRows, toolInitial, toolName, tokenBreakdownTitle, formatTokens, formatRequestCost, formatDuration, formatTime, percent } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>请求明细</h2><p>显示会话标题、时间、工具、项目、模型和 Token 数量，不读取对话正文。</p></div><span class="subtle-pill">最多显示 200 条</span></div>
    <div class="panel request-filters">
      <label>开始日期<input v-model="requestSince" type="date" /></label>
      <label>结束日期<input v-model="requestUntil" type="date" /></label>
      <label>工具<select v-model="requestTool"><option value="">所有工具</option><option v-for="source in requestTools" :key="source.id" :value="source.id">{{ source.display }}</option></select></label>
      <label class="search-filter">搜索会话标题、项目或模型<input v-model="requestSearch" type="search" placeholder="输入关键词" @keyup.enter="fetchRequests" /></label>
      <button class="primary-button" :disabled="requestLoading" @click="fetchRequests">{{ requestLoading ? '读取中…' : '应用筛选' }}</button>
    </div>
    <div v-if="requestError" class="notice-banner">{{ requestError }}</div>
    <div class="panel table-panel request-table-panel">
      <div v-if="requestLoading && !requestRows.length" class="loading-table">正在读取请求明细…</div>
      <div v-else-if="requestRows.length" class="usage-table request-table">
        <div class="table-head request-columns"><span>请求</span><span>模型</span><span>TOKEN</span><span>费用</span><span>本次耗时</span><span>请求时间</span></div>
        <div v-for="(row, index) in requestRows" :key="`${row.session}-${row.timestamp_ms}-${index}`" class="table-row request-columns">
          <div class="request-origin">
            <div class="request-origin-top"><span class="tool-avatar" :class="`avatar-${row.tool}`">{{ toolInitial(row.tool) }}</span><strong>{{ toolName(row.tool) }}</strong></div>
            <strong class="request-title" :title="row.session_title || row.session">{{ row.session_title || '未命名会话' }}</strong>
            <small class="request-project">{{ row.project || '未标记项目' }} · {{ row.session.slice(0, 16) }}</small>
          </div>
          <div class="request-model-group"><strong class="request-model" :title="row.model || ''">{{ row.model || '未知模型' }}</strong></div>
          <div class="request-token-group">
            <div class="request-token-primary">
              <div class="request-token-flow"><span class="token-down"><i>↓</i>{{ formatTokens(row.counts.input + row.counts.cache_creation + row.counts.cache_read) }}</span><span class="token-up"><i>↑</i>{{ formatTokens(row.counts.output) }}</span></div>
              <button class="token-detail-button" type="button" :title="tokenBreakdownTitle(row)" :aria-label="`查看 ${row.model || '未知模型'} 的详细 Token 统计`">i</button>
            </div>
            <div class="request-cache" :title="`缓存读取 ${formatTokens(row.counts.cache_read, false)} Token`"><span>缓存命中</span><strong>{{ formatTokens(row.counts.cache_read) }}</strong><span>{{ percent(row.counts.cache_read / Math.max(1, row.counts.input + row.counts.cache_creation + row.counts.cache_read) * 100) }}</span></div>
          </div>
          <div class="request-cost-group">
            <strong :class="{ 'cost-unpriced': row.cost_usd === null && row.credits_cost_usd === null }">{{ row.cost_usd !== null ? formatRequestCost(row.cost_usd) : row.credits_cost_usd !== null ? formatRequestCost(row.credits_cost_usd) : '未计价' }}</strong>
            <small v-if="row.cost_usd === null && row.credits_cost_usd !== null">按额度折算</small>
            <small v-else-if="row.cost_usd === null">价格表未匹配</small>
          </div>
          <div class="request-duration-group" :title="row.tool === 'codex' ? undefined : row.duration_ms === null ? '此来源没有记录单条请求用时' : '本条请求的用时。'">
            <strong>{{ formatDuration(row.duration_ms) }}</strong>
            <small v-if="row.tool !== 'codex'">{{ row.duration_ms === null ? '来源未记录' : '本次用时' }}</small>
          </div>
          <div class="request-date-group">{{ formatTime(row.timestamp_ms) }}</div>
        </div>
      </div>
      <div v-else-if="!requestLoading" class="empty-state"><span>≡</span><strong>没有符合条件的请求</strong><small>调整日期或清除工具筛选后再试。</small></div>
    </div>
  </section>
</template>
