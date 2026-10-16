import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  base: './',
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true
  }
})
