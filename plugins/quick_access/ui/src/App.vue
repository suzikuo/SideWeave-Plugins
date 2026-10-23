<script setup lang="ts">
import { ref } from 'vue'
import BookmarksModule from './BookmarksModule.vue'
import LauncherModule from './LauncherModule.vue'

const activeSection = ref<'apps' | 'bookmarks'>('apps')
</script>

<template>
  <main class="quick-access">
    <nav class="section-tabs" aria-label="快捷入口模块" role="tablist">
      <button
        type="button"
        role="tab"
        :aria-selected="activeSection === 'apps'"
        :class="{ active: activeSection === 'apps' }"
        @click="activeSection = 'apps'"
      >
        应用
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="activeSection === 'bookmarks'"
        :class="{ active: activeSection === 'bookmarks' }"
        @click="activeSection = 'bookmarks'"
      >
        书签
      </button>
    </nav>

    <section class="module-view" role="tabpanel">
      <LauncherModule v-if="activeSection === 'apps'" />
      <BookmarksModule v-else />
    </section>
  </main>
</template>

<style>
.quick-access {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  color: var(--sw-fg, #e7eaf0);
}

.quick-access .section-tabs {
  display: flex;
  flex: 0 0 auto;
  gap: 6px;
  padding: 10px 14px 8px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.quick-access .section-tabs button {
  padding: 6px 12px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  color: rgba(230, 234, 242, 0.68);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.quick-access .section-tabs button:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #fff;
}

.quick-access .section-tabs button.active {
  border-color: rgba(116, 169, 255, 0.28);
  background: rgba(85, 133, 214, 0.18);
  color: #fff;
}

.quick-access .module-view {
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
}
</style>
