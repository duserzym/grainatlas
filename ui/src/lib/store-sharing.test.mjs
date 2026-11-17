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
  hooks.deregister();
});
