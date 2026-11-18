import assert from 'node:assert/strict';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import { registerHooks } from 'node:module';

// Exercise the actual rune-backed store and sync class without a worker or DOM.
// Relative extension resolution mirrors Vite; only the pixel core is replaced.
const hooks = registerHooks({
  resolve(specifier, context, next) {
    try { return next(specifier, context); }
    catch (error) {
      if (specifier.startsWith('.') && (!/\.[a-z]+$/.test(specifier) || specifier.endsWith('.svelte'))) return next(specifier + '.ts', context);
      throw error;
    }
  },
  load(url, context, next) {
    if (url.endsWith('/core/core.ts')) return { format: 'module', shortCircuit: true, source: 'export const newCore = () => Promise.resolve({}); export const isDesktop = () => false; export const isHosted = () => false;' };
    const result = next(url, context);
    if (url.endsWith('.svelte.ts') || url.endsWith('/remote.ts')) {
      const source = ts.transpileModule(String(result.source), { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } }).outputText;
      return { format: 'module', shortCircuit: true, source: url.endsWith('.svelte.ts') ? compileModule(source, { filename: new URL(url).pathname, generate: 'client' }).js.code : source };
    }
    return result;
  },
});
const { current } = await import('./store.svelte.ts');
const { saveBundleTones, neutralTone } = await import('./bundle-tone.ts');
const { grainMapWithoutBlankArtifacts } = await import('./sync.ts');
const { pixelSignature } = await import('./sync-pixels.ts');
const { flushWrites } = await import('./deferred-writes.ts');
const { appStorage } = await import('./app-storage.ts');
const { SyncSession } = await import('./sync-session.svelte.ts');
const { draftSyncStatus, pixelsPending } = await import('./sync-status.ts');
const { hasPendingChanges, listDrafts } = await import('./drafts.ts');
const { canonicalJson } = await import('./sync-hash.ts');
const { HostedSync } = await import('./hosted-sync.ts');
test.after(() => hooks.deregister());

const app = current();
const source = {
  schema_version: 1, uuid: 'synthetic', name: 'Synthetic mount', package: { name: 'Synthetic', path: '', kind: 'package' },
  areas: [{ id: 'area', name: 'Synthetic area', sample: 'Synthetic', params: { tone: neutralTone(), segment: {}, stitch: {}, photometric: {}, blend: {} }, mosaic: { sha256: 'pixels', w: 32, h: 24, um_per_px: 1 } }],
  settings: {}, upbdb: { mount_id: null },
  grainatlas_ui: { grains: { area: [{ id: 'grain', number: 1, polygon_um: [[0, 0], [10, 0], [0, 10]], centroid_um: [3, 3], area_um2: 50 }] }, spots: [], labels: [], tone: {} },
};
app.project = structuredClone(source); app.adopt(structuredClone(source));
// Track real persistence invocations without using the host's localStorage.
const saved = [], storage = new Map();
appStorage.setItem = (key, value) => storage.set(key, value);
appStorage.getItem = key => { flushWrites(); return storage.get(key) ?? null; };
appStorage.track = (_owner, kind, write) => { saved.push(kind); return write(); };

function oldFullProject() {
  // Previous serialization algorithm: deep-copy everything, then replace UI.
  const p = JSON.parse(JSON.stringify(app.project));
  for (const a of p.areas) {
    const segment = app.params[a.id]?.segment;
    if (segment) a.params.segment = { ...JSON.parse(JSON.stringify(segment)), signal: segment.signal ?? null };
  }
  p.settings = JSON.parse(JSON.stringify(app.settings));
  p.grainatlas_ui = { grains: grainMapWithoutBlankArtifacts(JSON.parse(JSON.stringify(app.grains))), spots: JSON.parse(JSON.stringify(app.spots)), labels: JSON.parse(JSON.stringify(app.labels)), tone: JSON.parse(JSON.stringify(app.tone)), overlays: Object.fromEntries(Object.entries(app.overlays).map(([k, o]) => [k, { ...JSON.parse(JSON.stringify(o)), url: null }])) };
  saveBundleTones(p, app.bundle && JSON.parse(JSON.stringify(app.bundle)), JSON.parse(JSON.stringify(app.params)), JSON.parse(JSON.stringify(app.tone)), JSON.parse(JSON.stringify(app.project)));
  return p;
}

test('fullProject serializes byte-identically while sharing immutable geometry', () => {
  const grains = app.grains;
  const plain = app.fullProject();
  assert.equal(JSON.stringify(plain), JSON.stringify(oldFullProject()));
  assert.equal(plain.grainatlas_ui.grains, grains);
  assert.equal(plain.grainatlas_ui.spots, app.spots);
  structuredClone(plain); // No Svelte proxies can reach worker messages.
  // Preserve insertion order even when the original UI key precedes settings,
  // or neither field is present in a legacy record.
  const previous = app.project;
  app.project = { grainatlas_ui: previous.grainatlas_ui, ...source };
  assert.equal(JSON.stringify(app.fullProject()), JSON.stringify(oldFullProject()));
  const { grainatlas_ui, settings, ...legacy } = source;
  app.project = legacy;
  assert.equal(JSON.stringify(app.fullProject()), JSON.stringify(oldFullProject()));
  app.project = previous;
  assert.deepEqual(app.project, source);
});
test('checkpoint, edit, undo and redo share unchanged grains and restore every collection', () => {
  const original = { grains: app.grains, spots: app.spots, labels: app.labels };
  app.checkpoint();
  app.spots = [{ id: 'spot', name: 'Synthetic_1', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 1 }];
  const placed = app.spots;
  app.checkpoint();
  app.spots = app.spots.map(s => ({ ...s, name: 'Synthetic_2' }));
  app.labels = [{ id: 'label', area_id: 'area', x_um: 1, y_um: 1, text: 'Synthetic' }];
  app.grains = { ...app.grains, area: app.grains.area.map(g => ({ ...g, kept: true })) };
  const edited = { grains: app.grains, spots: app.spots, labels: app.labels };
  app.undo();
  assert.equal(app.grains, original.grains); assert.equal(app.spots, placed); assert.equal(app.labels, original.labels);
  app.undo();
  assert.equal(app.spots, original.spots);
  app.redo(); app.redo();
  for (const kind of ['grains', 'spots', 'labels']) assert.equal(app[kind], edited[kind]);
  assert.equal(JSON.stringify(app.fullProject()), JSON.stringify(oldFullProject()));
  app.flushPersistence();
  assert.deepEqual(saved, ['project', 'draft']);
  assert.equal(storage.get('grainatlas.project.Synthetic'), JSON.stringify(app.fullProject()));
  const draft = JSON.parse(storage.get('grainatlas.draft.synthetic'));
  assert.equal(draft.pending.spots[0].name, 'Synthetic_2');
  assert.equal(draft.pending.labels[0].text, 'Synthetic');
});
test('sync grain arrays and diffs survive spot edits and draft-only changes', () => {
  const grains = app.sync.elements.grains, diff = app.sync.localChanges.grains, more = app.sync.databaseMore.grains;
  const before = app.sync.counts.spots;
  app.spots = [...app.spots, { ...app.spots[0], id: 'spot2', name: 'Synthetic_3' }];
  assert.equal(app.sync.elements.grains, grains);
  assert.equal(app.sync.databaseMore.grains, more);
  assert.equal(app.sync.localChanges.grains, diff);
  assert.equal(app.sync.counts.spots, before + 1);
  app.sync.track();
  assert.equal(app.sync.localChanges.grains, diff);
  app.flushPersistence();
});

