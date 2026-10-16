<script setup lang="ts">
import { computed, ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  modelValue: number
  min: number
  max: number
  step?: number
  format?: (value: number) => string
  ariaLabel?: string
}>(), { step: 1 })
const emit = defineEmits<{ (event: 'commit', value: number): void }>()
const draft = ref(props.modelValue)
watch(() => props.modelValue, (value) => { draft.value = value })
const percent = computed(() => {
  const span = props.max - props.min
  return span > 0 ? Math.min(100, Math.max(0, ((draft.value - props.min) / span) * 100)) : 0
})
const label = computed(() => props.format?.(draft.value) ?? String(draft.value))
function onInput(event: Event) { draft.value = Number((event.target as HTMLInputElement).value) }
</script>

<template>
  <span class="rslider">
    <span class="bubble" :style="{ left: `${percent}%` }">{{ label }}</span>
    <input type="range" :min="min" :max="max" :step="step" :value="draft" :aria-label="ariaLabel" @input="onInput" @change="emit('commit', draft)" />
  </span>
</template>

<style scoped>
.rslider { position: relative; display: inline-flex; align-items: center; }
.rslider input { width: 180px; accent-color: var(--primary); cursor: pointer; }
.bubble { position: absolute; bottom: calc(100% + 8px); transform: translateX(-50%); padding: 3px 8px; border-radius: var(--radius, 6px); background: var(--popover, var(--surface)); border: 1px solid var(--border); color: var(--popover-foreground, var(--text)); box-shadow: var(--shadow); font-size: 11px; white-space: nowrap; opacity: 0; pointer-events: none; transition: opacity .16s ease; }
.rslider:hover .bubble, .rslider:focus-within .bubble { opacity: 1; }
</style>
