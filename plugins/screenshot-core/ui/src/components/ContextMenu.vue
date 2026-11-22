<script setup lang="ts">
import { nextTick, onUnmounted, ref, watch } from 'vue'

export interface ContextMenuItem {
  id: string
  label?: string
  shortcut?: string
  checked?: boolean
  danger?: boolean
  disabled?: boolean
  divider?: boolean
}

const props = defineProps<{
  visible: boolean
  x: number
  y: number
  items: ContextMenuItem[]
}>()

const emit = defineEmits<{
  select: [id: string]
  close: []
}>()

const menuRef = ref<HTMLDivElement | null>(null)
const pos = ref({ left: 0, top: 0 })

function clampPosition() {
  const pad = 8
  const el = menuRef.value
  const mw = el?.offsetWidth || 190
  const mh = el?.offsetHeight || 220
  const left = Math.max(pad, Math.min(props.x, window.innerWidth - mw - pad))
  const top = Math.max(pad, Math.min(props.y, window.innerHeight - mh - pad))
  pos.value = { left, top }
}

function onGlobalPointerDown(e: PointerEvent) {
  const el = menuRef.value
  if (el && el.contains(e.target as Node)) return
  emit('close')
}

function onGlobalKeyDown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.preventDefault()
    emit('close')
  }
}

function attach() {
  window.addEventListener('pointerdown', onGlobalPointerDown, true)
  window.addEventListener('keydown', onGlobalKeyDown, true)
  window.addEventListener('blur', () => emit('close'), { once: true })
}

function detach() {
  window.removeEventListener('pointerdown', onGlobalPointerDown, true)
  window.removeEventListener('keydown', onGlobalKeyDown, true)
}

watch(
  () => [props.visible, props.x, props.y],
  () => {
    if (props.visible) {
      pos.value = { left: props.x, top: props.y }
      attach()
      void nextTick(clampPosition)
    } else {
      detach()
    }
  },
  { immediate: true },
)

onUnmounted(() => {
  detach()
})

function onPick(item: ContextMenuItem) {
  if (item.disabled || item.divider) return
  emit('select', item.id)
  emit('close')
}
</script>

<template>
  <Teleport to="body">
    <Transition name="sn-ctx-pop">
      <div
        v-if="visible"
        ref="menuRef"
        class="sn-ctx-menu"
        :style="{ left: `${pos.left}px`, top: `${pos.top}px` }"
        @contextmenu.prevent
      >
        <template v-for="(item, idx) in items" :key="item.id || idx">
          <div v-if="item.divider" class="sn-ctx-sep" />
          <button
            v-else
            type="button"
            class="sn-ctx-item"
            :class="{ danger: item.danger, disabled: item.disabled }"
            :disabled="item.disabled"
            @click="onPick(item)"
          >
            <span class="sn-ctx-check">{{ item.checked ? '✓' : '' }}</span>
            <span class="sn-ctx-label">{{ item.label }}</span>
            <span v-if="item.shortcut" class="sn-ctx-kbd">{{ item.shortcut }}</span>
          </button>
        </template>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.sn-ctx-menu {
  position: fixed;
  z-index: 10000;
  min-width: 176px;
  max-width: 248px;
  padding: 5px;
  border-radius: 11px;
  border: 1px solid rgba(128, 128, 138, 0.24);
  background: rgba(28, 28, 32, 0.96);
  color: #f4f4f6;
  backdrop-filter: blur(16px);
  box-shadow:
    0 14px 36px rgba(0, 0, 0, 0.38),
    0 2px 8px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  gap: 1px;
  user-select: none;
}

.sn-ctx-sep {
  height: 1px;
  background: rgba(255, 255, 255, 0.09);
  margin: 4px 6px;
}

.sn-ctx-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 9px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: inherit;
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.sn-ctx-item:hover:not(:disabled) {
  background: rgba(52, 199, 89, 0.18);
  color: #5ce07a;
}

.sn-ctx-item.danger {
  color: #f87171;
}

.sn-ctx-item.danger:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.2);
  color: #fca5a5;
}

.sn-ctx-item:disabled {
  opacity: 0.38;
  cursor: not-allowed;
}

.sn-ctx-check {
  width: 14px;
  font-size: 11px;
  font-weight: 700;
  color: #34c759;
  text-align: center;
  flex-shrink: 0;
}

.sn-ctx-label {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.sn-ctx-kbd {
  font-size: 10.5px;
  color: rgba(244, 244, 246, 0.45);
  margin-left: 10px;
  flex-shrink: 0;
}

.sn-ctx-pop-enter-active,
.sn-ctx-pop-leave-active {
  transition: opacity 0.1s ease, transform 0.1s ease;
}
.sn-ctx-pop-enter-from,
.sn-ctx-pop-leave-to {
  opacity: 0;
  transform: scale(0.96);
}
</style>
