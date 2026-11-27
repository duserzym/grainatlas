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
  p.grainatlas_ui = { grains: grainMapWithoutBlankArtifacts(JSON.parse(JSON.stringify(app.grains))), spots: JSON.parse(JSON.stringify(app.spots)), labels: JSON.parse(JSON.stringify(app.labels)), tone: JSON.parse(JSON.stringify(app.tone)), overlays: Object.fromEntries(Object.entries(app.overlays).map(([k, o]) => [k, { ...JSON.parse(JSON.stringify(o)), url: null }])), print_label_size_mm: 'auto' };
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

test('automatic mode keeps Open list, unloaded session and tracked draft status fixtures consistent', () => {
  const previousAutomatic = app.link.syncAutomatically;
  app.link.syncAutomatically = true;
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
  app.link.syncAutomatically = previousAutomatic;
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
test('manual sync retains a failed Save and sync request until an explicit retry', async t => {
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
  assert.equal(writes, 1, 'opening or checking never uploads a failed request in manual mode');
  await session.sync();
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
  const local = structuredClone(source);
  local.grainatlas_ui.print_label_pins = { 'spot-1/band/0': [2, 3] };
  local.grainatlas_ui.print_label_size_mm = 3;
  const fake = { fullProject: () => structuredClone(local), stitched: {}, link: { nasRoot: '', quality: 100 },
    run: () => { throw new Error('A background bundle must not open the modal'); },
    core: Promise.resolve({ atlasZip: async (_p, opts) => {
      assert.equal(_p.grainatlas_ui.print_label_pins, undefined);
      assert.equal(_p.grainatlas_ui.print_label_size_mm, undefined);
      options = opts;
      const projectBytes = Buffer.from(JSON.stringify(_p));
      const atlas = { project_id: 'synthetic', files: [
        { path: 'project.json', size: projectBytes.length, sha256: await sha256Hex(projectBytes) },
        { path: 'tile.webp', size: bytes.length, sha256: hash },
      ] };
      return syntheticZip([['atlas.json', Buffer.from(JSON.stringify(atlas))], ['project.json', projectBytes], ['tile.webp', corrupt ? Buffer.alloc(bytes.length) : bytes]]);
    } }),
  };
  const first = await backgroundBundle(fake, 'synthetic', '2026-01-01T00:00:00Z');
  const second = await backgroundBundle(fake, 'synthetic', options.exported);
  assert.equal(first.atlasSha256, second.atlasSha256);
  assert.equal(options.exported, '2026-01-01T00:00:00Z');
  assert.equal(first.files.length, 3);
  const published = JSON.parse(new TextDecoder().decode(first.files.find(f => f.path === 'project.json').bytes));
  assert.equal(Object.hasOwn(published.grainatlas_ui, 'print_label_pins'), false);
  assert.equal(Object.hasOwn(published.grainatlas_ui, 'print_label_size_mm'), false);
  assert.deepEqual(published.grainatlas_ui.spots, local.grainatlas_ui.spots);
  assert.deepEqual(first.project.grainatlas_ui.print_label_pins, local.grainatlas_ui.print_label_pins);
  assert.equal(first.project.grainatlas_ui.print_label_size_mm, 3);
  assert.deepEqual(JSON.parse(JSON.stringify(local)).grainatlas_ui.print_label_pins, { 'spot-1/band/0': [2, 3] });
  assert.equal(JSON.parse(JSON.stringify(local)).grainatlas_ui.print_label_size_mm, 3);
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

for (const [name, version, statusVersion, projectId, signature, expectedCalls] of [
  ['matching versions', 7, 7, 'baseline-synthetic', true, 0],
  ['different versions', 7, 8, 'baseline-synthetic', true, 1],
  ['unknown opened version', undefined, 7, 'baseline-synthetic', true, 1],
  ['unknown published version', 7, null, 'baseline-synthetic', true, 1],
  ['another project', 7, 7, 'another-synthetic', true, 1],
  ['missing opened signature', 7, 7, 'baseline-synthetic', false, 1],
]) {
  test(`pixel baseline reuse: ${name}`, async t => {
    const { session, base, hashes } = quietSession('baseline-synthetic');
    t.after(() => { session.dispose(); app.remote = null; });
    app.remote = { server: 'https://quiet.test', projectId, version, readOnly: true,
      ...(signature ? { pixelsSignature: pixelSignature(source) } : {}) };
    const published = structuredClone(source);
    published.areas[0].mosaic.sha256 = 'newly-published-pixels';
    const status = { ...session.status, atlas_version: statusVersion };
    const download = t.mock.fn(async () => published);
    session.api = async () => ({ lookup: async () => status,
      elements: async () => ({ ...base, revision: 1, hashes }), publishedProject: download });
    app.project.areas[0].mosaic.sha256 = 'local-edits';
    await session.check();
    assert.equal(session.error, '');
    assert.equal(download.mock.callCount(), expectedCalls);
    assert.equal(session.draft.pixels_base, pixelSignature(expectedCalls ? published : source));
  });
}

async function serverOpenSetup(t) {
  const { openFromServer } = await import('./remote.ts');
  const { session } = quietSession('open-synthetic');
  app.stopNumbering(); // Keep unrelated TIMS effects off this network harness.
  app.project = null; app.link.token = 'synthetic-token';
  t.after(() => { session.dispose(); app.remote = null; app.link.token = ''; });
  const hashes = { spots: 's', grains: 'g', labels: 'l' };
  const published = { ...structuredClone(source), uuid: 'open-synthetic' };
  published.grainatlas_ui.tone = { area: { ...neutralTone(), gamma: 2 } };
  published.grainatlas_ui.bundle_tone = { area: { curve: neutralTone(), display: neutralTone(), on_sha256: 'pixels' } };
  published.image_edits = { areas: { area: { synthetic: true } } };
  const download = t.mock.fn(async () => published);
  session.api = async () => ({ lookup: async () => ({ project_uuid: published.uuid, mount_id: 'mount', revision: 1, atlas_version: 7, can_edit: true, hashes }),
    elements: async () => ({ spots: [], grains: [], labels: [], revision: 1, hashes }), publishedProject: download });
  const atlas = { format: 'grainatlas-atlas', version: 1, name: 'Synthetic', tile_size: 256,
    files: Array.from({ length: 5099 }, (_, n) => ({ path: `tiles/${n}.webp`, size: 1, sha256: `synthetic-${n}` })),
    areas: [{ ...published.areas[0], path: 'area', levels: [], w: 32, h: 24, um_per_px: 1 }] };
  const reply = (data, version = 7) => Response.json(data, { headers: version === null ? {} : { 'X-Atlas-Version': String(version) } });
  return { atlas, published, download, reply, open: () => openFromServer('https://quiet.test', published.uuid, null, 'synthetic-access', null, app) };
}

for (const [name, atlasVersion, projectVersion, count] of [
  ['matching versions', 7, 7, 2], ['a publish race', 7, 8, 3],
  ['a missing project version', 7, null, 3], ['a missing atlas version', null, 7, 3],
  ['missing versions', null, null, 3], ['an invalid project version', 7, 1.5, 3],
]) {
  test(`parallel server open handles ${name}`, async t => {
    const { atlas, published, download, reply, open } = await serverOpenSetup(t);
    const { oversightSession } = await import('./oversight.ts');
    oversightSession.set({ viewAs: { username: 'synthetic-viewer', name: 'Synthetic' }, onBehalf: false });
    t.after(() => oversightSession.clear());
    const requests = [];
    let releaseAtlas;
    const atlasGate = new Promise(resolve => { releaseAtlas = resolve; });
    t.mock.method(globalThis, 'fetch', async (url, init) => {
      const u = new URL(url); requests.push(u);
      assert.equal(u.searchParams.get('access'), 'synthetic-access');
      assert.equal(u.searchParams.get('as'), 'synthetic-viewer');
      assert.equal(new Headers(init.headers).get('Authorization'), 'Bearer synthetic-token');
      if (u.pathname.endsWith('/atlas.json')) { await atlasGate; return reply(atlas, atlasVersion); }
      if (requests.length === 2) {
        assert.equal(u.searchParams.has('version'), false);
        assert.equal(init.cache, 'no-cache');
        releaseAtlas(); // Atlas cannot finish unless project starts concurrently.
        const speculative = structuredClone(published);
        if (count === 3) speculative.areas[0].mosaic.sha256 = 'wrong-bundle';
        return reply(speculative, projectVersion);
      }
      assert.equal(u.searchParams.get('version'), atlasVersion === null ? null : String(atlasVersion));
      assert.equal(init.cache, atlasVersion === null ? 'no-cache' : 'default');
      return reply(published, atlasVersion);
    });
    assert.equal(await open(), true);
    assert.equal(app.error, null);
    assert.equal(requests.length, count);
    assert.equal(app.project.areas[0].mosaic.sha256, 'pixels');
    assert.equal(app.remote.pixelsSignature, pixelSignature(published));
    assert.equal(download.mock.callCount(), atlasVersion === null ? 1 : 0);
    assert.equal(app.sync.draft.pixels_base, pixelSignature(published));
  });
}

test('parallel server open preserves atlas validation and HTTP/JSON errors', async t => {
  for (const failure of ['format', 'version', 'atlas-http', 'project-http', 'json', 'network', 'fallback-http']) {
    const { atlas, published, reply, open } = await serverOpenSetup(t);
    let calls = 0;
    const fetch = t.mock.method(globalThis, 'fetch', async url => {
      calls++;
      if (String(url).includes('/atlas.json')) {
        if (failure === 'atlas-http') return new Response(null, { status: 404 });
        return reply({ ...atlas, ...(failure === 'format' ? { format: 'other' } : failure === 'version' ? { version: 2 } : {}) });
      }
      if (failure === 'project-http') return new Response(null, { status: 401 });
      if (failure === 'network') throw new TypeError('Synthetic network failure');
      if (failure === 'json') return new Response('{', { headers: { 'X-Atlas-Version': '7' } });
      if (failure === 'fallback-http' && calls === 3) return new Response(null, { status: 403 });
      return reply(published, failure === 'fallback-http' ? 8 : 7);
    });
    assert.equal(await open(), false, failure);
    assert.match(app.error, { format: /not a grainatlas atlas/, version: /newer than/, 'atlas-http': /atlas.json: 404.*not published/,
      'project-http': /project.json: 401.*sign in/, json: /JSON|property name/, network: /database did not answer/,
      'fallback-http': /project.json: 403/ }[failure]);
    assert.equal(app.project, null); assert.equal(app.cancelTask, null);
    fetch.mock.restore();
  }
});

test('a malformed speculative document from another version is discarded', async t => {
  const { atlas, published, reply, open } = await serverOpenSetup(t);
  const requests = [];
  t.mock.method(globalThis, 'fetch', async url => {
    const u = new URL(url); requests.push(u);
    if (u.pathname.endsWith('atlas.json')) return reply(atlas);
    return u.searchParams.has('version') ? reply(published)
      : new Response('{', { headers: { 'X-Atlas-Version': '8' } });
  });
  assert.equal(await open(), true);
  assert.equal(requests.length, 3);
  assert.equal(app.remote.pixelsSignature, pixelSignature(published));
});

test('imagesPublished still downloads the fresh baseline despite an opened signature', async t => {
  const { session } = quietSession('published-baseline-synthetic');
  t.after(() => { session.dispose(); app.remote = null; });
  app.remote = { server: 'https://quiet.test', projectId: app.project.uuid, version: 1,
    readOnly: true, pixelsSignature: pixelSignature(source) };
  const published = structuredClone(source);
  published.areas[0].mosaic.sha256 = 'freshly-published';
  const download = t.mock.fn(async () => published);
  session.api = async () => ({ publishedProject: download, lookup: async () => session.status });
  await session.imagesPublished(app.project.uuid);
  assert.equal(download.mock.callCount(), 1);
  assert.equal(session.draft.pixels_base, pixelSignature(published));
});

for (const fallback of [false, true]) {
  test(`cancelling parallel server open stops ${fallback ? 'the versioned fallback' : 'both initial requests'}`, async t => {
    const { atlas, published, reply, open } = await serverOpenSetup(t);
    const signals = [];
    let started;
    const waiting = new Promise(resolve => { started = resolve; });
    t.mock.method(globalThis, 'fetch', async (url, { signal }) => {
      signals.push(signal);
      if (fallback && signals.length < 3) return reply(String(url).includes('atlas.json') ? atlas : published, signals.length === 1 ? 7 : 8);
      if (signals.length === (fallback ? 3 : 2)) started();
      return new Promise((_, reject) => signal.addEventListener('abort', () => reject(signal.reason), { once: true }));
    });
    const opening = open();
    await waiting; app.cancelTask();
    assert.equal(await opening, false);
    assert.equal(signals.at(-1).aborted, true);
    if (!fallback) assert.ok(signals.every(s => s.aborted));
    assert.equal(app.error, null); assert.equal(app.project, null); assert.equal(app.cancelTask, null);
  });
}
test('real AppState saves label pins locally and reopening restores them without sync work', async () => {
  const { labelUI, pinLabel, readLabelOptions } = await import('./print-labels.ts');
  const { elementsOf } = await import('./sync.ts');
  const { elementHash } = await import('./sync-hash.ts');
  app.project = structuredClone(source); app.adopt(app.project); app.viewMode = false; app.remote = null; app.loadingProject = false; app.storedCopy = null;
  const bytes = canonicalJson(elementsOf(app.fullProject()));
  const hashes = await Promise.all(Object.values(elementsOf(app.fullProject())).map(elementHash));
  const label = { spot_id: 'spot-1', occurrence: 'band/0', anchor_mm: [20, 30] };
  const track = app.sync.track, image = app.sync.imagesEdited;
  app.sync.track = () => { throw Error('UI pins must not schedule sync'); };
  app.sync.imagesEdited = () => { throw Error('UI pins must not change image hashes'); };
  try {
    for (const position of [[42, 53], [46, 57], null]) {
      app.printLabels = pinLabel(app.printLabels, label, position);
      app.persistPrintLabels(); flushWrites();
      const saved = JSON.parse(JSON.stringify(app.fullProject()));
      assert.equal(canonicalJson(elementsOf(saved)), bytes);
      assert.deepEqual(await Promise.all(Object.values(elementsOf(saved)).map(elementHash)), hashes);
      assert.deepEqual(readLabelOptions(saved.grainatlas_ui), readLabelOptions(labelUI(app.printLabels)));
      app.adopt(saved);
      assert.deepEqual(app.printLabels, readLabelOptions(saved.grainatlas_ui));
    }
    app.viewMode = true;
    const writes = saved.length;
    app.printLabels = pinLabel(app.printLabels, label, [70, 80]); app.persistPrintLabels(); flushWrites();
    assert.equal(saved.length, writes);
    assert.deepEqual(app.printLabels.pins['spot-1/band/0'], [50, 50]);
  } finally { app.sync.track = track; app.sync.imagesEdited = image; app.viewMode = false; app.adopt(structuredClone(source)); }
});

test('fixed and Auto label sizes round-trip with pins in project files and clean database copies', async t => {
  const { editCopy } = await import('./remote.ts');
  const { loadProjectFile } = await import('./actions.ts');
  const { writeDraft } = await import('./drafts.ts');
  const { savedProject } = await import('./deeplink.ts');
  const { readLabelOptions } = await import('./print-labels.ts');
  const oldCore = app.core, oldPkg = app.pkg;
  t.after(() => {
    flushWrites(); app.core = oldCore; app.pkg = oldPkg; app.remote = null; app.projectPath = null;
    app.project = structuredClone(source); app.adopt(app.project);
  });
  let loaded;
  app.pkg = null; // This bundle fixture has no raw package to re-render on load.
  app.core = Promise.resolve({ blank: async () => {}, loadProject: async json => { loaded = JSON.parse(json); }, projectJson: async () => JSON.stringify(loaded) });
  t.mock.method(app, 'run', async (_title, work) => work());
  t.mock.method(app.sync, 'check', async () => {});
  t.mock.method(app.sync, 'editBase', async () => {});
  t.mock.method(app.sync, 'track', () => {});
  t.mock.method(app.sync, 'imagesEdited', () => {});
  for (const kind of ['project file', 'database copy']) {
    for (const size_mm of [3, 'auto']) {
      const p = structuredClone(source);
      p.uuid = `label-roundtrip-${kind === 'project file' ? 'file' : 'database'}-${size_mm}`;
      p.package.name = p.uuid;
      app.project = p; app.adopt(p); app.remote = null; app.bundle = null;
      app.viewMode = false; app.loadingProject = false; app.storedCopy = null; app.projectPath = null;
      // Start Auto from a saved fixed size, so a missing/reset field cannot pass.
      app.printLabels = { size_mm: size_mm === 'auto' ? 3 : 'auto', pins: {} };
      app.persistPrintLabels(); flushWrites();
      const preferences = { size_mm, pins: { 'spot-1/band/0': [22, 23] } };
      app.printLabels = structuredClone(preferences);
      app.persistPrintLabels(); flushWrites();
      const local = savedProject(p.package.name);
      assert.deepEqual(readLabelOptions(local.grainatlas_ui), preferences);
      assert.equal(local.grainatlas_ui.print_label_size_mm, size_mm);
      const file = JSON.parse(JSON.stringify(app.fullProject()));
      assert.deepEqual(readLabelOptions(file.grainatlas_ui), preferences);
      app.project = structuredClone(p); app.adopt(app.project);
      if (kind === 'project file') {
        // Exercise the actual local file load path, including stripping UI for
        // the core and adopting it again in the store. No server bundle involved.
        await loadProjectFile(new File([JSON.stringify(file)], 'Synthetic.gaproj', { type: 'application/json' }), app, true);
        assert.equal(loaded.grainatlas_ui, undefined);
      } else {
        // A clean element outbox must still recover local presentation, without
        // adopting the old registered rows from the full saved project.
        local.grainatlas_ui.spots = [{ id: 'stale-local-spot' }];
        storage.set(`grainatlas.project.${p.package.name}`, JSON.stringify(local));
        writeDraft({ uuid: p.uuid, name: p.name, server: 'https://labels.test', mount: 'mount',
          base_revision: 1, base: { spots: [], grains: [], labels: [] }, pending: null,
          changes: { spots: 0, grains: 0, labels: 0 }, pixels_changed: false,
          updated_at: '2026-01-01T00:00:00Z', source_keys: [`grainatlas.project.${p.package.name}`] });
        app.remote = { server: 'https://labels.test', projectId: p.uuid, readOnly: true, atlas: { name: 'Synthetic' } };
        await editCopy(app); flushWrites();
        assert.deepEqual(app.spots, p.grainatlas_ui.spots);
      }
      assert.deepEqual(app.printLabels, preferences, `${kind}, ${size_mm}`);
      assert.deepEqual(readLabelOptions(app.fullProject().grainatlas_ui), preferences);
    }
  }
});

test('spot viewing, selection, area switch and save preserve raw sizes, element hashes and empty diff', async () => {
  const { elementsOf, diffElements, changeCounts } = await import('./sync.ts');
  const { elementHash } = await import('./sync-hash.ts');
  const { spotDiameter, selectionSize, nextSpotDiameter, areaCalibration } = await import('./spot-size.ts');
  const p = structuredClone(source);
  p.uuid = 'synthetic-size-mount';
  p.areas.push({...structuredClone(p.areas[0]),id:'other',name:'Synthetic other'});
  p.grainatlas_ui.spots = [20,40,null,0,'20'].map((diameter_um,i)=>({id:`synthetic-size-${i}`,name:`Synthetic ${i}`,area_id:i===1?'other':'area',grain_id:'grain',x_um:2,y_um:2,diameter_um,created:`2026-01-0${i+1}`,acquisition_id:'synthetic-acquisition'}));
  const original = JSON.stringify(p.grainatlas_ui.spots);
  app.project = structuredClone(p); app.spots = []; app.areaId='area'; app.adopt(structuredClone(p));
  const before = elementsOf(app.fullProject()), hashes = await Promise.all(Object.values(before).map(elementHash));
  for (const area of ['area','other','area']) {
    app.areaId=area;
    app.selection=app.spots.filter(s=>s.area_id===area).map(s=>s.id);
    const here=app.spots.filter(s=>s.area_id===area);
    for(const s of here) spotDiameter(s);
    selectionSize(here,app.diameter); nextSpotDiameter(app.spots,area,25); areaCalibration(p.areas.find(a=>a.id===area),null);
    const saved=app.fullProject();
    assert.equal(JSON.stringify(saved.grainatlas_ui.spots),original);
    const after=elementsOf(saved);
    assert.deepEqual(await Promise.all(Object.values(after).map(elementHash)),hashes);
    assert.deepEqual(changeCounts(diffElements(before,after)),{grains:0,spots:0,labels:0});
  }
});

test('applying a size repair preview is one undo step and preserves registered names and links', async () => {
  const { planSizeRepairs, applySizeRepairs } = await import('./spot-size.ts');
  const p=structuredClone(source);
  p.grainatlas_ui.spots=[{id:'valid-size',name:'Synthetic valid',area_id:'area',grain_id:'grain',x_um:2,y_um:2,diameter_um:20,acquisition_id:'synthetic-link'}, {id:'no-size',name:'Synthetic missing',area_id:'area',grain_id:'grain',x_um:4,y_um:4,diameter_um:null,acquisition_id:'synthetic-link'}];
  app.project=p; app.adopt(p); app.clearHistory();
  const plan=planSizeRepairs(app.spots,25), original=JSON.stringify(app.spots);
  assert.equal(app.canUndo,false); assert.equal(JSON.stringify(app.spots),original);
  app.checkpoint(); app.spots=applySizeRepairs(app.spots,plan); app.persist();
  assert.equal(app.spots[1].diameter_um,20); assert.equal(app.spots[1].name,'Synthetic missing'); assert.equal(app.spots[1].acquisition_id,'synthetic-link');
  app.undo(); assert.equal(JSON.stringify(app.spots),original); assert.equal(app.canUndo,false);
});

test('next-spot size resets on project/area changes, preserves user entry between them and never edits spots', async () => {
  const { flush } = await import('svelte/internal/client');
  const p=structuredClone(source);
  p.uuid='synthetic-next-size'; p.areas.push({...structuredClone(p.areas[0]),id:'b',name:'Synthetic B'});
  p.grainatlas_ui.spots=[{id:'size-a',area_id:'area',diameter_um:20,created:'2026-01-01'}, {id:'size-b',area_id:'b',diameter_um:40,created:'2026-01-02'}];
  app.project=p; app.areaId='area'; app.adopt(p); flush();
  const original=JSON.stringify(app.spots);
  assert.equal(app.diameter,20);
  app.diameter=25; flush(); app.selection=['size-a']; flush();
  assert.equal(app.diameter,25);
  app.areaId='b'; flush(); assert.equal(app.diameter,40);
  app.areaId='area'; flush(); assert.equal(app.diameter,20);
  const empty={...structuredClone(p),uuid:'synthetic-empty',grainatlas_ui:{...p.grainatlas_ui,spots:[]}};
  app.project=empty; app.settings.default_diameter_um=35; app.adopt(empty); flush();
  assert.equal(app.diameter,35);
  assert.equal(JSON.stringify(p.grainatlas_ui.spots),original);
});

test('start-screen catch-up pulls an unseen server spot and preserves existing registered identity and links', async t => {
  const { session, base, hashes } = quietSession('catch-up-synthetic');
  t.after(() => { session.dispose(); app.link.token = ''; });
  const { oversightSession } = await import('./oversight.ts');
  const previous = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(previous));
  app.link.token = 'synthetic-token';
  addQuietSpot();
  Object.assign(app.spots[0], { acquisition_id: 'synthetic-measurement', n: 1, created: '', created_by: '', sample: 'Synthetic', grain_number: 1 });
  base.spots = JSON.parse(JSON.stringify(app.spots));
  session.draft = { ...session.draft, base, base_revision: 1 };
  const original = JSON.parse(JSON.stringify(app.spots[0]));
  const added = { ...original, id: 'new-server-spot', name: 'Synthetic incoming', acquisition_id: 'synthetic-new-link', project_uuid: app.project.uuid };
  const theirs = { ...structuredClone(base), spots: [...base.spots, added], revision: 2, hashes: { ...hashes, spots: 's2' } };
  session.serverElements = theirs; session.elementsRevision = 2;
  session.status = { ...session.status, revision: 2, hashes: theirs.hashes };
  let reads = 0, writes = 0;
  session.api = async () => ({ elements: async () => { reads++; return theirs; }, lookup: async () => session.status, sync: async () => { writes++; throw new Error('Catch-up must not upload'); } });
  await session.catchUp();
  assert.equal(reads, 1); assert.equal(writes, 0);
  assert.deepEqual(JSON.parse(JSON.stringify(app.spots[0])), original);
  assert.equal(app.spots[1].id, added.id); assert.equal(app.spots[1].acquisition_id, added.acquisition_id);
  assert.equal(session.draft.base_revision, 2);
  assert.equal(app.notice, 'Updated from the database: 1 spot');
});

test('start-screen catch-up waits for the opening check and ignores a changed project identity', async t => {
  const { session } = quietSession('catch-up-wait-synthetic');
  t.after(() => { session.dispose(); app.link.token = ''; });
  const { oversightSession } = await import('./oversight.ts');
  const previous = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(previous));
  app.link.token = 'synthetic-token'; session.busy = true;
  let updates = 0;
  t.mock.method(session, 'update', async () => { updates++; });
  let finished = false;
  const pending = session.catchUp().then(() => { finished = true; });
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal(finished, false, 'catch-up is still waiting for the opening check');
  assert.equal(updates, 0);
  app.project = { ...app.project, uuid: 'synthetic-other-project' };
  session.finished(); await pending;
  assert.equal(updates, 0);
});

test('start-screen catch-up reviews local edits without silently merging and leaves hosted opens alone', async t => {
  const { session, base } = quietSession('catch-up-review-synthetic');
  t.after(() => { session.dispose(); app.link.token = ''; app.hosted = false; });
  const { oversightSession } = await import('./oversight.ts');
  const previous = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(previous));
  app.link.token = 'synthetic-token'; app.link.syncAutomatically = false;
  addQuietSpot(); session.draft = { ...session.draft, base };
  session.status = { ...session.status, revision: 2 };
  let actions = 0, updates = 0;
  t.mock.method(session, 'update', async () => { updates++; });
  t.mock.method(session, 'action', async () => { actions++; });
  const before = JSON.stringify(app.spots);
  await session.catchUp();
  assert.equal(updates, 0); assert.equal(JSON.stringify(app.spots), before);
  assert.equal(app.notice, 'The database has newer changes than this local copy.');
  assert.equal(app.noticeAction.label, 'Review'); app.noticeAction.run(); assert.equal(actions, 1);
  app.notice = 'Synthetic existing notice'; app.hosted = true;
  await session.catchUp(); assert.equal(app.notice, 'Synthetic existing notice'); assert.equal(updates, 0);
});

test('offline bundle opens read-only with last database rows and no image or bundle network reads', async t => {
  const { mountMirror } = await import('./mount-mirror.ts');
  const { browserAccount, hideMountCache } = await import('./mount-browser.ts');
  const { cacheBundle } = await import('./offline-bundles.ts');
  const { openOfflineCopy, editCopy } = await import('./remote.ts');
  const { oversightSession } = await import('./oversight.ts');
  const oldScope = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(oldScope));
  const { session } = quietSession('offline-bundle-synthetic');
  app.stopNumbering();
  t.after(() => { session.dispose(); app.remote = null; app.link.token = ''; });
  app.link.token = 'synthetic-offline-token';
  const map = new Map();
  for (const [method, fn] of Object.entries({ getItem: key => map.get(key) ?? null, setItem: (key, value) => map.set(key, value), removeItem: key => map.delete(key) })) t.mock.method(mountMirror, method, fn);
  hideMountCache();
  t.mock.method(globalThis, 'fetch', async url => Response.json(String(url).includes('/accounts/me') ? { id: 'synthetic-account' } : String(url).includes('/v1/server') ? { dataset_id: 'synthetic-dataset' } : []));
  const account = await browserAccount('https://offline.test', app.link.token, mountMirror);
  const project = { ...structuredClone(source), uuid: 'offline-bundle-synthetic' };
  const spot = { id: 'offline-spot', name: 'Synthetic cached spot', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 20, acquisition_id: 'synthetic-cached-link' };
  const elements = { ...structuredClone(session.elements), spots: [spot] };
  const atlas = { format: 'grainatlas-atlas', version: 1, project_id: project.uuid, name: 'Synthetic cached mount', tile_size: 256, files: [],
    areas: [{ ...project.areas[0], path: 'area', levels: [], w: 32, h: 24, um_per_px: 1 }] };
  await cacheBundle(mountMirror, account.mirrorId, account.id, { atlas, project, elements, revision: 3, at: '2026-11-23T12:00:00Z' });
  const network = t.mock.method(globalThis, 'fetch', async () => { throw new Error('No offline bundle or tile download'); });
  app.project = null;
  let checks = 0;
  session.api = async () => { checks++; throw new Error('Synthetic offline check'); };
  assert.equal(await openOfflineCopy('https://offline.test', project.uuid, 'mount', 'Synthetic cached mount', app), true);
  assert.equal(app.readOnly, true); assert.equal(checks, 1);
  assert.equal(app.spots[0].id, spot.id); assert.equal(app.spots[0].acquisition_id, spot.acquisition_id);
  assert.match(app.notice, /^Offline copy of Synthetic cached mount · as of .* · read-only$/);
  await editCopy(app); assert.equal(app.readOnly, true);
  assert.equal(app.notice, 'Offline copy: read-only until the database is reachable');
  const { publishedFile } = await import('./published-files.ts');
  await assert.rejects(publishedFile(app.remote, 'missing.webp'), /not cached/);
  assert.equal(network.mock.callCount(), 0);
  assert.equal([...map.values()].some(value => value.includes('synthetic-offline-token')), false);
  session.reachable = true;
  const online = t.mock.method(globalThis, 'fetch', async () => new Response(new Uint8Array([1, 2])));
  assert.deepEqual([...new Uint8Array(await publishedFile(app.remote, 'after-online.webp'))], [1, 2]);
  assert.equal(online.mock.callCount(), 1, 'a successful database check permits uncached image reads again');
});

