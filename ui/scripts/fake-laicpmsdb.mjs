// A stand-in for the laicpmsdb endpoints grainatlas calls, for `pnpm dev`, `pnpm preview` and
// the Playwright tests, mounted by vite.config.ts under /fake-laicpmsdb (never in a build).
// It follows upbstudio/docs/WORKFLOW.md v1 and laicpmsdb's schemas/openapi.json; where
// laicpmsdb has no endpoint yet (atlas upload and serving, mount results) it implements
// what grainatlas codes against, so it is a fake, not a reference:
//
//   GET  /v1/mounts                       MountOut list (Bearer token required)
//   POST /v1/mounts/{id}/spots            SpotImportOut (Bearer)
//   POST /v1/mounts/{id}/atlas            multipart `bundle` = the zip; checks every hash in
//                                         atlas.json; 201 { mount_id, project_id, version, sha256, url, files }
//   GET  /v1/mounts/{id}/results          tests/fixtures/mount-results.json (Bearer)
//   GET  /atlas/projects/<id>/<path>      the latest published bundle, else a bundle folder
//                                         preloaded from ../target/atlas-fixture/
//   GET  /_fake/state, POST /_fake/reset  what was received (tests)
//
// The token is `test-token`. CORS is open so a page on another origin can use it.

import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

export const PREFIX = '/fake-laicpmsdb';
export const TOKEN = 'test-token';
const FIXTURES = new URL('../tests/fixtures/', import.meta.url).pathname;
const PRELOAD = new URL('../../target/atlas-fixture/', import.meta.url).pathname;

const MOUNTS = [
  { id: '01972c4e-0000-7000-8000-0000000000b1', label: 'YZ-ZRN-01', mount_type: 'epoxy 1 inch', grain_count: 24, spot_count: 3, can_edit: true },
  { id: '01972c4e-0000-7000-8000-0000000000b2', label: 'YZ-APT-02', mount_type: 'epoxy 1 inch', grain_count: 0, spot_count: 0, can_edit: false },
];

const sha = (b) => createHash('sha256').update(b).digest('hex');

