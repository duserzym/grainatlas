// A stand-in for the laicpmsdb endpoints grainatlas calls, for `pnpm dev`, `pnpm preview` and
// the Playwright tests, mounted by vite.config.ts under /fake-laicpmsdb (never in a build).
// It follows laicpmsdb B1 (3f63ee9), docs/atlas-sync.md and schemas/openapi.json.
// It is a fixture-backed fake with limited validation, not a reference server:
//
//   GET  /v1/atlas/projects[/{uuid}]   candidates / AtlasProjectOut
//   POST /v1/atlas/projects/{uuid}/link register the local UUID on a mount
//   GET  /v1/atlas/projects/{uuid}/elements  MountElementsOut
//   GET  /v1/mounts/{id}/changes        ChangesOut
//   POST /v1/mounts/{id}/sync           SyncIn -> SyncOut / ConflictOut / 422
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

// B1 atlas publish imports legacy spot documents, not the project snapshot.
// Only spot-bearing grains exist, and spots are not linked to atlas grain UUIDs.
function publishedElements(project) {
  const local = storedElements(elementsOf(project)), uuid = project.uuid;
  const referenced = new Set(local.spots.map(s => s.grain_id));
  return {
    grains: local.grains.filter(g => referenced.has(g.id)).map(g => ({
      id: null, server_id: `db-grains-${g.id}`, project_uuid: uuid, area_id: g.area_id,
      label: g.label, number: null, polygon_um: null, centroid_um: null, area_um2: null,
      sample: null, artifact: '', retired: false, split_from: [], merged_from: [], children: [],
    })),
    spots: local.spots.map(s => ({ ...s, grain_id: null, server_id: `db-spots-${s.id}`, project_uuid: uuid, acquisition_ids: [] })),
    labels: [],
  };
}