test('server open caches raw bundle and last server rows, never a pending local overlay, without extra identity requests', async t => {
  const { mountMirror } = await import('./mount-mirror.ts');
  const { browserAccount, hideMountCache } = await import('./mount-browser.ts');
  const { readBundle, bundleKey } = await import('./offline-bundles.ts');
  const { openFromServer } = await import('./remote.ts');
  const { oversightSession } = await import('./oversight.ts');
  const oldScope = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(oldScope));
  const { atlas, published } = await serverOpenSetup(t);
  atlas.project_id = published.uuid;
  const map = new Map();
  let cached;
  const written = new Promise(resolve => { cached = resolve; });
  t.mock.method(mountMirror, 'getItem', key => map.get(key) ?? null);
  t.mock.method(mountMirror, 'setItem', (key, value) => { map.set(key, value); if (key === bundleKey('synthetic-dataset', 'synthetic-account', published.uuid)) cached(); });
  t.mock.method(mountMirror, 'removeItem', key => map.delete(key));
  hideMountCache();
  const paths = [];
  t.mock.method(globalThis, 'fetch', async url => {
    const path = new URL(url).pathname; paths.push(path);
    const body = path.endsWith('/accounts/me') ? { id: 'synthetic-account' } : path.endsWith('/v1/server') ? { dataset_id: 'synthetic-dataset' } : path.endsWith('/v1/labs') ? [] : path.endsWith('/atlas.json') ? atlas : published;
    return Response.json(body, { headers: { 'X-Atlas-Version': '7' } });
  });
  const account = await browserAccount('https://quiet.test', 'synthetic-token', mountMirror);
  paths.length = 0;
  const localSpot = { id: 'synthetic-pending-spot', name: 'Synthetic pending spot', area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 20 };
  const { writeDraft } = await import('./drafts.ts');
  writeDraft({ ...app.sync.draft, pending: { spots: [localSpot], grains: [], labels: [] }, changes: { spots: 1, grains: 0, labels: 0 } });
  // The access-link path keeps the prior uncached request flow as a baseline.
  const baselineStart = performance.now();
  assert.equal(await openFromServer('https://quiet.test', published.uuid, null, 'synthetic-access', 'mount', app), true);
  const baseline = { bundleRequests: paths.length, identityRequests: 0, ms: performance.now() - baselineStart };
  assert.equal(baseline.bundleRequests, 2); paths.length = 0;
  const started = performance.now();
  assert.equal(await openFromServer('https://quiet.test', published.uuid, null, null, 'mount', app), true);
  await written;
  const record = await readBundle(mountMirror, account.mirrorId, account.id, published.uuid);
  assert.equal(record.project.uuid, published.uuid);
  assert.equal(app.spots[0].id, localSpot.id, 'the live view still shows the kept pending overlay');
  assert.deepEqual(record.project.grainatlas_ui.spots, []);
  assert.deepEqual(record.elements.spots, []); // The check's actual rows, independent of bundle/UI overlay.
  assert.equal(record.revision, 1);
  assert.deepEqual(paths, [`/atlas/projects/${published.uuid}/atlas.json`, `/atlas/projects/${published.uuid}/project.json`]);
  assert.equal([...map.values()].some(value => value.includes('synthetic-token')), false);
  console.info(JSON.stringify({ bundleCache: 'warm identity', baseline, after: { bundleRequests: paths.length, identityRequests: 0, ms: performance.now() - started } }));
});