test('pixel bookkeeping matches fullProject with and without bundle tone, ignoring spot edits', () => {
  app.pkg = { name: 'Synthetic' };
  app.stitched = { area: { mosaic: source.areas[0].mosaic } };
  app.remote = { readOnly: false }; app.bundle = { area: { sha256: 'pixels', rawCurve: neutralTone(), rawDisplay: neutralTone() } };
  app.tone = { area: { ...neutralTone(), gamma: 2 } };
  const p = app.fullProject();
  assert.equal(JSON.stringify(p), JSON.stringify(oldFullProject()));
  app.sync.draft = { ...app.sync.draft, pixels_base: pixelSignature(p) };
  assert.equal(app.sync.pixelsChanged, false);
  app.spots = app.spots.map(s => ({ ...s, x_um: s.x_um + 1 }));
  assert.equal(app.sync.pixelsChanged, false);
  app.tone.area.gamma = 3;
  assert.equal(app.sync.pixelsChanged, true);
  app.bundle = null; app.remote = null;
  const local = app.fullProject();
  local.areas = local.areas.map(a => ({ ...a, params: app.params[a.id] ?? a.params, mosaic: app.stitched[a.id]?.mosaic ?? a.mosaic }));
  app.sync.draft = { ...app.sync.draft, pixels_base: pixelSignature(local) };
  assert.equal(app.sync.pixelsChanged, false);
  app.flushPersistence();
});

test('Open list, unloaded session and tracked draft agree on the same status fixtures', () => {
  const p = structuredClone(source);
  p.upbdb.mount_id = 'mount';
  p.grainatlas_ui = { grains: {}, spots: [], labels: [], tone: {} };
  const key = pixelSignature(p);
  const oldKey = canonicalJson(JSON.parse(key).map(a => ({ ...a, stitch: { changed: true }, photometric: { gain: 2 }, blend: { changed: true } })));
  const legacyF32Key = canonicalJson(JSON.parse(oldKey).map(a => ({ ...a,
    tone: { ...a.tone, grain_norm: { ...a.tone.grain_norm, strength: 0.800000012 } } })));
  const changed = structuredClone(p); changed.areas[0].mosaic.sha256 = 'new-pixels';
  const toned = structuredClone(p); toned.grainatlas_ui.tone.area = { ...neutralTone(), gamma: 2 };
  const edited = structuredClone(p); edited.grainatlas_ui.labels = [{ id: 'label', area_id: 'area', x_um: 1, y_um: 1, text: 'Synthetic edit' }];
  const copyKey = 'grainatlas.project.Synthetic';
  const base = { spots: [], grains: [], labels: [] };
  const clean = { uuid: p.uuid, name: p.name, server: null, mount: 'mount', base_revision: 2, base,
    updated_at: '2026-01-01T00:00:00Z', changes: { spots: 0, grains: 0, labels: 0 }, pending: null,
    synced: true, pixels_base: oldKey, pixels_changed: true, source_keys: [copyKey] };
  const fixtures = [
    { name: 'stale legacy flag', copy: p, pixels: false, pending: false },
    { name: 'legacy f32 base matches decimal copy', copy: p, patch: { pixels_base: legacyF32Key }, pixels: false, pending: false },
    { name: 'legacy false flag with changed copy', copy: changed, patch: { pixels_changed: false }, pixels: false, pending: false },
    { name: 'legacy undefined flag with changed copy', copy: changed, patch: { pixels_changed: undefined }, pixels: false, pending: false },
    { name: 'changed mosaic', copy: changed, pixels: true, pending: true },
    { name: 'changed display tone', copy: toned, pixels: true, pending: true },
    { name: 'legacy false flag with first mosaic', copy: p, patch: { pixels_base: '[]', pixels_changed: false }, pixels: false, pending: false },
    { name: 'recorded first mosaic after an empty published project', copy: p, patch: { pixels_base: '[]', pixels_key: key, pixels_changed: false }, pixels: true, pending: true },
    { name: 'element edit', copy: edited, local: edited, patch: { changes: { spots: 0, grains: 0, labels: 1 }, pending: { ...base, labels: edited.grainatlas_ui.labels } }, pixels: false, pending: true },
    { name: 'pending snapshot without counts', copy: edited, local: edited, patch: { pending: { ...base, labels: edited.grainatlas_ui.labels } }, pixels: false, pending: true },
    { name: 'legacy elements in copy', copy: edited, local: edited, patch: { pending: undefined }, pixels: false, pending: true },
    { name: 'saved key beats older clean copy', copy: p, patch: { pixels_key: pixelSignature(changed) }, pixels: true, pending: true },
    { name: 'saved clean key beats changed copy', copy: changed, patch: { pixels_key: key }, pixels: false, pending: false },
    { name: 'unknown base', copy: changed, patch: { pixels_base: null }, pixels: false, pending: false },
    { name: 'damaged base', copy: changed, patch: { pixels_base: 'broken' }, pixels: false, pending: false },
    { name: 'unpublished legacy images', copy: p, patch: { pixels_base: undefined }, pixels: true, pending: true },
    { name: 'unpublished without image changes', copy: p, patch: { pixels_base: undefined, pixels_changed: false }, pixels: false, pending: false },
    { name: 'legacy flag without copy', patch: { source_keys: [] }, pixels: true, pending: true },
    { name: 'legacy clean without copy', patch: { source_keys: [], pixels_changed: false }, pixels: false, pending: false },
    { name: 'unlinked copy', copy: p, patch: { mount: null, pixels_changed: false }, pixels: false, pending: true },
    { name: 'unlinked candidate', patch: { mount: null, source_keys: [], pixels_changed: false, link_candidates: [{}] }, pixels: false, pending: true },
  ];
  for (const f of fixtures) {
    flushWrites(); storage.clear(); saved.length = 0;
    const draft = { ...clean, ...f.patch };
    if (f.copy) storage.set(copyKey, JSON.stringify(f.copy));
    storage.set('grainatlas.draft.synthetic', JSON.stringify(draft));
    storage.set('grainatlas.drafts', JSON.stringify([draft.uuid]));
    app.project = structuredClone(f.local ?? p); app.adopt(app.project);
    app.pkg = null; app.bundle = null; app.remote = null; app.stitched = {};
    const session = new SyncSession(app);
    session.draft = draft; session.registered = true; session.reachable = true;
    session.status = { revision: 2, atlas_version: 1 };
    const status = draftSyncStatus(draft);
    assert.equal(status.pixels, f.pixels, f.name);
    assert.equal(status.pending, f.pending, f.name);
    assert.equal(hasPendingChanges(draft), f.pending, f.name);
    assert.equal(listDrafts().filter(d => hasPendingChanges(d)).length, f.pending ? 1 : 0, f.name);
    const events = [];
    new HostedSync(event => events.push(event)).drafts([draft]);
    assert.equal(events[0].drafts.length, f.pending ? 1 : 0, f.name);
    if (f.pending) assert.equal(events[0].drafts[0].changes.pixels, f.pixels, f.name);
    const before = [...storage];
    session.track(false);
    assert.equal(session.pixelsChanged, f.pixels, f.name);
    assert.equal(session.unsynced, f.pending, f.name);
    assert.equal(session.detailText.includes('New images not published'), f.pixels, f.name);
    assert.equal(session.text, f.pending ? 'Saved' : 'Up to date', f.name);
    if (!f.pending) assert.equal(session.detailText, 'In sync · v1 r2', f.name);
    assert.equal(session.draft.pixels_changed, f.pixels, f.name);
    assert.deepEqual([...storage], before, 'tracking defers persistence until after paint');
    flushWrites();
    assert.equal(JSON.parse(storage.get('grainatlas.draft.synthetic')).pixels_changed, f.pixels, f.name);
    if (f.copy) assert.equal(storage.get(copyKey), JSON.stringify(f.copy), 'healing retains the project copy');
  }
});

