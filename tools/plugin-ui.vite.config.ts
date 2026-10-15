import { readdirSync } from 'node:fs'
import { basename, extname, resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const root = resolve(process.env.AGILE_PLUGIN_UI_ROOT ?? '.')
const outDir = resolve(process.env.AGILE_PLUGIN_UI_OUT ?? './dist/plugin-ui')
const input = Object.fromEntries(
  readdirSync(root)
    .filter((file) => extname(file).toLowerCase() === '.html')
    .map((file) => [basename(file, extname(file)), resolve(root, file)])
)

export default defineConfig({
  root,
  plugins: [vue()],
  base: './',
  build: {
    target: 'es2022',
    outDir,
    emptyOutDir: true,
    rollupOptions: { input }
  }
})
