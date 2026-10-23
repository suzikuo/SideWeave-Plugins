<script setup lang="ts">
import { ref } from 'vue'
import CloudflareModule from './CloudflareModule.vue'
import PortForwardModule from './PortForwardModule.vue'
import SshModule from './SshModule.vue'

type Section = 'ssh' | 'ports' | 'tunnel'
const activeSection = ref<Section>('ssh')
</script>

<template>
  <main class="dev-connection">
    <nav class="section-tabs" aria-label="开发连接模块" role="tablist">
      <button type="button" role="tab" :aria-selected="activeSection === 'ssh'" :class="{ active: activeSection === 'ssh' }" @click="activeSection = 'ssh'">
        SSH
      </button>
      <button type="button" role="tab" :aria-selected="activeSection === 'ports'" :class="{ active: activeSection === 'ports' }" @click="activeSection = 'ports'">
        端口转发
      </button>
      <button type="button" role="tab" :aria-selected="activeSection === 'tunnel'" :class="{ active: activeSection === 'tunnel' }" @click="activeSection = 'tunnel'">
        Cloudflare Tunnel
      </button>
    </nav>

    <section class="module-view" role="tabpanel">
      <SshModule v-if="activeSection === 'ssh'" />
      <PortForwardModule v-else-if="activeSection === 'ports'" />
      <CloudflareModule v-else />
    </section>
  </main>
</template>

<style>
.dev-connection {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  color: var(--sw-fg, #e7eaf0);
}

.dev-connection .section-tabs {
  display: flex;
  flex: 0 0 auto;
  gap: 6px;
  padding: 10px 14px 8px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.dev-connection .section-tabs button {
  padding: 6px 12px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  color: rgba(230, 234, 242, 0.68);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.dev-connection .section-tabs button:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #fff;
}

.dev-connection .section-tabs button.active {
  border-color: rgba(116, 169, 255, 0.28);
  background: rgba(85, 133, 214, 0.18);
  color: #fff;
}

.dev-connection .module-view {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
}
</style>