for (const hosted of [false, true]) {
  test(`server open skips bundle caching ${hosted ? 'for a hosted tab with a mirror binding' : 'without a mirror binding and never looks up identity'}`, async t => {
    const { mountMirror } = await import('./mount-mirror.ts');
    const { browserAccount, hideMountCache, mountSession } = await import('./mount-browser.ts');
    const { openFromServer } = await import('./remote.ts');
    const { oversightSession } = await import('./oversight.ts');
    const oldScope = oversightSession.state; oversightSession.set(null);
    t.after(() => { oversightSession.set(oldScope); app.hosted = false; });
    const { atlas, published, reply } = await serverOpenSetup(t);
    atlas.project_id = published.uuid;
    const map = new Map(), paths = [];
    let bindingRead;
    const read = new Promise(resolve => { bindingRead = resolve; });
    const get = t.mock.method(mountMirror, 'getItem', key => { if (key.startsWith('mirror-bindings/')) bindingRead(); return map.get(key) ?? null; });
    const set = t.mock.method(mountMirror, 'setItem', (key, value) => map.set(key, value));
    t.mock.method(mountMirror, 'removeItem', key => map.delete(key));
    hideMountCache();
    t.mock.method(globalThis, 'fetch', async url => {
      const path = new URL(url).pathname; paths.push(path);
      if (path.endsWith('/atlas.json')) return reply(atlas);
      if (path.endsWith('/project.json')) return reply(published);
      if (hosted) return Response.json(path.endsWith('/accounts/me') ? { id: 'synthetic-account' } : path.endsWith('/v1/server') ? { dataset_id: 'synthetic-dataset' } : []);
      return Response.json({ detail: 'Synthetic refused identity' }, { status: 401 });
    });
    if (hosted) await browserAccount('https://quiet.test', 'synthetic-token', mountMirror);
    const epoch = mountSession(), readsBefore = get.mock.callCount(), writesBefore = set.mock.callCount();
    paths.length = 0; app.hosted = hosted;
    const started = performance.now();
    assert.equal(await openFromServer('https://quiet.test', published.uuid, null, null, 'mount', app), true);
    if (!hosted) await read;
    // Let the fire-and-forget cache continuation finish after the observed binding read.
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(paths, [`/atlas/projects/${published.uuid}/atlas.json`, `/atlas/projects/${published.uuid}/project.json`]);
    assert.equal(set.mock.callCount(), writesBefore);
    assert.equal(get.mock.callCount() - readsBefore, hosted ? 0 : 1);
    assert.equal(mountSession(), epoch, 'cache work must not invalidate the signed-in mirror');
    assert.equal(app.project.uuid, published.uuid); assert.equal(app.screen, 'canvas');
    console.info(JSON.stringify({ bundleCache: hosted ? 'hosted tab' : 'no binding', bundleRequests: paths.length, identityRequests: 0, ms: performance.now() - started }));
  });
}


