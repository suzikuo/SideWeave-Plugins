<script setup lang="ts">
import { ref } from 'vue'
import { Button } from './components/ui/button'
import { pluginCall } from './runtime'

const message = ref('')
const loading = ref(false)

async function hello() {
  loading.value = true
  try {
    const result = await pluginCall<{ message: string }>('hello')
    message.value = result.message
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <section class="panel">
    <h2>Demo Plugin</h2>
    <p>{{ message || 'Plugin detail page is running.' }}</p>
    <Button variant="outline" size="sm" type="button" :disabled="loading" @click="hello">
      {{ loading ? 'Loading…' : 'Call plugin' }}
    </Button>
  </section>
</template>

<style scoped>
.panel { display: grid; gap: 12px; color: var(--text); }
h2, p { margin: 0; }
button { width: fit-content; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-subtle); color: inherit; }
</style>