test('live pixels override recorded keys and copies, and track the current key after paint', async () => {
  flushWrites(); storage.clear(); saved.length = 0;
  app.project = structuredClone(source); app.adopt(app.project);
  app.pkg = { name: 'Synthetic' }; app.bundle = null; app.remote = null;
  app.stitched = { area: { mosaic: structuredClone(source.areas[0].mosaic) } };
  const key = pixelSignature(app.fullProject());
  const session = new SyncSession(app);
  session.draft = { uuid: source.uuid, name: source.name, mount: 'mount', base: session.elements, synced: true,
    changes: { spots: 0, grains: 0, labels: 0 }, pending: null, pixels_base: key, pixels_key: '[]', pixels_changed: true };
  assert.equal(session.pixelsChanged, false, 'live clean key wins over saved changed key');
  // UI processing controls alone must not recreate the stale flag after sync.
  app.params.area.stitch = { changed: true }; app.params.area.photometric = { changed: true }; app.params.area.blend = { changed: true };
  assert.equal(session.pixelsChanged, false);
  app.stitched.area.mosaic.sha256 = 'new-pixels';
  assert.equal(session.pixelsChanged, true, 'a real correction changes the mosaic hash');
  session.registered = true; session.reachable = true;
  await session.action();
  assert.equal(session.dialog, 'pixels', 'real image changes still open the pixels dialog');
  saved.length = 0;
  session.track(false);
  assert.equal(saved.length, 0);
  assert.equal(session.draft.pixels_key, pixelSignature({ ...app.fullProject(), areas: app.project.areas.map(a => ({ ...a, mosaic: app.stitched[a.id].mosaic })) }));
  assert.equal(session.draft.pixels_changed, true);
  flushWrites();
  assert.equal(JSON.parse(storage.get('grainatlas.draft.synthetic')).pixels_key, session.draft.pixels_key);
  app.pkg = null; app.stitched = {};
  assert.equal(session.pixelsChanged, true, 'recorded key survives unloading');
  assert.equal(hasPendingChanges(session.draft), true);
  assert.equal(pixelsPending({ ...session.draft, pixels_base: null }), false);
});

test('unloaded session status reads do not flush a pending project serialisation', () => {
  flushWrites(); storage.clear(); saved.length = 0;
  app.project = structuredClone(source); app.adopt(app.project);
  app.pkg = null; app.bundle = null; app.remote = null; app.stitched = {};
  const p = app.fullProject();
  storage.set('grainatlas.project.Synthetic', JSON.stringify(p));
  const session = new SyncSession(app);
  session.draft = { uuid: source.uuid, name: source.name, mount: 'mount', base: session.elements, synced: true,
    changes: { spots: 0, grains: 0, labels: 0 }, pending: null, pixels_base: pixelSignature(p), pixels_changed: true,
    source_keys: ['grainatlas.project.Synthetic'] };
  session.track(false); flushWrites(); saved.length = 0;
  app.labels = [{ id: 'label', area_id: 'area', x_um: 1, y_um: 1, text: 'Synthetic' }];
  app.persist();
  assert.equal(session.pixelsChanged, false);
  assert.equal(session.unsynced, true);
  assert.equal(session.detailText.includes('New images not published'), false);
  assert.equal(saved.length, 0, 'status reads leave project and draft writes coalesced after paint');
  flushWrites();
  assert.deepEqual(saved, ['project', 'draft']);
});

test('opening a view resolves saved pixel status even though view mode does not track', async () => {
  flushWrites(); storage.clear(); saved.length = 0;
  app.project = structuredClone(source); app.adopt(app.project);
  app.pkg = null; app.bundle = null; app.remote = null; app.stitched = {}; app.viewMode = true;
  const p = app.fullProject(), changed = structuredClone(p);
  changed.areas[0].mosaic.sha256 = 'new-pixels';
  const draft = { uuid: source.uuid, name: source.name, mount: 'mount', base: { spots: [], grains: [], labels: [] }, synced: true,
    changes: { spots: 0, grains: 0, labels: 0 }, pending: null, pixels_base: pixelSignature(p), pixels_key: pixelSignature(changed), pixels_changed: false };
  storage.set('grainatlas.draft.synthetic', JSON.stringify(draft));
  storage.set('grainatlas.drafts', JSON.stringify([draft.uuid]));
  const session = new SyncSession(app);
  session.api = async () => ({ lookup: async () => null, candidates: async () => [] });
  try {
    await session.check();
    assert.equal(session.pixelsChanged, true);
    assert.equal(hasPendingChanges(session.draft), true);
    assert.equal(session.detailText.includes('New images not published'), true);
    assert.equal(session.unsynced, false, 'view mode still suppresses the quit warning');
    session.save({ pixels_base: null });
    assert.equal(session.pixelsChanged, false, 'a newly learned unknown base refreshes the view');
  } finally { app.viewMode = false; flushWrites(); }
});

