// A stand-in for the laicpmsdb endpoints grainatlas calls, for `pnpm dev`, `pnpm preview` and
// the Playwright tests, mounted by vite.config.ts under /fake-laicpmsdb (never in a build).
// It follows upblab/docs/WORKFLOW.md v1 and laicpmsdb's schemas/openapi.json; where
// laicpmsdb has no endpoint yet (atlas upload and serving, mount results) it implements
// what grainatlas codes against, so it is a fake, not a reference:
//
//   GET  /v1/mounts?session=<id>          MountOut list, optionally filtered by session (Bearer)
//   GET  /v1/sessions                    visible sessions (Bearer)
//   GET  /v1/sessions/{id}/acquisitions   instrument labels, including standards (Bearer)
//   POST /v1/mounts/{id}/spots            SpotImportOut (Bearer)
//   POST /v1/mounts/{id}/atlas            multipart `bundle` = the zip; checks every hash in
//                                         atlas.json; 201 (200 for the same zip again) with
//                                         laicpmsdb's AtlasUploadOut fields
//   GET  /v1/mounts/{id}/results          tests/fixtures/mount-results.json (Bearer)
//   GET  /atlas/projects/<id>/<path>      the latest published bundle, else a bundle folder
//                                         preloaded from ../target/atlas-fixture/; Bearer token
//                                         or ?access=test-access, as laicpmsdb
//   GET  /_fake/state, POST /_fake/reset  what was received (tests)
//
// `/fake-laicpmsdb/ns/<name>/…` is the same fake with its own state (parallel tests).
//
// The token is `test-token`. CORS is open so a page on another origin can use it.

import { createHash } from 'node:crypto';
import { elementHash } from '../src/lib/sync-hash.ts';
import { elementsOf, emptyElements, storedElement, storedElements } from '../src/lib/sync.ts';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