test('manual opening checks recognise a clean linked copy without acknowledging an upload', async t => {
  const { session, base, hashes } = quietSession('manual-clean-open-synthetic');
  const { oversightSession } = await import('./oversight.ts');
  const scope = oversightSession.state; oversightSession.set(null);
  t.after(() => { session.dispose(); app.link.syncAutomatically = true; app.hosted = false; oversightSession.set(scope); });
  app.link.syncAutomatically = false;
  // Package reopen now restores its stitched mosaic before the opening check.
  app.pkg = { name: 'Synthetic' };
  app.stitched = { area: { mosaic: structuredClone(source.areas[0].mosaic) } };
  const pixels = pixelSignature(app.fullProject());
  session.save({ synced: false, pixels_base: pixels }); flushWrites();
  const status = structuredClone(session.status);
  const reads = [];
  session.api = async () => ({
    lookup: async () => { reads.push('lookup'); return structuredClone(status); },
    elements: async () => { reads.push('elements'); return { ...structuredClone(base), revision: 1, hashes }; },
    sync: async () => { assert.fail('An opening check must never upload'); },
  });
  assert.equal(session.text, 'Not synced', 'a historical flag alone cannot establish freshness');
  session.opened(); await session.check();
  assert.deepEqual(reads, ['lookup', 'elements']);
  assert.equal(session.text, 'Up to date');
  assert.equal(session.unsynced, false);
  assert.equal(session.draft.synced, false, 'a read is not a successful sync acknowledgement');
  assert.equal(session.draft.pending, null);
  flushWrites();
  assert.equal(JSON.parse(storage.get('grainatlas.draft.manual-clean-open-synthetic')).synced, false);

  const clean = session.draft;
  for (const patch of [
    { pending_request: { id: 'synthetic-unacknowledged-request' } },
    { publish_job: { result: { version: 2 } } },
    { mount: null }, { base: null }, { pixels_base: null },
  ]) {
    session.draft = { ...clean, ...patch };
    assert.equal(session.unsynced, true, JSON.stringify(patch));
    assert.equal(session.text, 'Not synced', JSON.stringify(patch));
  }
  session.draft = clean;
  app.stitched.area.mosaic.sha256 = 'synthetic-local-correction';
  assert.equal(session.pixelsChanged, true);
  assert.equal(session.unsynced, true);
  assert.equal(session.text, 'Not synced', 'restored but different pixels still need upload');
  app.stitched.area.mosaic.sha256 = source.areas[0].mosaic.sha256;
  addQuietSpot(); session.track(); flushWrites();
  assert.equal(session.text, '1 change not synced');
  assert.equal(JSON.parse(storage.get('grainatlas.draft.manual-clean-open-synthetic')).pending.spots.length, 1);
  app.spots = []; session.track();
  assert.equal(session.text, 'Up to date', 'undo restores the checked clean state');
  app.hosted = true;
  assert.equal(session.unsynced, true, 'hosted acknowledgement semantics are unchanged');
});

