<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { previewSettingsError, previewSettings, previewWidgetOptions, hasPreviewWidget, togglePreviewWidget, previewSettingsMessage, previewSettingsLoading, resetPreviewSettings, savePreviewSettings, previewSettingsSaving, previewSample, formatTokens, formatMoney, toolName, formatTime, quotaWindow, percent, resetLabel } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>悬停预览设置</h2><p>选择鼠标悬停 AI 用量插件图标时展示的组件，并即时查看排版效果。</p></div><span class="subtle-pill">设置保存在本机</span></div>
    <div v-if="previewSettingsError" class="notice-banner">{{ previewSettingsError }}</div>
    <div class="preview-settings-layout">
      <article class="panel preview-settings-panel">
        <div class="settings-heading"><div><div class="section-kicker">CONTENT</div><h3>展示内容</h3></div><span class="subtle-pill">{{ previewSettings.widgets.length }} 项已选</span></div>
        <div class="preview-option-list">
          <button v-for="option in previewWidgetOptions" :key="option.id" class="preview-option" :class="{ selected: hasPreviewWidget(option.id) }" @click="togglePreviewWidget(option.id)">
            <span class="option-check" aria-hidden="true">{{ hasPreviewWidget(option.id) ? '✓' : '' }}</span>
            <span class="option-copy"><strong>{{ option.title }}</strong><small>{{ option.description }}</small></span>
          </button>
        </div>

        <div class="settings-heading display-heading"><div><div class="section-kicker">LAYOUT</div><h3>展示方式</h3></div></div>
        <div class="display-mode-options">
          <button class="display-mode-option" :class="{ selected: previewSettings.display_mode === 'cards' }" @click="previewSettings.display_mode = 'cards'; previewSettingsMessage = ''">
            <span class="mode-glyph cards-glyph"><i></i><i></i><i></i></span><strong>数据卡片</strong><small>指标分卡展示</small>
          </button>
          <button class="display-mode-option" :class="{ selected: previewSettings.display_mode === 'compact' }" @click="previewSettings.display_mode = 'compact'; previewSettingsMessage = ''">
            <span class="mode-glyph list-glyph"><i></i><i></i><i></i></span><strong>紧凑列表</strong><small>节省预览空间</small>
          </button>
        </div>

        <div class="preview-settings-actions">
          <span v-if="previewSettingsMessage" class="save-confirmation">{{ previewSettingsMessage }}</span>
          <span v-else class="settings-note">{{ previewSettingsLoading ? '读取设置中…' : '预览窗口下次打开时应用' }}</span>
          <button class="text-button" @click="resetPreviewSettings">恢复默认</button>
          <button class="primary-button" :disabled="previewSettingsSaving || previewSettingsLoading" @click="savePreviewSettings">{{ previewSettingsSaving ? '保存中…' : '保存设置' }}</button>
        </div>
      </article>

      <article class="preview-demo-column">
        <div class="preview-demo-label"><div><div class="section-kicker">LIVE PREVIEW</div><h3>预览效果</h3></div><span class="subtle-pill">360 × 260</span></div>
        <div class="preview-live-card" :class="previewSettings.display_mode">
          <div class="preview-live-head"><span class="preview-live-mark"><i></i><i></i><i></i></span><div><strong>AI 用量</strong><small>最近 24 小时</small></div><span class="live-indicator"><i></i> 本机</span></div>
          <div v-if="hasPreviewWidget('quotas')" class="preview-live-quotas">
            <div class="preview-live-section-title">订阅配额</div>
            <template v-if="previewSample?.quotas.length">
              <div v-for="quota in previewSample.quotas" :key="`${quota.tool}-${quota.id ?? quota.label ?? quota.window_minutes}`" class="preview-live-quota">
                <div class="preview-live-quota-head"><span>{{ toolName(quota.tool) }} · {{ quotaWindow(quota) }}</span><strong>剩余 {{ percent(100 - quota.used_percent) }}</strong></div>
                <i class="preview-live-quota-track"><b :class="{ warning: quota.used_percent >= 75, danger: quota.used_percent >= 92 }" :style="{ width: `${Math.max(0, Math.min(100, 100 - quota.used_percent))}%` }"></b></i>
                <small>{{ resetLabel(quota.resets_at_ms) }}</small>
              </div>
            </template>
            <small v-else>暂时没有读取到订阅额度</small>
          </div>
          <div class="preview-live-metrics" :class="previewSettings.display_mode">
            <div v-if="hasPreviewWidget('tokens')" class="preview-live-metric"><small>TOKEN 总量</small><strong>{{ formatTokens(previewSample?.total_tokens ?? 0) }}</strong><span>输入 {{ formatTokens(previewSample?.input_tokens ?? 0) }} · 输出 {{ formatTokens(previewSample?.output_tokens ?? 0) }}</span></div>
            <div v-if="hasPreviewWidget('cost')" class="preview-live-metric"><small>费用估算</small><strong>{{ formatMoney((previewSample?.cost_usd ?? 0) + (previewSample?.credit_cost_usd ?? 0)) }}</strong><span>{{ previewSample?.unpriced_models ? `${previewSample.unpriced_models} 个模型缺少价格` : '按模型价格估算' }}</span></div>
            <div v-if="hasPreviewWidget('requests')" class="preview-live-metric"><small>请求次数</small><strong>{{ formatTokens(previewSample?.requests ?? 0, false) }}</strong><span>最近 24 小时记录</span></div>
          </div>
          <div v-if="hasPreviewWidget('split')" class="preview-live-split"><div><span>输入</span><i><b :style="{ width: `${Math.max(2, (previewSample?.input_tokens ?? 0) / Math.max(1, (previewSample?.input_tokens ?? 0) + (previewSample?.output_tokens ?? 0)) * 100)}%` }"></b></i><span>输出</span></div></div>
          <div v-if="hasPreviewWidget('tools')" class="preview-live-tools"><div class="preview-live-section-title">工具排行</div><template v-if="previewSample?.tools.length"><div v-for="tool in previewSample.tools.slice(0, 2)" :key="tool.tool" class="preview-live-tool"><span>{{ toolName(tool.tool) }}</span><strong>{{ formatTokens(tool.total_tokens) }}</strong></div></template><small v-else>暂无工具用量记录</small></div>
          <div v-if="hasPreviewWidget('updated')" class="preview-live-foot">{{ previewSample?.last_ingest_ms ? `更新于 ${formatTime(previewSample.last_ingest_ms)}` : '尚未扫描本机日志' }}</div>
          <div v-if="!previewSettings.widgets.length" class="preview-live-empty">尚未选择展示内容</div>
        </div>
        <p class="preview-demo-note">实际预览读取最近 24 小时的本机索引；费用仅统计匹配到价格的模型。</p>
      </article>
    </div>
  </section>
</template>
