<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 二维码识别：选择图片文件 → 后端 rqrr 解码 → 展示图中全部二维码内容，点击复制 */
import { inject, ref } from 'vue'
import { pluginCall } from '../../../ui-sdk/runtime'
import { FALLBACK_API, TOOLBOX_API_KEY } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const results = ref<string[]>([])
const error = ref('')
const loading = ref(false)

async function pickImage() {
  if (loading.value) return
  loading.value = true
  error.value = ''
  results.value = []
  try {
    const list = await pluginCall<string[]>('decode_qr')
    results.value = list
    if (list.length === 0) error.value = '未在图片中识别到二维码'
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">二维码识别</div>
          <div class="tb-desc">选择图片文件，解码图中全部二维码（png / jpg / webp / bmp）</div>
        </div>
        <span class="tb-flex" />
        <Button variant="outline" size="sm" class="tb-btn" :disabled="loading" @click="pickImage">
          {{ loading ? '识别中…' : '选择图片' }}
        </Button>
      </div>

      <p v-if="error" class="qr-error">{{ error }}</p>

      <div v-for="(text, i) in results" :key="i" class="qr-result" @click="api.copyText(text, '已复制')">
        <span class="qr-idx">#{{ i + 1 }}</span>
        <span class="qr-text">{{ text }}</span>
      </div>
      <p v-if="results.length" class="qr-hint">点击任意结果复制内容</p>
    </div>
  </div>
</template>

<style scoped>
.qr-error {
  margin: 0 0 10px;
  padding: 9px 12px;
  border-radius: 9px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  color: var(--danger);
  font-size: 12.5px;
}
.qr-result {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 9px 12px;
  margin-bottom: 8px;
  border: 1px solid var(--border);
  border-radius: 9px;
  cursor: pointer;
  font-size: 12.5px;
  word-break: break-all;
}
.qr-result:hover {
  border-color: var(--primary);
}
.qr-idx {
  flex: none;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
.qr-text {
  min-width: 0;
  white-space: pre-wrap;
  color: var(--text);
}
.qr-hint {
  margin: 0;
  font-size: 11px;
  color: var(--text-muted);
}
</style>
