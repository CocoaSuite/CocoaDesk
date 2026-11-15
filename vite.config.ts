import { defineConfig } from 'vite'

export default defineConfig({
  base: './',
  clearScreen: false,
  build: { outDir: 'dist', target: 'es2022' },
  server: { port: 5190, strictPort: true }
})
