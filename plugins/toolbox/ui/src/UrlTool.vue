<script setup lang="ts">
/** URL 编解码工具：encodeURIComponent / decodeURIComponent 双向 + 交换 */
import { inject, ref } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const input = ref('')
const output = ref('')
const status = ref<{ kind: 'ok' | 'err' | 'hint'; text: string }>({
  kind: 'hint',
  text: '输入文本，点击编码或解码'
})

function encode() {
  if (!input.value) {
    api.showToast('请先输入内容', 'error')
    return
  }
  output.value = encodeURIComponent(input.value)
  status.value = { kind: 'ok', text: '编码成功 · 空格转 %20' }
}

function decode() {
  if (!input.value) {
    api.showToast('请先输入内容', 'error')
    return
  }
  try {
    output.value = decodeURIComponent(input.value)
    status.value = { kind: 'ok', text: '解码成功' }
  } catch {
    output.value = ''
    status.value = { kind: 'err', text: '解码失败：URL 编码格式不正确（含无效的 % 序列）' }
  }
}

function swap() {
  if (!output.value) {
    api.showToast('暂无可交换的输出', 'error')
    return
  }
  input.value = output.value
  output.value = ''
  status.value = { kind: 'hint', text: '已交换到输入区，请选择另一方向继续' }
}

function reset() {
  input.value = ''
  output.value = ''
  status.value = { kind: 'hint', text: '输入文本，点击编码或解码' }
}
</script>

<template>
  <div class="tb-stack">
    <div class="tb-panel">
      <div class="tb-head">
        <div class="tb-head-text">
          <div class="tb-title">URL 编解码</div>
          <div class="tb-desc">encodeURIComponent / decodeURIComponent</div>
        </div>
        <span class="tb-flex" />
        <button class="tb-btn primary" @click="encode">编码</button>
        <button class="tb-btn primary" @click="decode">解码</button>
        <button class="tb-btn" @click="swap">交换</button>
        <button class="tb-btn ghost" @click="reset">重置</button>
      </div>

      <div class="tb-label-row"><span class="tb-label">输入</span></div>
      <textarea
        v-model="input"
        class="tb-textarea io-area"
        spellcheck="false"
        placeholder="例如：https://example.com/search?q=工具箱"
      ></textarea>

      <div class="tb-status" :class="status.kind">{{ status.text }}</div>

      <div class="tb-label-row">
        <span class="tb-label">输出</span>
        <span class="tb-flex" />
        <button class="tb-btn sm" :disabled="!output" @click="api.copyText(output, '已复制输出')">
          复制
        </button>
      </div>
      <pre class="tb-out io-out" :class="{ empty: !output }">{{ output || '输出将显示在这里' }}</pre>
    </div>
  </div>
</template>

<style scoped>
.io-area {
  height: 110px;
  min-height: 80px;
}
.io-out {
  height: 150px;
  min-height: 80px;
}
</style>
