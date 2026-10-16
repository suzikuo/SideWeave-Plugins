<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** 颜色转换：HEX / RGB / HSL 双向解析 + 预览色块 + 取色器 */
import { computed, inject, ref } from 'vue'
import {
  FALLBACK_API,
  TOOLBOX_API_KEY,
  debouncedRef,
  formatHex,
  formatHsl,
  formatRgb,
  parseColor,
  rgbToHsl
} from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const DEFAULT = '#006874'

const input = ref(DEFAULT)
const debouncedInput = debouncedRef(input, 150)

const parsed = computed(() => parseColor(debouncedInput.value))
const hsl = computed(() => (parsed.value ? rgbToHsl(parsed.value) : null))

const hexOut = computed(() => (parsed.value ? formatHex(parsed.value) : ''))
const rgbOut = computed(() => (parsed.value ? formatRgb(parsed.value) : ''))
const hslOut = computed(() => (hsl.value ? formatHsl(hsl.value) : ''))

/** 预览色块背景（带透明度时用 rgba 表示） */
const cssColor = computed(() =>
  parsed.value
    ? `rgba(${parsed.value.r}, ${parsed.value.g}, ${parsed.value.b}, ${parsed.value.a})`
    : 'transparent'
)

/** 取色器只接受 #RRGGBB，无效时回退黑色 */
const pickerHex = computed(() => (/^#[0-9a-f]{6}$/.test(hexOut.value) ? hexOut.value : '#000000'))

function onPick(e: Event) {
  input.value = (e.target as HTMLInputElement).value
}

function reset() {
  input.value = DEFAULT
}
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">颜色转换</div>
          <div class="tb-desc">HEX / RGB / HSL 双向解析 · 支持透明度</div>
        </div>
        <span class="tb-flex" />
        <Button variant="ghost" size="sm" class="tb-btn ghost" @click="reset">重置</Button>
      </div>

      <div class="color-row">
        <input
          v-model="input"
          class="tb-input tb-mono color-input"
          spellcheck="false"
          placeholder="#006874 / rgb(0 104 116) / hsl(190, 100%, 23%)"
        />
        <input type="color" class="picker" :value="pickerHex" title="取色器" @input="onPick" />
      </div>

      <div v-if="input.trim() && !parsed" class="tb-status err">
        无法识别的颜色格式，支持 #RGB #RGBA #RRGGBB #RRGGBBAA / rgb() / hsl()
      </div>

      <div class="swatch" :style="{ background: cssColor }" />

      <div class="tb-kv">
        <span class="k">HEX</span>
        <code class="tb-mono">{{ hexOut || '—' }}</code>
        <Button variant="outline" size="sm" class="tb-btn sm" :disabled="!parsed" @click="api.copyText(hexOut)">复制</Button>
      </div>
      <div class="tb-kv">
        <span class="k">RGB</span>
        <code class="tb-mono">{{ rgbOut || '—' }}</code>
        <Button variant="outline" size="sm" class="tb-btn sm" :disabled="!parsed" @click="api.copyText(rgbOut)">复制</Button>
      </div>
      <div class="tb-kv">
        <span class="k">HSL</span>
        <code class="tb-mono">{{ hslOut || '—' }}</code>
        <Button variant="outline" size="sm" class="tb-btn sm" :disabled="!parsed" @click="api.copyText(hslOut)">复制</Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.color-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.color-input {
  flex: 1;
  min-width: 0;
}
.picker {
  flex: none;
  width: 36px;
  height: 32px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface);
  cursor: pointer;
}

.swatch {
  height: 52px;
  border: 1px solid var(--border);
  border-radius: 10px;
}
</style>