function quietSession(uuid = 'quiet-synthetic') {
  app.sync.dispose();
  app.flushPersistence(); storage.clear(); saved.length = 0;
  app.project = { ...structuredClone(source), uuid }; app.adopt(app.project);
  app.pkg = null; app.bundle = null; app.remote = null; app.stitched = {};
  app.viewMode = false; app.storedCopy = null; app.loadingProject = false;
  app.link.syncAutomatically = true; app.error = null;
  const session = new SyncSession(app);
  app.sync = session;
  const base = Object.fromEntries(Object.entries(JSON.parse(JSON.stringify(session.elements))).map(([kind, rows]) => [kind, rows.map(row => ({ ...row, project_uuid: uuid }))]));
  const hashes = { spots: 's1', grains: 'g1', labels: 'l1' };
  session.draft = { uuid, name: 'Synthetic', mount: 'mount', server: 'https://quiet.test', base, base_revision: 1, base_hashes: hashes,
    synced: true, changes: { spots: 0, grains: 0, labels: 0 }, pending: null, pixels_base: null, updated_at: new Date().toISOString(), server_can_edit: true };
  session.status = { project_uuid: uuid, mount_id: 'mount', revision: 1, atlas_version: 1, can_edit: true, counts: { spots: 0, grains: 1, labels: 0 }, hashes };
  session.registered = true; session.reachable = true;
  return { session, base, hashes };
}
function addQuietSpot() {
  app.spots = [...app.spots, { id: 'quiet-spot', name: 'Synthetic spot', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 1 }];
}
function quietReply(body, revision = 2) {
  return { ok: true, revision, previous_revision: 1, mode: body.mode, applied: {
    spots: { created: 1, updated: 0, deleted: 0 }, grains: { created: 0, adopted: 0, updated: 0, retired: 0 }, labels: { created: 0, updated: 0, retired: 0 } },
    not_in_snapshot: { spots: [], grains: [], labels: [] }, complete: true, warnings: [], rebased_over: 0 };
}
test('an older copy pauses all sends, restores base elements and drops a queued deletion', async t => {
  const { session, base, hashes } = quietSession('older-copy-synthetic');
  t.after(() => session.dispose());
  base.spots = [{ id: 'shared-spot', name: 'Synthetic spot', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 1 }];
  base.labels = [{ id: 'shared-label', area_id: 'area', x_um: 1, y_um: 1, text: 'Synthetic label' }];
  session.draft = { ...session.draft, base, pending_request: { body: { spots: { delete: ['shared-spot'] } } } };
  storage.set('grainatlas.draft.older-copy-synthetic', JSON.stringify(session.draft));
  storage.set('grainatlas.drafts', JSON.stringify([session.draft.uuid]));
  app.grains = {}; app.project.upbdb.mount_id = null;
  session.opened();
  let writes = 0;
  const status = session.status;
  const server = { ...base, revision: 1, hashes };
  session.api = async () => ({ lookup: async () => status, elements: async () => structuredClone(server),
    sync: async (_mount, body) => { writes++; assert.deepEqual(body.spots.delete, []); return quietReply(body); } });
  await session.check();
  assert.deepEqual(session.staleOpen, { spots: ['shared-spot'], grains: ['grain'], labels: ['shared-label'] });
  assert.equal(session.text, 'Needs your attention'); assert.equal(session.backgroundPending, false);
  await session.runAutomatic(); await session.sync();
  assert.equal(writes, 0);
  session.bringBack();
  assert.equal(app.spots[0].id, 'shared-spot'); assert.equal(app.grains.area[0].id, 'grain');
  assert.equal(app.labels[0].id, 'shared-label');
  assert.equal(session.staleOpen, null); assert.equal(session.draft.pending_request, null);
  await session.sync(); assert.equal(writes, 1);
});
test('a deletion made after opening stays an ordinary edit through the first check', async t => {
  const { session, base, hashes } = quietSession('current-copy-synthetic');
  t.after(() => session.dispose());
  addQuietSpot(); base.spots = JSON.parse(JSON.stringify(app.spots));
  session.draft = { ...session.draft, base };
  storage.set('grainatlas.draft.current-copy-synthetic', JSON.stringify(session.draft));
  storage.set('grainatlas.drafts', JSON.stringify([session.draft.uuid]));
  session.opened(); assert.equal(session.staleOpen, null);
  app.checkpoint(); app.spots = []; session.track();
  const status = session.status;
  session.api = async () => ({ lookup: async () => status, elements: async () => ({ ...base, revision: 1, hashes }) });
  await session.check();
  assert.equal(session.staleOpen, null); assert.deepEqual(session.localChanges.spots.delete, ['quiet-spot']);
});
test('bringing back an opening grain retirement also cancels the lineage that would retire it again', t => {
  const { session, base } = quietSession('old-lineage-synthetic');
  t.after(() => session.dispose());
  const parent = base.grains[0];
  app.grains = { area: [{ ...parent, id: 'split-child', split_from: parent.id },
    { ...parent, id: 'merged-child', merged_from: [parent.id] }] };
  session.staleOpen = { spots: [], grains: [parent.id], labels: [] };
  session.bringBack();
  assert.ok(app.grains.area.some(g => g.id === parent.id));
  assert.equal(app.grains.area.find(g => g.id === 'split-child').split_from, undefined);
  assert.deepEqual(app.grains.area.find(g => g.id === 'merged-child').merged_from, []);
});
test('automatic deletion of six base spots waits for a choice before any request', async t => {
  const { session, base } = quietSession('many-removals-synthetic');
  t.after(() => session.dispose());
  addQuietSpot();
  base.spots = Array.from({ length: 6 }, (_, n) => ({ ...app.spots[0], id: `shared-${n}` }));
  session.draft = { ...session.draft, base };
  app.spots = []; session.track();
  let requests = 0;
  session.api = async () => { requests++; throw new Error('No request before a choice'); };
  await session.runAutomatic();
  assert.equal(requests, 0); assert.equal(session.staleBulk, true);
  assert.equal(session.staleOpen.spots.length, 6); assert.equal(session.text, 'Needs your attention');
  session.bringBack(); assert.equal(app.spots.length, 6); assert.equal(session.staleOpen, null);
});
test('spot deletion waits at most eight seconds even if the measured-list body stalls', async t => {
  (await import('./sync-session.svelte.ts')).forgetUnreachable();
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'] });
  const { SyncApi } = await import('./sync-api.ts');
  const { session } = quietSession('stalled-measurements-synthetic');
  t.after(() => session.dispose());
  addQuietSpot(); session.track(false);
  const status = session.status;
  const fetch = t.mock.method(globalThis, 'fetch', async url => String(url).endsWith('/elements')
    ? new Response(new ReadableStream()) : Response.json(status));
  session.api = async () => new SyncApi('https://quiet.test', null);
  const checking = session.check();
  assert.equal(session.refuseSpotDeletion(['quiet-spot']), true);
  for (let i = 0; i < 50 && fetch.mock.callCount() < 2; i++) await Promise.resolve();
  assert.equal(fetch.mock.callCount(), 2);
  // Allow the second response's body reader to start before advancing time.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  t.mock.timers.tick(8000);
  assert.equal(session.checking, true, 'the body may take longer than the response timeout');
  assert.equal(session.refuseSpotDeletion(['quiet-spot']), false);
  t.mock.timers.tick(22000); await checking;
  assert.equal(session.text, 'Offline, will sync later');
});
test('saving a linked project while the database is known unreachable skips server checks and finishes at once', async t => {
  const { saveProjectFile } = await import('./actions.ts');
  const { session } = quietSession('offline-save-synthetic');
  const previousCore = app.core, previousServer = app.link.server;
  app.link.server = 'https://quiet.test'; app.link.syncOnSave = true;
  app.project.upbdb.mount_id = 'mount';
  let savedFile = false, release;
  const stalled = new Promise(resolve => { release = resolve; });
  app.core = Promise.resolve({ kind: 'wasm', checkProject: async () => [],
    saveProjectFile: async () => { savedFile = true; } });
  const fetch = t.mock.method(globalThis, 'fetch', () => { throw new Error('Save must not fetch acquisition labels'); });
  session.api = async () => { await stalled; throw new Error('Synthetic outage'); };
  session.reachable = false; // an earlier check found the database unreachable
  try {
    await saveProjectFile(app, false);
    // Known offline: the file is saved at once and no sync or label fetch is attempted.
    assert.equal(savedFile, true);
    assert.equal(fetch.mock.callCount(), 0);
    assert.match(app.notice, /Database checks skipped while saving/);
  } finally {
    release(); await session.settled(); session.dispose();
    app.core = previousCore; app.link.server = previousServer;
  }
});
test('real session coalesces automatic edits and stops for hosted, read-only, settings and attention', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { session } = quietSession();
  let runs = 0;
  session.sync = async () => { runs++; };
  addQuietSpot(); session.track();
  assert.equal(session.text, 'Saved');
  t.mock.timers.tick(2000); session.track();
  t.mock.timers.tick(2999); assert.equal(runs, 0);
  t.mock.timers.tick(1); assert.equal(runs, 1);
  for (const gate of ['hosted', 'readOnly', 'setting', 'confirmation', 'review', 'busy']) {
    app.hosted = gate === 'hosted'; app.viewMode = gate === 'readOnly'; app.link.syncAutomatically = gate !== 'setting';
    session.confirmation = gate === 'confirmation' ? {} : null;
    session.review = gate === 'review' ? {} : null; session.busy = gate === 'busy';
    session.refreshAutomatic(); session.track(); t.mock.timers.tick(3000);
    assert.equal(runs, 1, gate);
  }
  session.dispose(); app.hosted = false; app.viewMode = false; app.link.syncAutomatically = true; session.busy = false;
});
test('actual session persists exact request before sending, replays after restart, preserves newer edits', async t => {
  const { session, base, hashes } = quietSession('recovery-synthetic');
  t.after(() => session.dispose());
  const { SyncApi } = await import('./sync-api.ts');
  const calls = []; let server = { ...base, revision: 1, hashes };
  let fail = true;
  t.mock.method(console, 'warn', () => {});
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    calls.push([String(url), init]);
    if (String(url).endsWith('/sync')) {
      const persisted = JSON.parse(storage.get('grainatlas.draft.recovery-synthetic'));
      assert.equal(persisted.pending_request.id, init.headers['Idempotency-Key']);
      assert.deepEqual(persisted.pending_request.body.spots.upsert[0].name, 'Synthetic spot');
      if (fail) return Response.json({ detail: 'Synthetic relay down' }, { status: 503 });
      const body = JSON.parse(init.body);
      server = { ...base, spots: body.spots.upsert.map(s => ({ ...s, project_uuid: 'recovery-synthetic' })), revision: 2, hashes: { ...hashes, spots: 's2' } };
      return Response.json(quietReply(body), { headers: { 'Idempotent-Replayed': 'true' } });
    }
    if (String(url).endsWith('/elements')) return Response.json(server);
    return Response.json({ ...session.status, revision: server.revision });
  });
  session.api = async () => new SyncApi('https://quiet.test', null);
  addQuietSpot(); session.track();
  await session.runAutomatic(); session.dispose(); flushWrites();
  assert.equal(session.text, 'Offline, will sync later');
  assert.equal(app.error, null, 'automatic transient failures stay out of the error banner');
  const stored = JSON.parse(storage.get('grainatlas.draft.recovery-synthetic'));
  assert.ok(stored.pending_request.body); assert.ok(stored.pending.spots.length);
  // Edit after the failed request; recovery must send the old snapshot first.
  app.spots = app.spots.map(s => ({ ...s, name: 'Newer local name' }));
  const restarted = new SyncSession(app);
  app.sync = restarted;
  t.after(() => restarted.dispose());
  restarted.draft = stored; restarted.status = session.status; restarted.registered = true; restarted.reachable = true;
  restarted.api = session.api;
  restarted.track(); fail = false;
  const before = calls.length, start = performance.now();
  await restarted.runAutomatic(); restarted.dispose();
  const writes = calls.filter(([url]) => url.endsWith('/sync')).map(([, init]) => init);
  assert.equal(writes.length, 2);
  assert.equal(writes[0].headers['Idempotency-Key'], writes[1].headers['Idempotency-Key']);
  assert.equal(writes[0].body, writes[1].body);
  assert.equal(app.spots[0].name, 'Newer local name'); assert.equal(restarted.text, 'Saved', restarted.error);
  assert.equal(restarted.draft.pending_request, null); assert.equal(restarted.draft.base_revision, 2);
  assert.equal(calls[before][0].endsWith('/sync'), true, 'recovery replays before fetching any new snapshot');
  t.diagnostic(`Recovery: ${calls.length - before} requests, ${(performance.now() - start).toFixed(1)} ms; old rebuilt path requires 4 requests`);
});
test('sync on save retries after a successful check when automatic sync is off', async t => {
  const { SyncNetworkError } = await import('./sync-api.ts');
  const { session, base, hashes } = quietSession('save-check-synthetic');
  t.after(() => { session.dispose(); app.link.syncAutomatically = true; });
  app.link.syncAutomatically = false; app.link.syncOnSave = true;
  let writes = 0;
  const server = { ...base, revision: 1, hashes };
  const status = session.status;
  session.api = async () => ({ elements: async () => structuredClone(server), lookup: async () => ({ ...status, revision: server.revision }),
    sync: async (_mount, body) => {
      if (++writes === 1) throw new SyncNetworkError('Synthetic interruption');
      server.spots = body.spots.upsert.map(s => ({ ...s, project_uuid: session.draft.uuid })); server.revision = 2;
      return quietReply(body);
    } });
  session.track(false); await session.check();
  addQuietSpot();
  await session.afterSave();
  assert.equal(writes, 1); assert.equal(session.pendingSync, true);
  await session.check();
  assert.equal(writes, 2); assert.equal(session.pendingSync, false);
  assert.equal(session.text, 'Up to date', session.error);
});
test('exhausted in-progress replies back off and retry the same persisted write', async t => {
  const { SyncApi } = await import('./sync-api.ts');
  const { session, base, hashes } = quietSession('in-progress-synthetic');
  t.after(() => session.dispose());
  let pending = true;
  const requests = [];
  const server = { ...base, revision: 1, hashes };
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    if (!String(url).endsWith('/sync')) return Response.json(String(url).endsWith('/elements') ? server : { ...session.status, revision: server.revision });
    requests.push([init.headers['Idempotency-Key'], init.body]);
    if (pending) return Response.json({ code: 'idempotency_in_progress' }, { status: 409, headers: { 'Retry-After': '0' } });
    const body = JSON.parse(init.body);
    server.spots = body.spots.upsert.map(s => ({ ...s, project_uuid: session.draft.uuid })); server.revision = 2;
    return Response.json(quietReply(body));
  });
  session.api = async () => new SyncApi('https://quiet.test', null);
  addQuietSpot(); session.track();
  await session.runAutomatic();
  assert.equal(requests.length, 31);
  assert.equal(session.text, 'Offline, will sync later');
  assert.equal(app.error, null);
  assert.equal(session.pendingSync, true);
  assert.equal(session.backgroundPending, true);
  assert.equal(session.draft.pending_request.id, requests[0][0]);
  pending = false; await session.runAutomatic();
  assert.equal(requests.length, 32);
  assert.ok(requests.every(r => r[0] === requests[0][0] && r[1] === requests[0][1]));
  assert.equal(session.draft.pending_request, null);
  assert.equal(session.text, 'Up to date', session.error);
});
test('background sync combines non-overlapping remote edits, but registered spot collisions need attention', async t => {
  for (const variant of ['rows', 'fields', 'collision']) {
    const collision = variant === 'collision';
    const { session, base, hashes } = quietSession(`merge-synthetic-${variant}`);
    t.after(() => session.dispose());
    const spot = { project_uuid: session.draft.uuid, id: 'registered-spot', name: 'Synthetic spot', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 1, acquisition_ids: ['synthetic-acquisition'] };
    base.spots = [spot]; session.draft = { ...session.draft, base };
    app.spots = [{ ...spot, name: 'Local name' }];
    const remote = { ...base, spots: collision ? [{ ...spot, name: 'Shared name' }] : variant === 'fields' ? [{ ...spot, diameter_um: 3 }] : base.spots,
      labels: variant !== 'rows' ? [] : [{ project_uuid: session.draft.uuid, id: 'remote-label', area_id: 'area', x_um: 0, y_um: 0, text: 'Shared label' }], revision: 2, hashes };
    let writes = 0;
    session.api = async () => ({ elements: async () => remote, sync: async (_mount, body) => {
      writes++; remote.spots = body.spots.upsert.map(s => ({ ...s, project_uuid: session.draft.uuid })); remote.revision = 3; return quietReply(body, 3);
    }, lookup: async () => ({ ...session.status, revision: remote.revision }) });
    session.track(); await session.runAutomatic(); session.dispose();
    if (collision) { assert.equal(writes, 0); assert.equal(session.text, 'Needs your attention'); assert.equal(session.registeredSpotConflict, true); }
    else { assert.equal(writes, 1); if (variant === 'rows') assert.equal(app.labels[0].text, 'Shared label'); else assert.equal(app.spots[0].diameter_um, 3); assert.equal(app.spots[0].name, 'Local name'); assert.equal(session.text, 'Up to date', session.error); }
  }
});
test('a server bulk guard stops automatic sync without confirming anything', async () => {
  const { SyncApiError } = await import('./sync-api.ts');
  const { session, base, hashes } = quietSession('bulk-synthetic');
  let writes = 0;
  session.api = async () => ({ elements: async () => ({ ...base, revision: 1, hashes }), sync: async (_mount, body) => {
    writes++; assert.equal(body.confirm_bulk, undefined);
    const error = new SyncApiError(422, ['Confirm this change']);
    error.bulkConfirmation = true; error.confirmation = { retireGrains: 0, grainsTotal: 1, deleteSpots: 0, spotsTotal: 0 }; throw error;
  } });
  addQuietSpot(); session.track(); await session.runAutomatic(); session.dispose();
  assert.equal(writes, 1); assert.equal(session.text, 'Needs your attention'); assert.ok(session.confirmation);
  assert.equal(session.draft.pending_request, null); assert.ok(session.draft.pending.spots.length);
});
test('keeping a bulk removal restores database grains and spots and preserves other edits', async t => {
  const { SyncApiError } = await import('./sync-api.ts');
  for (const variant of ['remove', 'split', 'merge']) {
    const { session, base, hashes } = quietSession(`keep-bulk-${variant}`);
    t.after(() => session.dispose());
    const original = { id: 'original', name: 'Database spot', area_id: 'area', grain_id: variant === 'remove' ? 'grain' : null, x_um: 1, y_um: 1, diameter_um: 1, project_uuid: session.draft.uuid };
    base.spots = [original]; session.draft = { ...session.draft, base };
    const server = { ...base, revision: 1, hashes };
    const bodies = [];
    session.api = async () => ({ elements: async () => structuredClone(server), lookup: async () => ({ ...session.status, revision: server.revision }),
      sync: async (_mount, body) => {
        bodies.push(body);
        assert.equal(body.confirm_bulk, undefined);
        if (bodies.length === 1) {
          const error = new SyncApiError(422, ['Confirm this change']); error.bulkConfirmation = true; throw error;
        }
        assert.deepEqual(body.spots.delete, []); assert.deepEqual(body.grains.retire, []); assert.deepEqual(body.grains.delete, []);
        server.spots = [...base.spots, ...body.spots.upsert.map(s => ({ ...s, project_uuid: session.draft.uuid }))];
        server.grains = [...base.grains, ...body.grains.upsert.map(g => ({ ...g, project_uuid: session.draft.uuid }))];
        server.labels = body.labels.upsert.map(l => ({ ...l, project_uuid: session.draft.uuid })); server.revision = 2;
        return quietReply(body);
      } });
    const parent = app.grains.area[0];
    app.grains = { area: variant === 'remove' ? [] : [{ ...parent, id: 'child', number: 2,
      ...(variant === 'split' ? { split_from: parent.id } : { merged_from: [parent.id] }) }] };
    addQuietSpot(); app.spots = app.spots.map(s => ({ ...s, grain_id: variant === 'remove' ? null : 'child' }));
    session.track(); await session.runAutomatic();
    assert.equal(session.text, 'Needs your attention');
    assert.ok(bodies.length, `${variant}: ${session.error}`);
    assert.deepEqual(bodies[0].spots.delete, [original.id], variant);
    app.labels = [{ id: 'later-label', area_id: 'area', x_um: 1, y_um: 1, text: 'Later edit' }];
    const started = performance.now();
    session.keepBulk();
    assert.equal(session.confirmation, null); assert.equal(session.dialog, null);
    assert.equal(app.spots.find(s => s.id === original.id).name, original.name);
    assert.deepEqual(app.grains.area.find(g => g.id === parent.id).polygon_um, parent.polygon_um);
    assert.equal(app.labels[0].text, 'Later edit');
    assert.equal(session.backgroundPending, true);
    await session.runAutomatic();
    assert.equal(bodies.length, 2); assert.equal(session.text, 'Up to date', session.error);
    t.diagnostic(`Keep bulk (${variant}): 1 guarded POST before the choice, 1 resumed POST after; ${(performance.now() - started).toFixed(1)} ms to restore and sync locally`);
    session.dispose();
  }
});