test('manual sessions persist edits and recovered outbox without scheduling elements, images or polls', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { session } = quietSession('manual-scheduling-synthetic');
  const { oversightSession } = await import('./oversight.ts');
  const scope = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(scope));
  app.link.syncAutomatically = false;
  t.after(() => { session.dispose(); app.link.syncAutomatically = true; });
  let syncs = 0, images = 0, checks = 0;
  session.sync = async () => { syncs++; };
  session.publishImages = async () => { images++; };
  session.check = async () => { checks++; };
  addQuietSpot(); session.track(); flushWrites();
  session.draft = { ...session.draft, pending_request: { id: 'durable-request' },
    publish_job: { result: { version: 2 } } };
  for (let i = 0; i < 10; i++) {
    session.refreshAutomatic(); session.wakeAutomatic(); await session.poll();
    t.mock.timers.tick(30_000);
  }
  assert.deepEqual({ syncs, images, checks }, { syncs: 0, images: 0, checks: 0 });
  const stored = JSON.parse(storage.get('grainatlas.draft.manual-scheduling-synthetic'));
  assert.equal(stored.pending.spots[0].name, 'Synthetic spot', 'manual mode still persists the exact edit');
  assert.equal(session.unsynced, true);
  assert.equal(session.text, '1 change not synced');
  app.link.syncAutomatically = true;
  session.refreshAutomatic();
  t.mock.timers.tick(3000);
  assert.equal(syncs, 1, 'opting in wakes the existing durable work');
});

