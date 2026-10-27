<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { summary, formatTokens, formatMoney, period, report, trendDays, trendOptions, historyRows, hourMax, trendMax, quotaRows, toolInitial, toolName, percent, quotaWindow, resetLabel, resetTimeText, quotaEstimate, toolRows, tokenShareTotal, recentSessions, formatTime, selectPage, payload, detectedCount } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="stats-grid">
      <article class="stat-card stat-primary">
        <div class="stat-head"><span>Token 用量</span><span class="stat-icon violet">◈</span></div>
        <strong class="stat-value">{{ formatTokens(summary?.total_tokens ?? 0) }}</strong>
        <div class="stat-foot"><span>输入 {{ formatTokens(summary?.counts.input ?? 0) }}</span><span>输出 {{ formatTokens(summary?.counts.output ?? 0) }}</span></div>
      </article>
      <article class="stat-card">
        <div class="stat-head"><span>估算费用</span><span class="stat-icon amber">$</span></div>
        <strong class="stat-value">{{ formatMoney((summary?.cost ?? 0) + (summary?.credit_cost ?? 0)) }}</strong>
        <div class="stat-foot"><span>{{ summary?.unpriced.length ? `${summary.unpriced.length} 个模型无公开价格` : '基于模型公开单价估算' }}</span></div>
      </article>
      <article class="stat-card">
        <div class="stat-head"><span>请求次数</span><span class="stat-icon blue">↗</span></div>
        <strong class="stat-value">{{ formatTokens(summary?.requests ?? 0, false) }}</strong>
        <div class="stat-foot"><span>{{ formatTokens(summary?.sessions ?? 0, false) }} 个会话</span></div>
      </article>
      <article class="stat-card">
        <div class="stat-head"><span>缓存命中</span><span class="stat-icon green">⌁</span></div>
        <strong class="stat-value">{{ percent(summary?.cached_pct ?? 0) }}</strong>
        <div class="stat-foot"><span>输入上下文中的缓存占比</span></div>
      </article>
    </div>

    <div v-if="summary?.unpriced.length" class="unpriced-notice" role="note">
      <span class="unpriced-mark">!</span>
      <div class="unpriced-copy">
        <strong>{{ summary.unpriced.length }} 个模型没有公开价格，费用估算未包含这些用量</strong>
        <div class="unpriced-models">
          <span v-for="item in summary.unpriced.slice(0, 3)" :key="`${item.tool}-${item.model}`">
            {{ toolName(item.tool) }} · {{ item.model || '未知模型' }} · {{ formatTokens(item.total_tokens) }} tokens
          </span>
          <span v-if="summary.unpriced.length > 3">另有 {{ summary.unpriced.length - 3 }} 个模型</span>
        </div>
      </div>
    </div>

    <div class="overview-grid">
      <article class="panel trend-panel">
        <div class="panel-heading">
          <div><div class="section-kicker">ACTIVITY</div><h2>{{ period === 'day' ? '今日用量趋势' : `${trendDays} 天用量趋势` }}</h2></div>
          <div v-if="period !== 'day'" class="mini-selects">
            <button v-for="days in trendOptions" :key="days" :class="{ chosen: trendDays === days }" @click="trendDays = days">{{ days }}天</button>
          </div>
          <span v-else class="chart-unit">Token / 小时</span>
        </div>
        <div v-if="period === 'day'" class="hour-chart">
          <div v-for="(cell, hour) in report?.hourly ?? []" :key="hour" class="hour-col" :title="`${hour}:00 · ${formatTokens(cell.total_tokens, false)} Token`">
            <div class="hour-track"><i :style="{ height: `${Math.max(cell.total_tokens ? 5 : 1, cell.total_tokens / hourMax * 100)}%` }"></i></div>
            <small v-if="hour % 3 === 0">{{ String(hour).padStart(2, '0') }}</small>
          </div>
          <div v-if="!report?.hourly.length" class="chart-empty">暂无小时数据</div>
        </div>
        <div v-else class="day-chart">
          <div v-for="(cell, index) in historyRows" :key="cell.date" class="day-col" :title="`${cell.date} · ${formatTokens(cell.total_tokens, false)} Token`">
            <div class="day-track"><i :style="{ height: `${Math.max(cell.total_tokens ? 5 : 1, cell.total_tokens / trendMax * 100)}%` }"></i></div>
            <small v-if="index % Math.max(1, Math.ceil(historyRows.length / 7)) === 0">{{ cell.date.slice(5) }}</small>
          </div>
          <div v-if="!historyRows.length" class="chart-empty">暂无历史数据</div>
        </div>
        <div class="chart-legend"><span><i class="legend-swatch"></i>Token 使用量</span><span>{{ period === 'day' ? '按本地时间统计' : `最近 ${trendDays} 天` }}</span></div>
      </article>

      <article class="panel quota-preview">
        <div class="panel-heading">
          <div><div class="section-kicker">QUOTA</div><h2>订阅配额</h2></div>
          <button class="text-button" @click="selectPage('quotas')">查看全部 <span>→</span></button>
        </div>
        <div v-if="quotaRows.length" class="quota-list">
          <div v-for="quota in quotaRows.slice(0, 4)" :key="`${quota.tool}-${quota.id ?? quota.label}`" class="quota-row">
            <div class="quota-title"><span class="tool-avatar" :class="`avatar-${quota.tool}`">{{ toolInitial(quota.tool) }}</span><div><strong>{{ toolName(quota.tool) }}</strong><small>{{ quotaWindow(quota) }}</small></div><b>剩余 {{ percent(100 - quota.used_percent) }}</b></div>
            <div class="progress-track"><i :class="{ warning: quota.used_percent >= 75, danger: quota.used_percent >= 92 }" :style="{ width: `${Math.max(0, Math.min(100, 100 - quota.used_percent))}%` }"></i></div>
            <div class="quota-reset">{{ resetTimeText(quota.resets_at_ms) }} · {{ resetLabel(quota.resets_at_ms) }}</div>
            <div v-if="quotaEstimate(quota)?.total_cost_usd != null" class="quota-reset">预估满额 ≈ {{ formatMoney(quotaEstimate(quota)!.total_cost_usd!) }}</div>
            <div v-if="quotaEstimate(quota)?.coverage_note" class="quota-reset">{{ quotaEstimate(quota)!.coverage_note }}</div>
            <div v-if="quotaEstimate(quota)?.speed_note" class="quota-reset">{{ quotaEstimate(quota)!.speed_note }}</div>
          </div>
        </div>
        <div v-else class="empty-compact"><span>◷</span><strong>暂时没有实时配额</strong><small>有些工具仅保存本地用量记录。</small></div>
      </article>
    </div>

    <div class="lower-grid">
      <article class="panel tool-preview">
        <div class="panel-heading">
          <div><div class="section-kicker">BY TOOL</div><h2>工具用量</h2></div>
          <button class="text-button" @click="selectPage('tools')">查看排行 <span>→</span></button>
        </div>
        <div v-if="toolRows.length" class="tool-list">
          <div v-for="tool in toolRows.slice(0, 6)" :key="tool.key" class="tool-row">
            <span class="tool-avatar" :class="`avatar-${tool.key}`">{{ toolInitial(tool.key) }}</span>
            <div class="tool-info"><div class="tool-line"><strong>{{ tool.label }}</strong><span>{{ formatTokens(tool.total_tokens) }} tokens</span></div><div class="inline-track"><i :style="{ width: `${Math.max(2, tool.total_tokens / tokenShareTotal * 100)}%` }"></i></div></div>
            <div class="tool-cost"><strong>{{ formatMoney(tool.cost, tool.priced) }}</strong><small>{{ formatTokens(tool.requests, false) }} 次请求</small></div>
          </div>
        </div>
        <div v-else class="empty-compact"><strong>这个周期还没有用量</strong><small>检测到工具后，用量会显示在这里。</small></div>
      </article>

      <article class="panel session-preview">
        <div class="panel-heading">
          <div><div class="section-kicker">RECENT SESSIONS</div><h2>近期会话</h2></div>
          <button class="text-button" @click="selectPage('sessions')">会话排行 <span>→</span></button>
        </div>
        <div v-if="recentSessions.length" class="session-list">
          <div v-for="(session, index) in recentSessions.slice(0, 5)" :key="`${session.tool}-${session.session}`" class="session-row">
            <span class="rank-number">{{ String(index + 1).padStart(2, '0') }}</span>
            <div class="session-info"><strong>{{ session.session_title || session.project || session.model || '未命名会话' }}</strong><small>{{ toolName(session.tool) }} · {{ session.model }} · {{ formatTime(session.last_ms) }}</small></div>
            <div class="session-metric"><strong>{{ formatMoney(session.cost) }}</strong><small>{{ formatTokens(session.total_tokens) }} tokens</small></div>
          </div>
        </div>
        <div v-else class="empty-compact"><strong>还没有会话记录</strong><small>完成一次 AI 编程请求后会出现在这里。</small></div>
      </article>
    </div>

    <div v-if="!payload?.indexed_events" class="first-run-banner">
      <span class="first-run-icon">✦</span>
      <div><strong>开始汇总 AI 用量</strong><p>AI 用量会扫描已安装工具保存的本地使用日志。当前已识别 {{ detectedCount }} 个来源。首次索引完成后，这里会显示 Token、费用估算和使用趋势。</p></div>
      <button class="text-button" @click="selectPage('tools')">查看支持来源 →</button>
    </div>
  </section>
</template>
