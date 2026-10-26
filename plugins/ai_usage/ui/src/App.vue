<script setup lang="ts">
import { provide } from 'vue'
import AppHeader from './components/AppHeader.vue'
import AppSidebar from './components/AppSidebar.vue'
import AccountNameDialog from './components/AccountNameDialog.vue'
import OverviewPage from './pages/OverviewPage.vue'
import ToolsPage from './pages/ToolsPage.vue'
import SourcesPage from './pages/SourcesPage.vue'
import AccountsPage from './pages/AccountsPage.vue'
import SessionsPage from './pages/SessionsPage.vue'
import RequestsPage from './pages/RequestsPage.vue'
import QuotasPage from './pages/QuotasPage.vue'
import PreviewSettingsPage from './pages/PreviewSettingsPage.vue'
import { aiUsageKey } from './context'
import { useAiUsage } from './composables/useAiUsage'
import './ai-usage.css'

const state = useAiUsage()
provide(aiUsageKey, state)
const { page, period, periodOptions, report, selectedWindow, loading, error, payload, refresh, syncing, syncWarning } = state
const pageViews = {
  overview: OverviewPage,
  tools: ToolsPage,
  sources: SourcesPage,
  accounts: AccountsPage,
  sessions: SessionsPage,
  requests: RequestsPage,
  quotas: QuotasPage,
  preview: PreviewSettingsPage,
}
</script>

<template>
  <div class="usage-app">
    <AppSidebar />
    <main class="main-panel">
      <AppHeader />
      <div v-if="report && ['overview', 'tools', 'sessions'].includes(page)" class="period-switch" aria-label="统计周期">
        <button v-for="option in periodOptions" :key="option.id" :class="{ selected: period === option.id }" @click="period = option.id">
          {{ option.label }}
        </button>
        <span class="period-hint">{{ selectedWindow?.label }}</span>
      </div>
      <section v-if="loading && !payload" class="loading-view">
        <div class="loading-mark"><i></i><i></i><i></i></div>
        <strong>正在读取本机用量</strong>
        <span>正在读取已保存的统计，日志将在后台同步</span>
      </section>
      <section v-else-if="error && !payload" class="error-view">
        <div class="error-mark">!</div>
        <h2>暂时无法读取用量</h2>
        <p>{{ error }}</p>
        <button class="primary-button" @click="refresh()">重试</button>
      </section>
      <template v-else-if="report">
        <div v-if="syncing" class="notice-banner">正在后台同步日志，当前显示已索引的数据；首次同步可能需要较长时间。</div>
        <div v-if="syncWarning || error" class="notice-banner" role="alert">{{ syncWarning || error }}</div>
        <div v-if="payload?.refresh_warning" class="notice-banner">{{ payload.refresh_warning }}</div>
        <component :is="pageViews[page]" />
      </template>
    </main>
    <AccountNameDialog />
  </div>
</template>