test('close reminders include images and outbox counts, retain failed work and await local durability', async t => {
  const { syncReminder, reminderCounts, needsCloseReminder, syncForClose, keepLocalForClose } = await import('./close-sync.ts');
  const { session } = quietSession('manual-close-synthetic');
  const { oversightSession } = await import('./oversight.ts');
  const scope = oversightSession.state; oversightSession.set(null); t.after(() => oversightSession.set(scope));
  app.link.syncAutomatically = false; app.dirty = false; app.storageWarning = null;
  t.after(() => { session.dispose(); app.link.syncAutomatically = true; app.tims.store = null; });
  addQuietSpot(); session.track();
  session.save({ pixels_base: '[]', pixels_key: pixelSignature(source) });
  app.tims.store = { data: { outbox: [{ id: 'pluck' }] }, persist: async () => {},
    setAutomatic() {}, mountSynced() {}, flush: async () => {} };
  const reminder = syncReminder(app);
  assert.equal(reminder.changes, 1); assert.equal(reminder.images, 1); assert.equal(reminder.outbox, 1);
  assert.match(reminderCounts(reminder), /1 change not synced.*1 image set pending.*1 outbox entry/);
  assert.equal(needsCloseReminder(app), true);
  session.sync = async () => { session.error = 'Database 503: synthetic outage'; };
  assert.equal(await syncForClose(app), false);
  assert.match(app.error, /Could not sync and close.*Database 503/);
  assert.equal(session.unsynced, true);
  app.storageWarning = 'Synthetic storage full';
  await assert.rejects(keepLocalForClose(app), /Synthetic storage full/);
  app.storageWarning = null;
  session.sync = async () => {
    session.error = ''; session.save({ base: structuredClone(session.elements),
      pixels_base: null, pending: null, changes: { spots: 0, grains: 0, labels: 0 } });
  };
  app.tims.store.flush = async () => { app.tims.store.data.outbox = []; };
  assert.equal(await syncForClose(app), true);
  assert.equal(needsCloseReminder(app), false);
});


