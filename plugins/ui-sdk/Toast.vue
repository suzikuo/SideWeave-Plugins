<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{
  message: string
  kind?: 'info' | 'error' | 'success'
  duration?: number
}>()

const emit = defineEmits<{ (event: 'done'): void }>()
const visible = ref(false)
let timer: number | undefined

watch(
  () => props.message,
  (message) => {
    if (!message.trim()) {
      visible.value = false
      window.clearTimeout(timer)
      return
    }
    visible.value = true
    window.clearTimeout(timer)
    timer = window.setTimeout(() => {
      visible.value = false
      emit('done')
    }, props.duration ?? 3200)
  },
  { immediate: true }
)
</script>

<template>
  <div v-if="visible" class="plugin-toast" :class="kind ?? 'info'">{{ message }}</div>
</template>

<style scoped>
.plugin-toast {
  position: fixed;
  left: 50%;
  bottom: 28px;
  transform: translateX(-50%);
  max-width: 70%;
  padding: 9px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius, 6px);
  background: var(--surface-subtle);
  box-shadow: var(--shadow);
  color: var(--text);
  font-size: 12.5px;
  white-space: pre-wrap;
  word-break: break-all;
  z-index: 1000;
}
.plugin-toast.error { border-color: var(--danger); color: var(--danger); }
.plugin-toast.success { border-color: var(--success); color: var(--success); }
</style>
