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
    if (url.endsWith('.svelte.ts')) {
      const source = ts.transpileModule(String(result.source), { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } }).outputText;
      return { format: 'module', shortCircuit: true, source: compileModule(source, { filename: new URL(url).pathname, generate: 'client' }).js.code };
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
    assert.equal(session.text.includes('New images not published'), f.pixels, f.name);
    if (!f.pending) assert.equal(session.text, 'In sync · v1 r2', f.name);
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
  assert.equal(session.text.includes('New images not published'), false);
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
    assert.equal(session.text.includes('New images not published'), true);
    assert.equal(session.unsynced, false, 'view mode still suppresses the quit warning');
    session.save({ pixels_base: null });
    assert.equal(session.pixelsChanged, false, 'a newly learned unknown base refreshes the view');
  } finally { app.viewMode = false; flushWrites(); }
});