test('close failures expose HTTP status and a bounded reply while retaining the exact durable edit', async t => {
  const { syncForClose, needsCloseReminder } = await import('./close-sync.ts');
  const { SyncApi } = await import('./sync-api.ts');
  const { oversightSession } = await import('./oversight.ts');
  const scope = oversightSession.state; oversightSession.set(null);
  t.after(() => { app.link.syncAutomatically = true; oversightSession.set(scope); });
  t.mock.method(console, 'warn', () => {});
  for (const reply of [
    { status: 503, body: JSON.stringify({ detail: 'Synthetic database maintenance' }) },
    { status: 403, body: 'Synthetic permission refusal' },
    { status: 503, body: `Synthetic database maintenance\n${'x'.repeat(1000)} END OF REPLY` },
  ]) {
    const { session, base, hashes } = quietSession(`close-http-${reply.status}-${reply.body.length}-synthetic`);
    app.link.syncAutomatically = false; app.dirty = false; app.storageWarning = null;
    addQuietSpot(); session.track(); flushWrites();
    const pending = structuredClone(session.draft.pending);
    const fetch = t.mock.method(globalThis, 'fetch', async (url, init) => {
      if (String(url).endsWith('/elements')) return Response.json({ ...base, revision: 1, hashes });
      assert.equal(init.method, 'POST');
      assert.ok(String(url).endsWith('/sync'));
      return new Response(reply.body, { status: reply.status });
    });
    session.api = async () => new SyncApi('https://quiet.test', null);
    try {
      assert.equal(await syncForClose(app), false);
      assert.match(app.error, new RegExp(`${reply.status}.*Synthetic`));
      assert.equal(app.notice, null, 'failure uses the persistent alert, not the six-second notice');
      assert.equal(session.errorDetails, `${reply.status} ${reply.body}`, 'full details remain available');
      if (reply.body.length > 300) {
        assert.ok(app.error.endsWith('…)'));
        assert.ok(!app.error.includes('END OF REPLY'));
        assert.ok(app.error.length < 450);
      }
      assert.equal(needsCloseReminder(app), true);
      assert.deepEqual(session.draft.pending, pending);
      assert.deepEqual(JSON.parse(storage.get(`grainatlas.draft.${session.draft.uuid}`)).pending, pending);
      assert.equal(fetch.mock.callCount(), 2, 'one elements read and one refused POST, with no extra request');
    } finally {
      session.dispose(); fetch.mock.restore();
    }
  }
});