function unzip(buf) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  let end = buf.length - 22;
  while (end >= 0 && dv.getUint32(end, true) !== 0x06054b50) end--;
  if (end < 0) throw new Error('not a zip');
  const n = dv.getUint16(end + 10, true);
  let at = dv.getUint32(end + 16, true);
  const files = new Map();
  for (let i = 0; i < n; i++) {
    if (dv.getUint16(at + 10, true) !== 0) throw new Error('compressed entries are not accepted by the fake');
    const size = dv.getUint32(at + 20, true);
    const nameLen = dv.getUint16(at + 28, true);
    const local = dv.getUint32(at + 42, true);
    const name = buf.subarray(at + 46, at + 46 + nameLen).toString('utf8');
    const start = local + 30 + dv.getUint16(local + 26, true) + dv.getUint16(local + 28, true);
    // Entries may sit under "<id>.atlas/" or at the root.
    files.set(name.replace(/^[^/]*\.atlas\//, ''), buf.subarray(start, start + size));
    at += 46 + nameLen + dv.getUint16(at + 30, true) + dv.getUint16(at + 32, true);
  }
  return files;
}

function loadPreloaded() {
  const out = new Map();
  if (!existsSync(PRELOAD)) return out;
  for (const d of readdirSync(PRELOAD)) {
    if (!d.endsWith('.atlas')) continue;
    const root = join(PRELOAD, d);
    const files = new Map();
    const walk = (dir, rel) => {
      for (const n of readdirSync(dir)) {
        const p = join(dir, n);
        if (statSync(p).isDirectory()) walk(p, `${rel}${n}/`);
        else files.set(`${rel}${n}`, readFileSync(p));
      }
    };
    walk(root, '');
    out.set(d.replace(/\.atlas$/, ''), { files, version: 0 });
  }
  return out;
}

export function createFake() {
  let state;
  const reset = () => {
    state = { atlases: loadPreloaded(), published: [], spots: [], requests: [] };
  };
  reset();

  const send = (res, status, body, type = 'application/json') => {
    res.statusCode = status;
    res.setHeader('Content-Type', type);
    res.end(type === 'application/json' ? JSON.stringify(body) : body);
  };
  const authed = (req) => req.headers.authorization === `Bearer ${TOKEN}`;
  const body = (req) =>
    new Promise((resolve, reject) => {
      const chunks = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', () => resolve(Buffer.concat(chunks)));
      req.on('error', reject);
    });

  return async function handle(req, res, next) {
    const url = new URL(req.url || '/', 'http://x');
    if (!url.pathname.startsWith(PREFIX + '/')) return next();
    const path = decodeURIComponent(url.pathname.slice(PREFIX.length));
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    if (req.method === 'OPTIONS') return send(res, 204, '', 'text/plain');
    state.requests.push({ method: req.method, path, auth: req.headers.authorization ?? null });
    let m;
    try {
      if (path === '/_fake/state') return send(res, 200, { published: state.published, spots: state.spots, requests: state.requests, atlases: [...state.atlases.keys()] });
      if (path === '/_fake/reset' && req.method === 'POST') {
        reset();
        return send(res, 200, { ok: true });
      }
      if ((m = path.match(/^\/atlas\/projects\/([^/]+)\/(.+)$/)) && req.method === 'GET') {
        const a = state.atlases.get(m[1]);
        const f = a?.files.get(m[2]);
        if (!f) return send(res, 404, { detail: 'not found' });
        const type = m[2].endsWith('.webp') ? 'image/webp' : m[2].endsWith('.geojson') ? 'application/geo+json' : 'application/json';
        res.setHeader('Cache-Control', 'no-cache');
        return send(res, 200, f, type);
      }
      if (path === '/v1/mounts' && req.method === 'GET') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        return send(res, 200, MOUNTS);
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/spots$/)) && req.method === 'POST') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const doc = JSON.parse((await body(req)).toString('utf8'));
        if (!doc.project_id || !Array.isArray(doc.spots)) return send(res, 422, { detail: 'project_id and spots are required' });
        state.spots.push({ mount_id: m[1], doc });
        return send(res, 200, { mount_id: m[1], project_id: doc.project_id, sample_id: null, sample_name: doc.sample || null, created: doc.spots.length, updated: 0, unchanged: 0, grains_created: 0, acquisitions: [] });
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/atlas$/)) && req.method === 'POST') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const raw = await body(req);
        const form = await new Response(raw, { headers: { 'content-type': req.headers['content-type'] || '' } }).formData();
        const file = form.get('bundle');
        if (!file || typeof file === 'string') return send(res, 422, { detail: 'multipart field "bundle" (the zip) is required' });
        const zip = Buffer.from(await file.arrayBuffer());
        const files = unzip(zip);
        const atlas = JSON.parse(files.get('atlas.json')?.toString('utf8') ?? 'null');
        if (atlas?.format !== 'grainatlas-atlas' || atlas.version !== 1) return send(res, 422, { detail: 'atlas.json: not a grainatlas-atlas version 1' });
        for (const f of atlas.files) {
          const b = files.get(f.path);
          if (!b || b.length !== f.size || sha(b) !== f.sha256) return send(res, 422, { detail: `${f.path} does not match atlas.json` });
        }
        const prev = state.atlases.get(atlas.project_id);
        const version = (prev?.version ?? 0) + 1;
        state.atlases.set(atlas.project_id, { files, version });
        const rec = { mount_id: m[1], project_id: atlas.project_id, version, sha256: sha(zip), url: `/atlas/projects/${atlas.project_id}/`, files: files.size, filename: file.name };
        state.published.push(rec);
        return send(res, 201, rec);
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/results$/)) && req.method === 'GET') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const r = JSON.parse(readFileSync(join(FIXTURES, 'mount-results.json'), 'utf8'));
        return send(res, 200, { ...r, mount_id: m[1] });
      }
      return send(res, 404, { detail: `no fake for ${req.method} ${path}` });
    } catch (e) {
      return send(res, 500, { detail: String(e) });
    }
  };
}

// `node scripts/fake-laicpmsdb.mjs [port]`: the fake on its own (another origin than the page).
if (import.meta.url === `file://${process.argv[1]}`) {
  const { createServer } = await import('node:http');
  const port = Number(process.argv[2] || 8765);
  const h = createFake();
  createServer((req, res) => h(req, res, () => send404(res))).listen(port, () => console.log(`fake laicpmsdb on http://localhost:${port}${PREFIX}`));
  function send404(res) {
    res.statusCode = 404;
    res.end();
  }
}
