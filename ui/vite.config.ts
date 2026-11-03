import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

// The test fixture (one TM4000 area) is served as ./demo/ so the static page
// can open an example without an upload. It is copied at build time, never
// duplicated in git.
const FIXTURE = new URL('../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url).pathname;

function list(dir: string): string[] {
  const out: string[] = [];
  const walk = (d: string) => {
    for (const n of readdirSync(d)) {
      const p = join(d, n);
      if (statSync(p).isDirectory()) walk(p);
      else if (!n.startsWith('.') && n !== 'README.md') out.push(relative(dir, p).split('\\').join('/'));
    }
  };
  walk(dir);
  return out.sort();
}

function demo(): Plugin {
  const manifest = () => JSON.stringify({ name: 'Sweden_XS_150X', files: list(FIXTURE) });
  return {
    name: 'grainmap-demo',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const u = decodeURIComponent((req.url || '').split('?')[0]);
        const i = u.indexOf('/demo/');
        if (i < 0) return next();
        const rel = u.slice(i + 6);
        try {
          res.end(rel === 'manifest.json' ? manifest() : readFileSync(join(FIXTURE, rel)));
        } catch {
          res.statusCode = 404;
          res.end();
        }
      });
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'demo/manifest.json', source: manifest() });
      for (const f of list(FIXTURE)) this.emitFile({ type: 'asset', fileName: `demo/${f}`, source: readFileSync(join(FIXTURE, f)) });
    },
  };
}

// One build for the static web page and for Tauri: `base: './'` keeps every
// asset path relative (any static host, tauri://localhost), routing is
// hash-based, and the wasm core runs in an ES-module worker.
export default defineConfig({
  base: './',
  plugins: [svelte(), demo()],
  worker: { format: 'es' },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false, target: 'es2022', chunkSizeWarningLimit: 4000 },
  server: { port: 5174, strictPort: true },
  clearScreen: false,
});
