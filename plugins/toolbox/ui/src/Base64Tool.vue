<script setup lang="ts">
import Button from '@ui-sdk/Button.vue'
/** Base64 工具：UTF-8 安全（TextEncoder + 分块 btoa），双向编码解码 + 交换 */
import { inject, ref } from 'vue'
import { FALLBACK_API, TOOLBOX_API_KEY, decodeBase64, encodeBase64 } from './shared'

const api = inject(TOOLBOX_API_KEY, FALLBACK_API)

const input = ref('')
const output = ref('')
const status = ref<{ kind: 'ok' | 'err' | 'hint'; text: string }>({
  kind: 'hint',
  text: '输入文本，点击编码或解码'
})

function fail(text: string) {
  output.value = ''
  status.value = { kind: 'err', text }
}

function encode() {
  if (!input.value) {
    api.showToast('请先输入内容', 'error')
    return
  }
  try {
    output.value = encodeBase64(input.value)
    status.value = { kind: 'ok', text: `编码成功 · ${output.value.length} 字符` }
  } catch {
    fail('编码失败：内容无法转换为 Base64')
  }
}

function decode() {
  if (!input.value) {
    api.showToast('请先输入内容', 'error')
    return
  }
  try {
    output.value = decodeBase64(input.value)
    status.value = { kind: 'ok', text: '解码成功' }
  } catch {
    fail('解码失败：不是有效的 Base64 字符串')
  }
}

/** 把输出搬回输入区，方便反向转换 */
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
          <div class="tb-title">Base64 编解码</div>
          <div class="tb-desc">UTF-8 安全 · 支持大文本与 data: 前缀</div>
        </div>
        <span class="tb-flex" />
        <Button variant="default" size="sm" class="tb-btn primary" @click="encode">编码</Button>
        <Button variant="default" size="sm" class="tb-btn primary" @click="decode">解码</Button>
        <Button variant="outline" size="sm" class="tb-btn" @click="swap">交换</Button>
        <Button variant="ghost" size="sm" class="tb-btn ghost" @click="reset">重置</Button>
      </div>

      <div class="tb-label-row"><span class="tb-label">输入</span></div>
      <textarea
        v-model="input"
        class="tb-textarea io-area"
        spellcheck="false"
        placeholder="输入要编码或解码的文本…"
      ></textarea>

      <div class="tb-status" :class="status.kind">{{ status.text }}</div>

      <div class="tb-label-row">
        <span class="tb-label">输出</span>
        <span class="tb-flex" />
        <Button variant="outline" size="sm" class="tb-btn sm" :disabled="!output" @click="api.copyText(output, '已复制输出')">
          复制
        </Button>
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