test('pixel-only work still warns after dirty clears and failed image publication cannot close it', async t => {
  const { needsCloseReminder, syncReminder, syncForClose } = await import('./close-sync.ts');
  const { session } = quietSession('pixel-close-synthetic');
  const { oversightSession } = await import('./oversight.ts');
  const scope = oversightSession.state; oversightSession.set(null);
  app.link.syncAutomatically = false; app.dirty = false; app.storageWarning = null;
  t.after(() => { session.dispose(); app.link.syncAutomatically = true; oversightSession.set(scope); });
  const pixels = pixelSignature(source);
  session.save({ pixels_base: '[]', pixels_key: pixels });
  assert.deepEqual(session.counts, { spots: 0, grains: 0, labels: 0 });
  assert.equal(syncReminder(app).images, 1);
  assert.equal(session.unsynced, true);
  assert.equal(needsCloseReminder(app), true, 'saved files do not clear pending image work');
  session.sync = async () => {};
  let publishes = 0;
  session.publishImages = async () => { publishes++; session.imageError = 'Synthetic image upload refused'; };
  assert.equal(await syncForClose(app), false);
  assert.equal(publishes, 1);
  assert.match(app.error, /Could not sync and close.*Synthetic image upload refused/);
  assert.equal(needsCloseReminder(app), true);
  session.publishImages = async () => { publishes++; session.imageError = ''; session.save({ pixels_base: pixels }); };
  assert.equal(await syncForClose(app), true);
  assert.equal(publishes, 2);
  assert.equal(needsCloseReminder(app), false);
});
