<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { selectedWindow, toolRows, toolInitial, formatTokens, tokenShareTotal, percent, formatMoney } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>工具用量排行</h2><p>{{ selectedWindow?.label }} · 按 Token 总量排序</p></div><span class="subtle-pill">{{ toolRows.length }} 个工具</span></div>
    <div class="panel table-panel">
      <div v-if="toolRows.length" class="usage-table">
        <div class="table-head tool-columns"><span>工具</span><span>Token 用量</span><span>请求 / 会话</span><span>缓存命中</span><span>估算费用</span></div>
        <div v-for="(tool, index) in [...toolRows].sort((a, b) => b.total_tokens - a.total_tokens)" :key="tool.key" class="table-row tool-columns">
          <div class="tool-cell"><span class="rank-number">{{ String(index + 1).padStart(2, '0') }}</span><span class="tool-avatar" :class="`avatar-${tool.key}`">{{ toolInitial(tool.key) }}</span><strong>{{ tool.label }}</strong></div>
          <div class="token-cell"><strong>{{ formatTokens(tool.total_tokens) }}</strong><div class="inline-track"><i :style="{ width: `${Math.max(2, tool.total_tokens / tokenShareTotal * 100)}%` }"></i></div></div>
          <div>{{ formatTokens(tool.requests, false) }} <small>/ {{ formatTokens(tool.sessions, false) }}</small></div>
          <div>{{ percent(tool.total_tokens ? (tool.counts.cache_read / Math.max(1, tool.counts.input + tool.counts.cache_creation + tool.counts.cache_read) * 100) : 0) }}</div>
          <div class="money-cell">{{ formatMoney(tool.cost, tool.priced) }}</div>
        </div>
      </div>
      <div v-else class="empty-state"><span>⌘</span><strong>当前周期暂无用量</strong><small>试试切换到“本周”或“本月”，或重新扫描本机日志。</small></div>
    </div>
  </section>
</template>