export function createFake() {
  const states = new Map();
  const kinds = ['spots', 'grains', 'labels'];
  const fresh = (snapshot = false) => {
    const atlases = loadPreloaded();
    const projects = new Map();
    const mounts = new Map(MOUNTS.map(m => [m.id, { revision: 0, changes: [], shared: emptyElements(), current: null }]));
    for (const [uuid, atlas] of atlases) {
      const project = JSON.parse(atlas.files.get('project.json').toString('utf8'));
      const mount = project.laicpmsdb.mount_id ?? MOUNTS[0].id;
      const elements = snapshot ? storedElements(elementsOf(project)) : publishedElements(project);
      if (snapshot) for (const kind of kinds) elements[kind] = elements[kind].map(e => ({ ...e, server_id: `db-${kind}-${e.id}`, project_uuid: uuid, ...(kind === 'spots' ? { acquisition_ids: [] } : {}), ...(kind === 'grains' ? { retired: false, split_from: [], merged_from: [], children: [] } : {}), ...(kind === 'labels' ? { retired: false } : {}) }));
      projects.set(uuid, { uuid, mount, name: project.package.name, version: atlas.version || 1, elements, changed_at: new Date().toISOString() });
      mounts.get(mount).revision = 1; mounts.get(mount).current ??= uuid;
    }
    return { atlases, projects, mounts, published: [], spots: [], syncs: [], dryRuns: [], requests: [], offline: false, options: {} };
  };
  const allElements = (state, mount) => Object.fromEntries(kinds.map(k => [k, [...state.projects.values()].filter(q => q.mount === mount).flatMap(q => q.elements[k]).concat(state.mounts.get(mount).shared[k])]));
  const elementsFor = (state, project) => Object.fromEntries(kinds.map(k => [k, [...project.elements[k], ...state.mounts.get(project.mount).shared[k]]]));
  // Python's canonical JSON uses ASCII escapes and integral geometry floats.
  // These hashes stay opaque to the client; never use its narrowed-field hash here.
  const pythonJson = (value, float = false) => {
    if (Array.isArray(value)) return `[${value.map(v => pythonJson(v, float)).join(',')}]`;
    if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(k => `${pythonJson(k)}:${pythonJson(value[k], /_um$|_um2$/.test(k))}`).join(',')}}`;
    if (typeof value === 'number') return float && Number.isInteger(value) ? `${value}.0` : JSON.stringify(value);
    return JSON.stringify(value).replace(/[\u007f-\uffff]/g, c => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`);
  };
  const hashesFor = async elements => Object.fromEntries(kinds.map(k => [k, sha(elements[k].filter(e => e.id != null && !e.retired).map(e => {
    const { server_id, acquisition_ids, ...content } = e;
    return `${e.id} ${sha(pythonJson(content))}\n`;
  }).sort().join(''))]));
  const lookup = async (state, p, canEdit = true) => {
    const elements = allElements(state, p.mount), mount = state.mounts.get(p.mount);
    return { project_uuid: p.uuid, mount_id: p.mount, mount_label: MOUNTS.find(m => m.id === p.mount)?.label ?? p.mount,
      name: p.name, package_name: p.name, atlas_version: state.projects.get(mount.current)?.version ?? null, atlas_bundle_id: null, revision: mount.revision,
      counts: { ...Object.fromEntries(kinds.map(k => [k, elements[k].filter(e => !e.retired).length])), grains_retired: elements.grains.filter(g => g.retired).length, grains_without_atlas_id: elements.grains.filter(g => g.id === null).length },
      hashes: await hashesFor(elements), changed_at: p.changed_at ?? null, changed_by: 'fake-user', can_edit: canEdit && state.options.can_edit !== false,
      current_project_uuid: mount.current, project_uuids: [...state.projects.values()].filter(q => q.mount === p.mount).map(q => ({ project_uuid: q.uuid, name: q.name, package_name: q.name, is_current: mount.current === q.uuid })) };
  };
  const advance = (state, project, changes) => {
    const mount = state.mounts.get(project.mount);
    const revision = ++mount.revision;
    const at = new Date().toISOString();
    project.changed_at = at;
    for (const kind of kinds) {
      const entries = new Map(project.elements[kind].map(e => [e.id ?? e.server_id, e]));
      const shared = new Map(mount.shared[kind].map(e => [e.id ?? e.server_id, e]));
      const target = id => project.uuid !== null && shared.has(id) ? shared : entries;
      const owner = id => target(id) === shared ? null : project.uuid;
      const record = (id, op, data, project_uuid = owner(id)) => mount.changes.push({ revision, kind: kind.slice(0, -1), id, op, data: data ? structuredClone(data) : null, project_uuid, at, by: 'fake-user' });
      for (const id of [...(changes[kind]?.delete ?? []), ...(changes[kind]?.retire ?? [])]) {
        const values = target(id);
        const project_uuid = owner(id);
        if (kind !== 'spots') {
          const old = values.get(id);
          if (old) { const retired = { ...old, retired: true }; values.set(id, retired); record(id, 'retire', retired, project_uuid); }
        } else { values.delete(id); record(id, 'delete', null, project_uuid); }
      }
      for (const el of changes[kind]?.upsert ?? []) {
        let values = target(el.id);
        let old = values.get(el.id);
        let adopted = false;
        if (kind === 'grains' && el.id !== null && !old) {
          const label = el.label ?? `${el.area_id}/${el.number}`;
          for (const candidates of [entries, shared]) {
            const legacy = [...candidates.values()].find(g => g.id === null && !g.retired && g.label === label);
            if (legacy) { adopted = true; old = legacy; values = candidates; candidates.delete(legacy.server_id); break; }
          }
        }
        const data = { ...old, ...storedElement(kind, el), server_id: old?.server_id ?? `db-${kind}-${el.id}`, project_uuid: old?.project_uuid ?? project.uuid,
          ...(kind === 'spots' ? { acquisition_ids: el.acquisition_ids ?? old?.acquisition_ids ?? [] } : {}),
          ...(kind === 'grains' ? { sample: adopted ? old?.sample ?? null : el.sample !== undefined ? el.sample : old?.sample ?? null, retired: false, children: old?.children ?? [], split_from: el.split_from ? (Array.isArray(el.split_from) ? el.split_from : [el.split_from]) : old?.split_from ?? [], merged_from: el.merged_from ?? old?.merged_from ?? [] } : {}), ...(kind === 'labels' ? { retired: false } : {}) };
        if (kind === 'spots') delete data.sample;
        if (kind === 'labels') delete data.server_id;
        values.set(el.id ?? data.server_id, data); record(el.id ?? data.server_id, 'upsert', data);
      }
      project.elements[kind] = [...entries.values()];
      if (project.uuid !== null) mount.shared[kind] = [...shared.values()];
    }
    for (const child of changes.grains?.upsert ?? []) {
      for (const id of [...(child.split_from ? (Array.isArray(child.split_from) ? child.split_from : [child.split_from]) : []), ...(child.merged_from ?? [])]) {
        const parent = [...project.elements.grains, ...mount.shared.grains].find(g => g.id === id);
        if (parent) { parent.retired = true; parent.children = [...new Set([...(parent.children ?? []), child.id])]; mount.changes.push({ revision: mount.revision, kind: 'grain', id, op: 'retire', data: structuredClone(parent), project_uuid: parent.project_uuid, at: new Date().toISOString(), by: 'fake-user' }); }
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
      if (path === '/_fake/state') return send(res, 200, { published: state.published, spots: state.spots, syncs: state.syncs, dryRuns: state.dryRuns, offline: state.offline, requests: state.requests, atlases: [...state.atlases.keys()] });
      if (path === '/_fake/reset' && req.method === 'POST') {
        // Explicit already-snapshotted fixture for delta tests. Default reset and
        // normal startup always reproduce the real legacy publish state.
        const options = JSON.parse((await body(req)).toString('utf8') || '{}');
        state = fresh(options.snapshot === true);
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
      if (path === '/_fake/legacy-grain' && req.method === 'POST') {
        const doc = JSON.parse((await body(req)).toString('utf8'));
        const p = state.projects.get(doc.project_uuid);
        const grain = p?.elements.grains.find(g => g.id === doc.id);
        if (!grain) return send(res, 404, { detail: 'unknown grain' });
        grain.id = null;
        return send(res, 200, grain);
      }
      if (path === '/_fake/options' && req.method === 'POST') {
        Object.assign(state.options, JSON.parse((await body(req)).toString('utf8')));
        return send(res, 200, state.options);
      }
      if (state.offline) return send(res, 503, { detail: 'fake is offline' });
      if (path === '/v1/atlas/projects' && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const packageName = url.searchParams.get('package'), name = url.searchParams.get('name');
        if (!packageName && !name) return send(res, 422, { detail: ['package or name is required'] });
        const matches = (a, b) => !!b && a.toLowerCase() === b.toLowerCase();
        return send(res, 200, [...state.projects.values()].filter(p => matches(p.name, packageName) || matches(p.name, name) || matches(MOUNTS.find(m => m.id === p.mount)?.label ?? '', name)).map(p => ({ project_uuid: p.uuid, mount_id: p.mount, mount_label: MOUNTS.find(m => m.id === p.mount)?.label ?? p.mount, matched_by: matches(p.name, packageName) ? 'package' : matches(p.name, name) ? 'project_name' : 'mount_label', atlas_version: p.version, revision: state.mounts.get(p.mount).revision, spot_count: p.elements.spots.length, grain_count: p.elements.grains.filter(g => !g.retired).length, can_edit: authed(req) && state.options.can_edit !== false })));
      }
      if ((m = path.match(/^\/v1\/atlas\/projects\/([^/]+)\/link$/)) && req.method === 'POST') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const doc = JSON.parse((await body(req)).toString('utf8'));
        let p = state.projects.get(m[1]);
        if (state.options.link_conflict || (p && p.mount !== doc.mount_id)) return send(res, 409, { detail: 'Project is already linked to another mount.' });
        if (state.options.can_edit === false || !MOUNTS.some(q => q.id === doc.mount_id && q.can_edit)) return send(res, 403, { detail: 'Read-only mount' });
        if (!p) { p = { uuid: m[1], mount: doc.mount_id, name: m[1], version: null, elements: emptyElements(), changed_at: null }; state.projects.set(p.uuid, p); }
        const mount = state.mounts.get(p.mount);
        if (doc.make_current) mount.current = p.uuid;
        return send(res, 200, await lookup(state, p));
      }
      if ((m = path.match(/^\/v1\/atlas\/projects\/([^/]+)(\/elements)?$/)) && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const p = state.projects.get(m[1]);
        if (!p) return send(res, 404, { detail: 'unknown project' });
        const mount = state.mounts.get(p.mount);
        const elements = elementsFor(state, p);
        const hashes = await hashesFor(elements);
        if (m[2]) {
          if (url.searchParams.has('revision') && Number(url.searchParams.get('revision')) !== mount.revision) return send(res, 422, { detail: 'only the current revision is kept as a snapshot; use /changes?since=N' });
          return send(res, 200, { mount_id: p.mount, revision: mount.revision, project_uuids: [...state.projects.values()].filter(q => q.mount === p.mount).map(q => q.uuid), current_project_uuid: mount.current, ...elements, hashes });
        }
        return send(res, 200, await lookup(state, p, authed(req)));
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/changes$/)) && req.method === 'GET') {
        if (!authed(req) && url.searchParams.get('access') !== ACCESS) return send(res, 401, { detail: 'Unauthorized' });
        const mount = state.mounts.get(m[1]);
        if (!mount) return send(res, 404, { detail: 'unknown mount' });
        const since = Number(url.searchParams.get('since'));
        return send(res, 200, { mount_id: m[1], since, revision: mount.revision, changes: mount.changes.filter(c => c.revision > since) });
      }
      if ((m = path.match(/^\/v1\/mounts\/([^/]+)\/sync$/)) && req.method === 'POST') {
        if (!authed(req)) return send(res, 401, { detail: 'Unauthorized' });
        const doc = JSON.parse((await body(req)).toString('utf8'));
        (doc.dry_run ? state.dryRuns : state.syncs).push({ mount_id: m[1], doc: structuredClone(doc) });
        // Execute the same validation and prospective write on isolated state.
        // Neither revisions, adoption, audit rows nor registrations escape a preview.
        if (doc.dry_run) state = structuredClone(state);
        let p = state.projects.get(doc.project_uuid);
        if (state.options.validation_errors) return send(res, 422, { detail: state.options.validation_errors });
        if (state.options.can_edit === false) return send(res, 403, { detail: 'Read-only mount' });
        if (!Number.isInteger(doc.base_revision) || doc.base_revision < 0) return send(res, 422, { detail: ['base_revision must be a nonnegative integer'] });
        if (!MOUNTS.some(mount => mount.id === m[1] && mount.can_edit)) return send(res, 403, { detail: 'choose an editable mount' });
        if (p && p.mount !== m[1]) return send(res, 409, { detail: 'Project is linked to another mount', base_revision: doc.base_revision, current_revision: state.mounts.get(m[1]).revision, changes: [] });
        if (!['snapshot', 'delta'].includes(doc.mode) || (!p && doc.mode !== 'snapshot')) return send(res, 422, { detail: ['first sync requires snapshot mode'] });
        const mount = state.mounts.get(m[1]);
        const since = mount.changes.filter(c => c.revision > (doc.base_revision ?? 0));
        if (doc.base_revision > mount.revision || (doc.base_revision !== mount.revision && since.some(c => c.project_uuid === doc.project_uuid || c.project_uuid === null))) return send(res, 409, { detail: 'mount changed', base_revision: doc.base_revision, current_revision: mount.revision, changes: since });
        if (!p) {
          p = { uuid: doc.project_uuid, mount: m[1], name: doc.project_uuid, version: null, elements: emptyElements(), changed_at: new Date().toISOString() };
          state.projects.set(p.uuid, p); mount.current ??= p.uuid;
        }
        const labels = new Map();
        for (const grain of doc.grains.upsert) {
          const label = grain.label ?? elementsFor(state, p).grains.find(g => g.id === grain.id)?.label ?? `${grain.area_id}/${grain.number}`;
          if (!label || label.length > 50 || (labels.has(label) && labels.get(label) !== grain.id)) return send(res, 422, { detail: ['grain labels must be non-empty, unique and at most 50 characters'] });
          grain.label = label;
          labels.set(label, grain.id);
        }
        const parents = new Set(doc.grains.upsert.flatMap(g => [...(g.split_from ? [g.split_from] : []), ...(g.merged_from ?? [])]));
        const mountElements = allElements(state, p.mount);
        const retiring = new Set([...doc.grains.delete, ...parents]);
        const confirm = {
          retireGrains: mountElements.grains.filter(g => !g.retired && retiring.has(g.id)).length,
          grainsTotal: mountElements.grains.filter(g => !g.retired).length,
          deleteSpots: mountElements.spots.filter(s => doc.spots.delete.includes(s.id)).length,
          spotsTotal: mountElements.spots.length,
        };
        if (!doc.confirm_bulk && (confirm.retireGrains > Math.max(3, confirm.grainsTotal * .1) || confirm.deleteSpots > Math.max(3, confirm.spotsTotal * .1))) return send(res, 422, { detail: ['Bulk sync requires confirm_bulk'], confirm });
        const warnings = [...(state.options.warnings ?? [])];
        const keptNulls = [];
        // Existing null/absent geometry keeps the stored value; explicit detach
        // is the only way to clear a spot's grain link.
        for (const kind of kinds) doc[kind].upsert = doc[kind].upsert.map(input => {
          const old = elementsFor(state, p)[kind].find(e => e.id === input.id) ?? (kind === 'grains' ? elementsFor(state, p).grains.find(g => g.id === null && g.label === input.label) : undefined);
          const row = { ...old, ...input };
          const keep = kind === 'spots' ? ['grain_id', 'x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um'] : kind === 'grains' ? ['polygon_um', 'centroid_um', 'area_um2', 'number'] : [];
          for (const key of keep) if (input[key] == null) {
            if (old && input[key] === null) keptNulls.push(`${kind}.${input.id}.${key}`);
            row[key] = old?.[key] ?? null;
          }
          if (kind === 'spots') {
            if (input.detach_grain === true) row.grain_id = null;
            delete row.detach_grain;
          }
          if (input.sample && state.options.sample_unknown !== false) {
            warnings.push(`sample '${input.sample}' is unknown or ambiguous; no sample was assigned`);
            row.sample = null;
          }
          return row;
        });
        if (doc.spots.upsert.some(s => !s.name?.trim() || !Number.isFinite(s.x_um) || !Number.isFinite(s.y_um))) return send(res, 422, { detail: ['new spots require finite x,y and a non-empty name'] });
        if (keptNulls.length) warnings.push(`Kept null fields (${keptNulls.length}): ${keptNulls.join(', ')}`);
        for (const spot of elementsFor(state, p).spots.filter(s => parents.has(s.grain_id))) {
          const reassigned = doc.spots.upsert.find(s => s.id === spot.id);
          if (!reassigned?.grain_id || parents.has(reassigned.grain_id)) return send(res, 422, { detail: ['reassign every parent spot in the same request'] });
        }
        for (const id of doc.spots.delete) {
          const spot = elementsFor(state, p).spots.find(s => s.id === id);
          if (spot?.acquisition_ids?.length) return send(res, 422, { detail: [`spot ${spot.name} has measurements and cannot be deleted`] });
        }
        const previous_revision = mount.revision;
        const canonicalEqual = (kind, a, b) => pythonJson(storedElement(kind, a)) === pythonJson(storedElement(kind, b));
        const applied = Object.fromEntries(kinds.map(k => [k, { created: 0, updated: 0, unchanged: 0, ...(k === 'grains' ? { adopted: 0 } : {}), [k === 'spots' ? 'deleted' : 'retired']: doc[k].delete.length }]));
        for (const k of kinds) {
          const deleted = k === 'grains' ? [...new Set([...doc.grains.delete, ...parents])] : doc[k].delete;
          applied[k][k === 'spots' ? 'deleted' : 'retired'] = deleted.filter(id => elementsFor(state, p)[k].some(e => e.id === id && !e.retired)).length;
        }
        for (const k of kinds) for (const e of doc[k].upsert) {
          const old = elementsFor(state, p)[k].find(s => s.id === e.id);
          const adopted = k === 'grains' && !old && elementsFor(state, p).grains.some(g => g.id === null && g.label === (e.label ?? `${e.area_id}/${e.number}`));
          applied[k][adopted ? 'adopted' : old ? canonicalEqual(k, old, e) ? 'unchanged' : 'updated' : 'created']++;
        }
        const changes = Object.fromEntries(kinds.map(k => [k, {
          upsert: doc[k].upsert.filter(e => { const old = elementsFor(state, p)[k].find(s => s.id === e.id); return !old || !canonicalEqual(k, old, e); }),
          delete: doc[k].delete.filter(id => elementsFor(state, p)[k].some(e => e.id === id && !e.retired)),
        }]));
        if (kinds.some(k => changes[k].upsert.length || changes[k].delete.length)) advance(state, p, changes);
        const all = allElements(state, p.mount);
        const not_in_snapshot = Object.fromEntries(kinds.map(k => [k, doc.mode === 'snapshot' ? all[k].filter(e => !e.retired && !doc[k].upsert.some(s => s.id === e.id)).map(e => e.id ?? e.server_id) : []]));
        if (kinds.some(k => not_in_snapshot[k].length)) warnings.push('Some database elements were absent from the snapshot; they were kept.');
        return send(res, 200, { mount_id: p.mount, project_uuid: p.uuid, mode: doc.mode, previous_revision, revision: mount.revision, applied,
          ids: Object.fromEntries(kinds.map(k => [k, Object.fromEntries(doc[k].upsert.map(e => [e.id, k === 'labels' ? e.id : elementsFor(state, p)[k].find(s => s.id === e.id).server_id]))])),
          acquisitions_linked: 0, warnings: [...new Set(warnings)], rebased_over: previous_revision - doc.base_revision, not_in_snapshot, dry_run: !!doc.dry_run });
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
        if (linkedProject && kinds.some(k => linkedProject.elements[k].length)) linkedProject.version = version;
        else {
          const project = JSON.parse(files.get('project.json').toString('utf8'));
          const elements = publishedElements(project);
          state.projects.set(atlas.project_id, { uuid: atlas.project_id, mount: m[1], name: project.package.name, version, elements, changed_at: new Date().toISOString() });
          const mount = state.mounts.get(m[1]);
          mount.revision++; mount.current = atlas.project_id;
        }
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
