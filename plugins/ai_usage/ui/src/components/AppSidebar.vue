<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { pages, page, selectPage, quotaRows, formatTokens, payload } = inject(aiUsageKey)!
</script>

<template>
  <aside class="sidebar">
    <div class="brand">
      <span class="brand-mark"><i></i><i></i><i></i></span>
      <div><strong>AI 用量</strong><small>LOCAL USAGE</small></div>
    </div>

    <div class="nav-caption">工作台</div>
    <nav class="navigation" aria-label="用量页面">
      <button
        v-for="item in pages"
        :key="item.id"
        class="nav-link"
        :class="{ active: page === item.id }"
        @click="selectPage(item.id)"
      >
        <span class="nav-icon">{{ item.icon }}</span>
        <span>{{ item.label }}</span>
        <span v-if="item.id === 'quotas' && quotaRows.length" class="nav-count">{{ quotaRows.length }}</span>
      </button>
    </nav>

    <div class="sidebar-spacer"></div>
    <div class="local-card">
      <span class="local-dot"></span>
      <div><strong>数据留在本机</strong><small>{{ formatTokens(payload?.indexed_events ?? 0, false) }} 条用量记录</small></div>
    </div>
    <div class="sidebar-foot">不读取对话内容 · 只统计用量</div>
  </aside>
</template>
