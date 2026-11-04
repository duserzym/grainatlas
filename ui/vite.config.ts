import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
// @ts-expect-error plain ES module without types
import { createFake } from './scripts/fake-laicpmsdb.mjs';

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
    name: 'grainatlas-demo',
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

// A stand-in for laicpmsdb under /fake-laicpmsdb/ (scripts/fake-laicpmsdb.mjs) in `pnpm dev`
// and `pnpm preview` only (the Playwright tests): atlas publish and serving, mounts, spot sync
// and mount results, until the real endpoints exist. Never part of a build.
function fakeLaicpmsdb(): Plugin {
  const handler = createFake();
  return {
    name: 'grainatlas-fake-laicpmsdb',
    configureServer(server) {
      server.middlewares.use(handler);
    },
    configurePreviewServer(server) {
      server.middlewares.use(handler);
    },
  };
}

// One build for the static web page and for Tauri: `base: './'` keeps every
// asset path relative (any static host, tauri://localhost), routing is
// hash-based, and the wasm core runs in an ES-module worker.
export default defineConfig({
  base: './',
  plugins: [svelte(), demo(), fakeLaicpmsdb()],
  worker: { format: 'es' },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false, target: 'es2022', chunkSizeWarningLimit: 4000 },
  server: { port: 5174, strictPort: true },
  clearScreen: false,
});