test('renewed idempotency key is durable and keeps identical wire bytes after restart', async t => {
  const { SyncApi } = await import('./sync-api.ts');
  const { session, base, hashes } = quietSession('renewal-synthetic');
  t.after(() => session.dispose());
  const writes = []; let server = { ...base, revision: 1, hashes };
  t.mock.method(console, 'warn', () => {});
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    if (String(url).endsWith('/sync')) {
      writes.push(init);
      if (writes.length === 1) return Response.json({ code: 'idempotency_key_reused' }, { status: 422 });
      const persisted = JSON.parse(storage.get('grainatlas.draft.renewal-synthetic')).pending_request;
      assert.equal(persisted.id, init.headers['Idempotency-Key']);
      assert.equal(persisted.wire_id, JSON.parse(init.body).request_id);
      if (writes.length === 2) return Response.json({ detail: 'Synthetic relay down' }, { status: 503 });
      const body = JSON.parse(init.body);
      server = { ...base, spots: body.spots.upsert.map(s => ({ ...s, project_uuid: 'renewal-synthetic' })), revision: 2, hashes };
      return Response.json(quietReply(body));
    }
    return Response.json(String(url).endsWith('/elements') ? server : { ...session.status, revision: 2 });
  });
  session.api = async () => new SyncApi('https://quiet.test', null);
  addQuietSpot(); session.track(); await session.sync(); session.dispose(); flushWrites();
  assert.equal(writes.length, 2); assert.notEqual(writes[0].headers['Idempotency-Key'], writes[1].headers['Idempotency-Key']);
  const restored = JSON.parse(storage.get('grainatlas.draft.renewal-synthetic'));
  const restarted = new SyncSession(app); app.sync = restarted; t.after(() => restarted.dispose());
  restarted.draft = restored; restarted.status = session.status; restarted.registered = true; restarted.reachable = true; restarted.api = session.api;
  await restarted.sync(); restarted.dispose();
  assert.equal(writes.length, 3);
  assert.equal(writes[1].headers['Idempotency-Key'], writes[2].headers['Idempotency-Key']);
  assert.equal(writes[0].body, writes[1].body); assert.equal(writes[1].body, writes[2].body);
  assert.equal(restarted.error, ''); assert.equal(restarted.draft.pending_request, null);
});