export const PREFIX = '/fake-laicpmsdb';
export const TOKEN = 'test-token';
export const ACCESS = 'test-access';
const FIXTURES = new URL('../tests/fixtures/', import.meta.url).pathname;
const SESSION_LABELS = JSON.parse(readFileSync(join(FIXTURES, 'session-labels.json'), 'utf8'));
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
  const states = new Map();
  const kinds = ['spots', 'grains', 'labels'];
  const fresh = () => {
    const atlases = loadPreloaded();
    const projects = new Map();
    const mounts = new Map(MOUNTS.map(m => [m.id, { revision: 0, changes: [], shared: emptyElements() }]));
    for (const [uuid, atlas] of atlases) {
      const project = JSON.parse(atlas.files.get('project.json').toString('utf8'));
      const mount = project.laicpmsdb.mount_id ?? MOUNTS[0].id;
      const elements = storedElements(elementsOf(project));
      for (const kind of kinds) elements[kind] = elements[kind].map(e => ({ ...e, server_id: `db-${kind}-${e.id}`, project_uuid: uuid, ...(kind === 'spots' ? { acquisition_ids: [] } : {}), ...(kind === 'grains' ? { retired: false, children: [] } : {}) }));
      projects.set(uuid, { uuid, mount, name: project.package.name, version: atlas.version || 1, elements, changed_at: new Date().toISOString() });
      mounts.get(mount).revision = 1;
    }
    return { atlases, projects, mounts, published: [], spots: [], syncs: [], requests: [], offline: false };
  };
  const elementsFor = (state, project) => Object.fromEntries(kinds.map(k => [k, [...project.elements[k], ...state.mounts.get(project.mount).shared[k]]]));
  const hashesFor = async elements => Object.fromEntries(await Promise.all(kinds.map(async k => [k, await elementHash(storedElements(elements)[k])])));
  const advance = (state, project, changes) => {
    const mount = state.mounts.get(project.mount);
    const revision = ++mount.revision;
    const at = new Date().toISOString();
    project.changed_at = at;
    for (const kind of kinds) {
      const entries = new Map(project.elements[kind].map(e => [e.id, e]));
      const shared = new Map(mount.shared[kind].map(e => [e.id, e]));
      const target = id => project.uuid !== null && shared.has(id) ? shared : entries;
      const owner = id => target(id) === shared ? null : project.uuid;
      const record = (id, op, data, project_uuid = owner(id)) => mount.changes.push({ revision, kind: kind.slice(0, -1), id, op, data: data ? structuredClone(data) : null, project_uuid, at, by: 'fake-user' });
      for (const id of [...(changes[kind]?.delete ?? []), ...(changes[kind]?.retire ?? [])]) {
        const values = target(id);
        const project_uuid = owner(id);
        if (kind === 'grains') {
          const old = values.get(id);
          if (old) { const retired = { ...old, retired: true }; values.set(id, retired); record(id, 'retire', retired, project_uuid); }
        } else { values.delete(id); record(id, 'delete', null, project_uuid); }
      }
      for (const el of changes[kind]?.upsert ?? []) {
        const values = target(el.id);
        const old = values.get(el.id);
        const data = { ...old, ...storedElement(kind, el), server_id: old?.server_id ?? `db-${kind}-${el.id}`, project_uuid: owner(el.id),
          ...(kind === 'spots' ? { acquisition_ids: old?.acquisition_ids ?? [] } : {}),
          ...(kind === 'grains' ? { retired: false, children: old?.children ?? [], ...(el.split_from ? { split_from: el.split_from } : {}), ...(el.merged_from ? { merged_from: el.merged_from } : {}) } : {}) };
        values.set(el.id, data); record(el.id, 'upsert', data);
      }
      project.elements[kind] = [...entries.values()];
      if (project.uuid !== null) mount.shared[kind] = [...shared.values()];
    }
    for (const child of changes.grains?.upsert ?? []) {
      for (const id of [...(child.split_from ? [child.split_from] : []), ...(child.merged_from ?? [])]) {
        const parent = [...project.elements.grains, ...mount.shared.grains].find(g => g.id === id);
        if (parent) parent.children = [...new Set([...(parent.children ?? []), child.id])];
      }
    }
  };

  const send = (res, status, body, type = 'application/json') => {
    res.statusCode = status;
    res.setHeader('Content-Type', type);
    res.end(Buffer.isBuffer(body) || typeof body === 'string' ? body : JSON.stringify(body));
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
    let path = decodeURIComponent(url.pathname.slice(PREFIX.length));
    let ns = '';
    const nm = path.match(/^\/ns\/([^/]+)(\/.*)$/);
    if (nm) [, ns, path] = nm;
    if (!states.has(ns)) states.set(ns, fresh());
    let state = states.get(ns);
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    if (req.method === 'OPTIONS') return send(res, 204, '', 'text/plain');
    state.requests.push({ method: req.method, path, auth: req.headers.authorization ?? null, access: url.searchParams.has('access') });
    let m;
    try {
      if (path === '/_fake/state') return send(res, 200, { published: state.published, spots: state.spots, syncs: state.syncs, offline: state.offline, requests: state.requests, atlases: [...state.atlases.keys()] });
      if (path === '/_fake/reset' && req.method === 'POST') {
        state = fresh();
        states.set(ns, state);
        return send(res, 200, { ok: true });
      }
      if (path === '/_fake/offline' && req.method === 'POST') {
        state.offline = JSON.parse((await body(req)).toString('utf8') || '{}').offline ?? true;
        return send(res, 200, { offline: state.offline });
      }
      // Test-only delta injection. Same change shape as /sync, no authentication required.
      if (path === '/_fake/atlas-edit' && req.method === 'POST') {
        const doc = JSON.parse((await body(req)).toString('utf8'));
        const sharedMount = doc.mount_id ?? MOUNTS[0].id;
        const project = doc.project_uuid === null ? { uuid: null, mount: sharedMount, elements: state.mounts.get(sharedMount).shared } : state.projects.get(doc.project_uuid ?? '01972c4e-0000-7000-8000-00000000a7a5');
        if (!project) return send(res, 404, { detail: 'unknown project' });
        const changes = doc.changes ?? doc;
        advance(state, project, { spots: { upsert: [], delete: [], ...changes.spots }, grains: { upsert: [], retire: [], delete: [], ...changes.grains }, labels: { upsert: [], delete: [], ...changes.labels } });
        if (project.uuid === null) state.mounts.get(project.mount).shared = project.elements;
        return send(res, 200, { revision: state.mounts.get(project.mount).revision });
      }
      if (state.offline) return send(res, 503, { detail: 'fake is offline' });
      if (path === '/v1/atlas/projects' && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        return send(res, 200, [...state.projects.values()].filter(p => p.name === url.searchParams.get('package')).map(p => ({ project_uuid: p.uuid, mount_id: p.mount, mount_label: MOUNTS.find(m => m.id === p.mount)?.label ?? p.mount, package_name: p.name, revision: state.mounts.get(p.mount).revision })));
      }
      if ((m = path.match(/^\/v1\/atlas\/projects\/([^/]+)(\/elements)?$/)) && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const p = state.projects.get(m[1]);
        if (!p) return send(res, 404, { detail: 'unknown project' });
        const mount = state.mounts.get(p.mount);
        const elements = elementsFor(state, p);
        const hashes = await hashesFor(elements);
        if (m[2]) {
          if (url.searchParams.has('revision') && Number(url.searchParams.get('revision')) !== mount.revision) return send(res, 404, { detail: 'older revisions use mount changes' });
          return send(res, 200, { mount_id: p.mount, revision: mount.revision, ...elements, hashes });
        }
        return send(res, 200, { project_uuid: p.uuid, mount_id: p.mount, mount_label: MOUNTS.find(m => m.id === p.mount)?.label ?? p.mount, atlas_version: p.version, revision: mount.revision, counts: Object.fromEntries(kinds.map(k => [k, storedElements(elements)[k].length])), hashes, changed_at: p.changed_at, changed_by: 'fake-user' });
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/changes$/)) && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const mount = state.mounts.get(m[1]);
        if (!mount) return send(res, 404, { detail: 'unknown mount' });
        const since = Number(url.searchParams.get('since'));
        return send(res, 200, { mount_id: m[1], base_revision: since, current_revision: mount.revision, changes: mount.changes.filter(c => c.revision > since) });
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/sync$/)) && req.method === 'POST') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const doc = JSON.parse((await body(req)).toString('utf8'));
        state.syncs.push({ mount_id: m[1], doc });
        let p = state.projects.get(doc.project_uuid);
        if (!MOUNTS.some(mount => mount.id === m[1] && mount.can_edit)) return send(res, 403, { detail: 'choose an editable mount' });
        if (p && p.mount !== m[1]) return send(res, 422, { detail: 'mount mismatch' });
        if (!['snapshot', 'delta'].includes(doc.mode) || (!p && doc.mode !== 'snapshot')) return send(res, 422, { detail: 'first sync requires snapshot mode' });
        const mount = state.mounts.get(m[1]);
        const since = mount.changes.filter(c => c.revision > (doc.base_revision ?? 0));
        if (p && doc.base_revision !== mount.revision && since.some(c => c.project_uuid === p.uuid || c.project_uuid === null)) return send(res, 409, { detail: 'mount changed', base_revision: doc.base_revision, current_revision: mount.revision, changes: since });
        if (!p) {
          p = { uuid: doc.project_uuid, mount: m[1], name: doc.project_uuid, version: null, elements: emptyElements(), changed_at: new Date().toISOString() };
          state.projects.set(p.uuid, p);
        }
        const parents = new Set(doc.grains.upsert.flatMap(g => [...(g.split_from ? [g.split_from] : []), ...(g.merged_from ?? [])]));
        for (const spot of elementsFor(state, p).spots.filter(s => parents.has(s.grain_id))) {
          const reassigned = doc.spots.upsert.find(s => s.id === spot.id);
          if (!reassigned?.grain_id || parents.has(reassigned.grain_id)) return send(res, 422, { detail: 'reassign every parent spot in the same request' });
        }
        const not_in_snapshot = Object.fromEntries(kinds.map(k => [k, doc.mode === 'snapshot' ? storedElements(elementsFor(state, p))[k].filter(e => !doc[k].upsert.some(s => s.id === e.id)).map(e => e.id) : []]));
        const previous_revision = mount.revision;
        const applied = Object.fromEntries(kinds.map(k => [k, { upsert: doc[k].upsert.length, [k === 'grains' ? 'retire' : 'delete']: doc.mode === 'snapshot' ? 0 : doc[k].delete.length }]));
        const changes = Object.fromEntries(kinds.map(k => [k, { upsert: doc[k].upsert, delete: doc.mode === 'snapshot' ? [] : doc[k].delete }]));
        advance(state, p, changes);
        return send(res, 200, { mount_id: p.mount, project_uuid: p.uuid, mode: doc.mode, previous_revision, revision: mount.revision, applied,
          ids: Object.fromEntries(kinds.map(k => [k, Object.fromEntries(doc[k].upsert.map(e => [e.id, elementsFor(state, p)[k].find(s => s.id === e.id).server_id]))])),
          acquisitions_linked: [], warnings: [], rebased_over: [...new Set(since.map(c => c.revision))], not_in_snapshot });
      }
      if ((m = path.match(/^\/atlas\/projects\/([^/]+)\/(.+)$/)) && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const a = state.atlases.get(m[1]);
        const f = a?.files.get(m[2]);
        if (!f) return send(res, 404, { detail: 'not found' });
        const type = m[2].endsWith('.webp') ? 'image/webp' : m[2].endsWith('.geojson') ? 'application/geo+json' : 'application/json';
        res.setHeader('Cache-Control', 'no-cache');
        return send(res, 200, f, type);
      }
      if (path === '/v1/mounts' && req.method === 'GET') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const session = url.searchParams.get('session');
        return send(res, 200, session ? MOUNTS.filter((m) => SESSION_LABELS.mounts[session]?.includes(m.id)) : MOUNTS);
      }
      if (path === '/v1/sessions' && req.method === 'GET') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        return send(res, 200, SESSION_LABELS.sessions);
      }
      if ((m = path.match(/^\/v1\/sessions\/([^/]+)\/acquisitions$/)) && req.method === 'GET') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const acquisitions = SESSION_LABELS.acquisitions[m[1]];
        return acquisitions ? send(res, 200, acquisitions) : send(res, 404, { detail: 'Not Found' });
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
        const zsha = sha(zip);
        const again = state.published.find((p) => p.project_id === atlas.project_id && p.sha256 === zsha);
        if (again) return send(res, 200, { ...again, created: false });
        const version = (prev?.version ?? 0) + 1;
        state.atlases.set(atlas.project_id, { files, version });
        const linkedProject = state.projects.get(atlas.project_id);
        if (linkedProject) linkedProject.version = version;
        const doc = JSON.parse(files.get('spots.json')?.toString('utf8') ?? '{}');
        const docs = Array.isArray(doc.documents) ? doc.documents : [doc];
        const n = docs.reduce((k, d) => k + (Array.isArray(d.spots) ? d.spots.length : 0), 0);
        const base = `/atlas/projects/${atlas.project_id}/`;
        const rec = {
          mount_id: m[1], project_id: atlas.project_id, version, sha256: zsha, url: `http://${req.headers.host}${PREFIX}${ns ? `/ns/${ns}` : ''}${base}`,
          created: true, is_current: true, file_count: files.size, total_bytes: zip.length, base_url: base, warnings: [],
          spots: { created: n, updated: 0, unchanged: 0, grains_created: 0, mount_id: m[1], project_id: atlas.project_id, acquisitions: [] },
          filename: file.name,
        };
        state.published.push(rec);
        return send(res, 201, rec);
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/results$/)) && req.method === 'GET') {
        // As laicpmsdb 1bf6ad4: the viewer's access token reads the results of the mount its
        // project is published on (the fixture's), 404 for any other mount.
        const viaAccess = !authed(req) && url.searchParams.get('access') === ACCESS;
        if (!authed(req) && !viaAccess) return send(res, 401, { detail: 'Unauthorized' });
        if (viaAccess && m[1] !== MOUNTS[0].id) return send(res, 404, { detail: 'Not Found' });
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
