<script setup lang="ts">
import { inject } from 'vue'
import { aiUsageKey } from '../context'

const { sourceCatalog, payload, toolInitial, rootsError, rootPathDraft, addDataRoot, rootsSaving, removeDataRoot } = inject(aiUsageKey)!
</script>

<template>
  <section class="page-content">
    <div class="page-intro"><div><h2>数据来源</h2><p>本机和已添加目录都会由各工具对应的适配器扫描。</p></div><span class="subtle-pill">{{ sourceCatalog.length }} 个适配器</span></div>
    <div class="source-grid">
      <article v-for="source in sourceCatalog" :key="source.id" class="source-card">
        <span class="tool-avatar" :class="`avatar-${source.id}`">{{ toolInitial(source.id) }}</span>
        <div><strong>{{ source.display }}</strong><small>{{ source.detected ? '已发现用量数据' : source.hint || '尚未检测到数据' }}</small></div>
        <span class="source-status" :class="{ found: source.detected }">{{ source.detected ? '已发现' : '未发现' }}</span>
      </article>
    </div>
    <section class="panel roots-panel">
      <div class="roots-heading"><div><h2>外部数据目录</h2><p>输入 Linux 或 WSL 的用户根目录。插件会让所有支持的工具适配器在该目录中查找各自的数据，不需要填写 .codex 等工具子目录。</p></div></div>
      <div class="root-add-row"><input v-model="rootPathDraft" type="text" placeholder="\\wsl.localhost\Ubuntu-20.04\root" @keyup.enter="addDataRoot" /><button class="primary-button" :disabled="rootsSaving || !rootPathDraft.trim()" @click="addDataRoot">添加目录</button></div>
      <div v-if="rootsError" class="notice-banner">{{ rootsError }}</div>
      <div v-if="payload?.data_roots.length" class="root-list"><div v-for="root in payload.data_roots" :key="root.id" class="root-row"><code>{{ root.path }}</code><button class="text-button" :disabled="rootsSaving" @click="removeDataRoot(root.id)">移除</button></div></div>
      <div v-else class="root-empty">尚未添加外部目录</div>
      <p class="source-help">添加后，顶部“来源”筛选器会出现该目录，可切换为只看这个目录的用量。</p>
    </section>
  </section>
</template>