test('image jobs use deferred draft writes, survive further tracking and restore the attention decision', async t => {
  const { session } = quietSession('image-job-synthetic');
  t.after(() => session.dispose());
  const job = { server: 'https://quiet.test', mount: 'mount', account: 'synthetic-account', upload_id: 'synthetic-upload', atlas_sha256: 'manifest',
    start_key: 'start', commit_key: 'commit', zip_key: 'zip', uploaded: 120, expires_at: '2099-01-01T00:00:00Z', exported: '2026-01-01T00:00:00Z',
    replace_current: false, decision: 'ask', pixels_key: 'pixels' };
  saved.length = 0;
  session.savePublishJob(job);
  assert.equal(saved.length, 0);
  session.track(false); flushWrites();
  const restored = JSON.parse(storage.get('grainatlas.draft.image-job-synthetic'));
  assert.deepEqual(restored.publish_job, job);
  const restarted = new SyncSession(app); app.sync = restarted;
  t.after(() => restarted.dispose());
  restarted.draft = restored;
  assert.equal(restarted.text, 'Needs your attention');
  restarted.showImageDecision(); assert.equal(restarted.dialog, 'replace-images');
  restarted.keepImages(); flushWrites();
  assert.equal(restarted.draft.publish_job.decision, 'keep');
  assert.equal(restarted.draft.publish_job.replace_current, false);
  assert.equal(restarted.dialog, null);
  let run = 0;
  restarted.publishImages = async () => { run++; };
  restarted.draft = { ...restored, publish_job: job };
  await restarted.replaceImages();
  assert.equal(run, 1);
  assert.equal(restarted.draft.publish_job.replace_current, true);
  assert.notEqual(restarted.draft.publish_job.start_key, 'start');
  assert.notEqual(restarted.draft.publish_job.zip_key, 'zip');
});

