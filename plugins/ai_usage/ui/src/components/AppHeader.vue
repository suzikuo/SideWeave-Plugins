<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'
import GlobalToolbar from './GlobalToolbar.vue'

const { pages, page, detectedCount } = inject(aiUsageKey)!
</script>

<template>
  <header class="topbar">
  <div class="title-group">
    <div class="eyebrow">USAGE WORKSPACE <span>·</span> {{ detectedCount }} 个来源</div>
    <h1>{{ pages.find((item) => item.id === page)?.label }}</h1>
  </div>
    <GlobalToolbar />
  </header>
</template>
