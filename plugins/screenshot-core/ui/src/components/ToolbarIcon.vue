<script setup lang="ts">
defineProps<{
  id: string
  size?: number
}>()
</script>

<template>
  <!-- 1:1 synchronized SVG paths matching native GDI+ draw_action_icon_on_graphics() -->
  <svg
    :width="size ?? 18"
    :height="size ?? 18"
    viewBox="-10 -10 20 20"
    fill="none"
    stroke="currentColor"
    stroke-width="1.85"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="id === 'drag'">
      <circle cx="-2.6" cy="-5.5" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="2.6" cy="-5.5" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="-2.6" cy="0" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="2.6" cy="0" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="-2.6" cy="5.5" r="1.35" fill="currentColor" stroke="none" />
      <circle cx="2.6" cy="5.5" r="1.35" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="id === 'select'">
      <path d="M-5.5 -7.2 L6.2 0.2 L0.6 1.8 L-1.6 7.2 Z" />
    </template>
    <template v-else-if="id === 'rect'">
      <rect x="-7.8" y="-6.6" width="15.6" height="13.2" rx="2.8" />
    </template>
    <template v-else-if="id === 'arrow'">
      <path d="M-6.6 6.6 L6.6 -6.6 M0.8 -6.6 L6.6 -6.6 L6.6 -0.8" />
    </template>
    <template v-else-if="id === 'pen'">
      <path d="M-6.8 7.0 L-4.5 1.8 L4.2 -7.0 L7.2 -4.0 L-1.6 4.8 Z M1.8 -4.6 L4.8 -1.6" />
    </template>
    <template v-else-if="id === 'text'">
      <rect x="-7.5" y="-7.5" width="15" height="15" rx="3" />
      <path d="M-3.8 -3.2 L3.8 -3.2 M0 -3.2 L0 4.4" />
    </template>
    <template v-else-if="id === 'number'">
      <circle cx="0" cy="0" r="8" />
      <path d="M-2.2 -1.8 L0.2 -4.0 L0.2 4.2 M-2.4 4.2 L2.6 4.2" />
    </template>
    <template v-else-if="id === 'mosaic'">
      <rect x="-7.5" y="-7.5" width="15" height="15" rx="2.8" />
      <rect x="-5.6" y="-5.6" width="3.3" height="3.3" fill="currentColor" stroke="none" />
      <rect x="2.0" y="-5.6" width="3.3" height="3.3" fill="currentColor" stroke="none" />
      <rect x="-1.8" y="-1.8" width="3.3" height="3.3" fill="currentColor" stroke="none" />
      <rect x="-5.6" y="2.0" width="3.3" height="3.3" fill="currentColor" stroke="none" />
      <rect x="2.0" y="2.0" width="3.3" height="3.3" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="id === 'eraser'">
      <path d="M-7.0 2.0 L1.0 -6.4 L7.4 0 L1.2 6.8 L-3.8 6.8 Z M-2.6 -2.0 L3.6 4.2 M1.2 6.8 L7.2 6.8" />
    </template>
    <template v-else-if="id === 'undo'">
      <path d="M-6.8 -2.6 L-2.6 -6.8 M-6.8 -2.6 L-2.6 1.6 M-6.8 -2.6 L2.4 -2.6 A4.8 4.8 0 1 1 2.4 7.0 L-2.4 7.0" />
    </template>
    <template v-else-if="id === 'redo'">
      <path d="M6.8 -2.6 L2.6 -6.8 M6.8 -2.6 L2.6 1.6 M6.8 -2.6 L-2.4 -2.6 A4.8 4.8 0 1 0 -2.4 7.0 L2.4 7.0" />
    </template>
    <template v-else-if="id === 'record'">
      <path d="M-8 -3.6 V-8 H-3.6 M3.6 -8 H8 V-3.6 M-8 3.6 V8 H-3.6 M3.6 8 H8 V3.6" />
      <circle cx="0" cy="0" r="3.1" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="id === 'pin'">
      <path d="M-0.8 -7.2 L7.2 0.8 M0.8 -5.6 L-3.4 -0.4 M5.6 -0.8 L0.4 3.4 M-5.2 -1.2 L1.2 5.2 M-2.4 2.4 L-7.4 7.4" />
    </template>
    <template v-else-if="id === 'ocr'">
      <path d="M-8 -3.6 V-8 H-3.6 M3.6 -8 H8 V-3.6 M-8 3.6 V8 H-3.6 M3.6 8 H8 V3.6 M-4.2 -2.6 H4.2 M-4.2 0.6 H4.2 M-4.2 3.8 H1.6" />
    </template>
    <template v-else-if="id === 'translate'">
      <path d="M-8 -4.8 H-0.6 M-4.3 -7.6 V-4.8 M-2.2 -4.8 L-7.0 2.6 M-6.4 -3.0 L-1.6 1.6 M1.2 7.0 L4.8 -1.6 L8.2 7.0 M2.4 4.2 H7.0" />
    </template>
    <template v-else-if="id === 'scroll'">
      <rect x="-6.4" y="-8" width="12.8" height="16" rx="2.6" />
      <path d="M0 -4.4 V4.4 M-2.8 -1.6 L0 -4.4 L2.8 -1.6 M-2.8 1.6 L0 4.4 L2.8 1.6" />
    </template>
    <template v-else-if="id === 'edit'">
      <rect x="-7.5" y="-7" width="15" height="14" rx="2.8" />
      <path d="M-1 -7 V7 M-1 0 H7.5" />
    </template>
    <template v-else-if="id === 'qr'">
      <rect x="-7.5" y="-7.5" width="5.8" height="5.8" />
      <rect x="1.7" y="-7.5" width="5.8" height="5.8" />
      <rect x="-7.5" y="1.7" width="5.8" height="5.8" />
      <circle cx="3.2" cy="3.2" r="1.4" fill="currentColor" stroke="none" />
      <circle cx="6.4" cy="6.4" r="1.4" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="id === 'save'">
      <path d="M0 -7.6 V2.6 M-3.8 -1.2 L0 2.6 L3.8 -1.2 M-7.2 2.6 V7 H7.2 V2.6" />
    </template>
    <template v-else-if="id === 'cancel'">
      <path d="M-6 -6 L6 6 M6 -6 L-6 6" />
    </template>
    <template v-else-if="id === 'finish'">
      <path d="M-7 0.4 L-1.6 6 L7.6 -5" />
    </template>
  </svg>
</template>