// STORE zip built in memory, using invented bytes only; no source packages or fixtures.
function syntheticZip(files) {
  const locals = [], directory = [];
  let offset = 0;
  for (const [path, bytes] of files) {
    const name = Buffer.from(`synthetic.atlas/${path}`), local = Buffer.alloc(30), entry = Buffer.alloc(46);
    local.writeUInt32LE(0x04034b50); local.writeUInt16LE(name.length, 26);
    entry.writeUInt32LE(0x02014b50); entry.writeUInt32LE(bytes.length, 20); entry.writeUInt16LE(name.length, 28); entry.writeUInt32LE(offset, 42);
    locals.push(local, name, bytes); directory.push(entry, name); offset += 30 + name.length + bytes.length;
  }
  const end = Buffer.alloc(22); end.writeUInt32LE(0x06054b50); end.writeUInt16LE(files.length, 10); end.writeUInt32LE(offset, 16);
  return new Uint8Array(Buffer.concat([...locals, ...directory, end]));
}
test('background bundle avoids the modal, preserves export time and verifies every file hash', async () => {
  const { backgroundBundle, sha256Hex } = await import('./atlas.ts');
  const bytes = Buffer.from('Invented image bytes');
  const hash = await sha256Hex(bytes);
  let corrupt = false, options;
  const atlas = { project_id: 'synthetic', files: [{ path: 'tile.webp', size: bytes.length, sha256: hash }] };
  const fake = { fullProject: () => structuredClone(source), stitched: {}, link: { nasRoot: '', quality: 100 },
    run: () => { throw new Error('A background bundle must not open the modal'); },
    core: Promise.resolve({ atlasZip: async (_p, opts) => {
      options = opts;
      return syntheticZip([['atlas.json', Buffer.from(JSON.stringify(atlas))], ['tile.webp', corrupt ? Buffer.alloc(bytes.length) : bytes]]);
    } }),
  };
  const first = await backgroundBundle(fake, 'synthetic', '2026-01-01T00:00:00Z');
  const second = await backgroundBundle(fake, 'synthetic', options.exported);
  assert.equal(first.atlasSha256, second.atlasSha256);
  assert.equal(options.exported, '2026-01-01T00:00:00Z');
  assert.equal(first.files.length, 2);
  corrupt = true;
  await assert.rejects(backgroundBundle(fake, 'synthetic', options.exported), /checksum differs: tile.webp/);
});

