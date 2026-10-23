<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { sessionRows, selectedWindow, sessionError, sessionLoading, formatTokens, toolName, formatMoney } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>会话消耗排行</h2><p>按上方统计周期筛选，再根据会话累计 Token 和费用估算排序</p></div><span class="subtle-pill">{{ sessionRows.length }} 个会话 · {{ selectedWindow?.label }}</span></div>
    <div v-if="sessionError" class="notice-banner">{{ sessionError }}</div>
    <div class="panel table-panel">
      <div v-if="sessionLoading && !sessionRows.length" class="loading-table">正在读取会话排行…</div>
      <div v-else-if="sessionRows.length" class="usage-table">
        <div class="table-head session-columns"><span>排名 / 项目</span><span>工具 / 模型</span><span>Token 总量</span><span>请求</span><span>估算费用</span></div>
        <div v-for="(session, index) in sessionRows" :key="`${session.tool}-${session.session}-${index}`" class="table-row session-columns">
          <div class="session-cell"><span class="rank-number">{{ String(index + 1).padStart(2, '0') }}</span><div><strong :title="session.session_title || session.session">{{ session.session_title || session.project || '未命名会话' }}</strong><small>{{ session.project || session.session.slice(0, 18) }}</small></div></div>
          <div><strong>{{ toolName(session.tool) }}</strong><small>{{ session.model }}</small></div>
          <div class="money-cell">{{ formatTokens(session.total_tokens) }}</div>
          <div>{{ formatTokens(session.requests, false) }}</div>
          <div class="money-cell">{{ formatMoney(session.cost) }}</div>
        </div>
      </div>
      <div v-else-if="!sessionLoading" class="empty-state"><span>▤</span><strong>这个周期没有会话</strong><small>切换上方周期，或检查当前来源是否有用量记录。</small></div>
    </div>
  </section>
</template>
