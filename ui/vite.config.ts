import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

// One build for the static web page and for Tauri: `base: './'` keeps every
// asset path relative (any static host, tauri://localhost), routing is
// hash-based, and the wasm core runs in an ES-module worker.
export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version) },
  base: './',
  plugins: [svelte()],
  worker: { format: 'es' },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false, target: 'es2022', chunkSizeWarningLimit: 4000 },
  server: { port: 5174, strictPort: true },
  clearScreen: false,
});