test('shared publish tail preserves edits made during transfer and acknowledges only published pixels', async t => {
  const { sha256Hex } = await import('./atlas.ts');
  const { session } = quietSession('tail-synthetic');
  const oldCore = app.core;
  t.after(() => { session.dispose(); app.core = oldCore; app.pkg = null; app.stitched = {}; });
  app.project.upbdb.mount_id = 'mount';
  app.pkg = { name: 'Synthetic' };
  app.stitched = { area: { mosaic: app.project.areas[0].mosaic } };
  session.save({ pixels_base: pixelSignature(app.fullProject()) });
  app.tone.area = { ...neutralTone(), gamma: 2 };
  session.track(false);
  assert.equal(session.pixelsChanged, true);
  let published, recorded, records = 0, commits = 0, refreshes = 0;
  app.core = Promise.resolve({
    async atlasZip(p) {
      published ??= p;
      const bytes = Buffer.from(JSON.stringify(p));
      const atlas = { project_id: p.uuid, files: [{ path: 'project.json', size: bytes.length, sha256: await sha256Hex(bytes) }] };
      return syntheticZip([['atlas.json', Buffer.from(JSON.stringify(atlas))], ['project.json', bytes]]);
    },
    async recordPublished(rec) {
      recorded = rec; records++;
      // These are newer than the submitted snapshot and must survive its successful tail.
      addQuietSpot(); app.tone.area = { ...neutralTone(), gamma: 3 };
    },
    async projectJson() { return JSON.stringify({ ...source, upbdb: { mount_id: 'mount', published: [recorded] } }); },
  });
  session.api = async () => ({ publishedProject: async () => { if (++refreshes === 1) throw new Error('Synthetic refresh outage'); return published; }, lookup: async () => ({ ...session.status, atlas_version: 2 }) });
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    if (String(url).endsWith('/v1/assets/check')) return Response.json({ have: [] });
    if (String(url).endsWith('/atlas/uploads')) {
      const savedJob = JSON.parse(storage.get('grainatlas.draft.tail-synthetic')).publish_job;
      assert.equal(savedJob.start_key, init.headers['Idempotency-Key']);
      return Response.json({ upload_id: 'tail-upload', missing: [], expires_at: '2099-01-01T00:00:00Z' });
    }
    const savedJob = JSON.parse(storage.get('grainatlas.draft.tail-synthetic')).publish_job;
    assert.equal(savedJob.upload_id, 'tail-upload');
    commits++; return Response.json({ version: 2 });
  });
  await session.publishImages();
  assert.equal(session.draft.publish_job.result.version, 2);
  assert.equal(session.imageDecision, false);
  await session.publishImages();
  assert.equal(commits, 1, 'a failed acknowledgement must not publish another version');
  assert.equal(records, 1, 'recordPublished is also idempotent across acknowledgement recovery');
  assert.equal(app.spots.at(-1).id, 'quiet-spot');
  assert.equal(app.tone.area.gamma, 3);
  assert.equal(session.draft.pixels_base, pixelSignature(published));
  assert.equal(session.pixelsChanged, true);
  assert.equal(session.draft.publish_job, null);
  assert.equal(app.lastPublish.version, 2);
});

test('Edit restores an image-only draft with null pending elements so a held upload can resume', async t => {
  const { editCopy } = await import('./remote.ts');
  const { writeDraft } = await import('./drafts.ts');
  const { session } = quietSession('image-reopen-synthetic');
  const oldCore = app.core;
  t.after(() => { session.dispose(); app.core = oldCore; app.remote = null; });
  const local = app.fullProject();
  local.grainatlas_ui.bundle_tone = { area: { curve: { ...neutralTone(), low_pct: 12 }, display: neutralTone(), on_sha256: 'pixels' } };
  const key = 'grainatlas.project.Synthetic';
  storage.set(key, JSON.stringify(local));
  const draft = { ...session.draft, source_keys: [key], pending: null, pixels_base: pixelSignature(app.project), pixels_key: pixelSignature(local), pixels_changed: true };
  writeDraft(draft);
  app.remote = { server: 'https://quiet.test', projectId: local.uuid, readOnly: true, atlas: { name: 'Synthetic' } };
  app.bundle = null;
  app.core = Promise.resolve({ blank: async () => {}, loadProject: async () => {} });
  t.mock.method(app, 'run', async (_title, work) => work());
  t.mock.method(session, 'editBase', async () => {});
  await editCopy(app);
  assert.equal(app.project.grainatlas_ui.bundle_tone.area.curve.low_pct, 12);
  assert.equal(app.readOnly, false);
  assert.equal(session.draft.pending, null);
});

test('background images wait through stale, offline and remembered failures until a successful check', async t => {
  const { forgetUnreachable } = await import('./sync-session.svelte.ts');
  const { SyncNetworkError } = await import('./sync-api.ts');
  forgetUnreachable();
  const { session, base, hashes } = quietSession('image-offline-synthetic');
  session.draft = { ...session.draft, server: 'https://image-offline-only.test' };
  const oldCore = app.core;
  t.after(() => { session.dispose(); forgetUnreachable(); app.core = oldCore; app.pkg = null; app.stitched = {}; });
  session.uuid = app.project.uuid;
  app.project.upbdb.mount_id = 'mount';
  app.pkg = { name: 'Synthetic' };
  app.stitched = { area: { mosaic: app.project.areas[0].mosaic } };
  session.save({ pixels_base: pixelSignature(app.fullProject()) });
  app.tone.area = { ...neutralTone(), gamma: 2 };
  session.track(false); session.lastImageEdit = -Infinity;
  let builds = 0;
  app.core = Promise.resolve({ atlasZip: async () => { builds++; return null; } });
  const fetch = t.mock.method(globalThis, 'fetch', () => { throw new Error('No network call while paused'); });
  assert.equal(session.imagePending, true);
  session.staleOpen = { spots: ['shared-spot'], grains: [], labels: [] };
  await session.publishImages(undefined, true);
  assert.equal(session.imagePending, false); assert.equal(builds, 0);
  session.staleOpen = null;
  for (const reachable of [false, null]) {
    session.reachable = reachable;
    await session.publishImages(undefined, true);
    assert.equal(session.imagePending, false); assert.equal(builds, 0);
  }
  session.api = async () => { throw new SyncNetworkError('Synthetic outage'); };
  await session.check();
  // Cached reachability in another copy cannot override the remembered failure.
  session.reachable = true;
  assert.equal(session.knownUnreachable, true);
  await session.publishImages(undefined, true);
  assert.equal(builds, 0); assert.equal(fetch.mock.callCount(), 0);
  const status = session.status;
  session.api = async () => ({ lookup: async () => status, elements: async () => ({ ...base, revision: 1, hashes }) });
  await session.check();
  assert.equal(session.knownUnreachable, false); assert.equal(session.imagePending, true);
  await session.publishImages(undefined, true);
  assert.equal(builds, 1);
  // A guard raised while the worker builds must also stop the capability request.
  for (const guard of ['stale', 'offline']) {
    session.staleOpen = null; session.reachable = true;
    app.core = Promise.resolve({ atlasZip: async () => {
      if (guard === 'stale') session.staleOpen = { spots: ['shared-spot'], grains: [], labels: [] };
      else session.reachable = false;
      return syntheticZip([['atlas.json', Buffer.from(JSON.stringify({ project_id: app.project.uuid, files: [] }))]]);
    } });
    await session.publishImages(undefined, true);
    assert.equal(session.imageBusy, false);
    assert.equal(fetch.mock.callCount(), 0, guard);
  }
  session.staleOpen = null; session.reachable = true;
  app.core = Promise.resolve({ atlasZip: async () => syntheticZip([
    ['atlas.json', Buffer.from(JSON.stringify({ project_id: app.project.uuid, files: [] }))],
  ]) });
  await session.publishImages(undefined, true);
  assert.equal(fetch.mock.callCount(), 1);
  assert.equal(session.knownUnreachable, true);
  assert.equal(session.text, 'Offline, will sync later');
  assert.equal(session.imageTimer.scheduled, false);
  await session.publishImages(undefined, true);
  assert.equal(fetch.mock.callCount(), 1, 'a failed capability waits for a successful check before another upload');
});
