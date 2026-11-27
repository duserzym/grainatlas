import assert from 'node:assert/strict';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import { registerHooks } from 'node:module';

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
test.after(() => hooks.deregister());
const { tabs } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const { flushWrites } = await import('./deferred-writes.ts');
const { neutralTone } = await import('./bundle-tone.ts');
const { openFiles, openPaths, loadProjectFile, restoreAreas, stitch, liveChanged, liveIdle } = await import('./actions.ts');
const { openRecent } = await import('./recent.ts');
const { stitchSettingsChanged, stitchSourceChanged } = await import('./stitch-state.ts');
const { recoverStitchResults, storedCopyKey } = await import('./stored-copy.ts');

function fixture() {
  const tile = { file: 'synthetic.png', sha256: 'invented-hash', pos_um: [0, 0], stage_nm: null, pixel_size_nm: 1000, w: 64, h: 64 };
  const params = { stitch: { min_psr: 6, coarse_factor: 4 }, photometric: {}, blend: {}, tone: neutralTone(), segment: { signal: null } };
  const placement = { tiles: [{ index: 0, x: 0, y: 0, w: 64, h: 64, prior: [0, 0], pos: [0, 0], drift: [0, 0] }], pairs: [], width: 64, height: 64 };
  const mosaic = { w: 64, h: 64, sha256: 'invented-pixels', um_per_px: 1, levels: [{ level: 0, w: 64, h: 64, cols: 1, rows: 1 }] };
  const area = { id: 'a', name: 'Synthetic area', pixel_size_nm: 1000, tiles: [tile], params, placement, mosaic };
  const project = { uuid: 'synthetic-reopen', name: 'Synthetic reopen', package: { name: 'Synthetic reopen', path: '/synthetic/package', kind: 'package' }, areas: [area], settings: {}, upbdb: { mount_id: 'synthetic-mount' }, grainatlas_ui: {
    grains: { a: [{ id: 'registered-grain', number: 1, bbox_um: [2, 2, 20, 20], polygon_um: [[2, 2], [20, 2], [2, 20]] }] },
    spots: [{ id: 'registered-spot', area_id: 'a', grain_id: 'registered-grain', x_um: 6, y_um: 7, acquisition_ids: ['registered-link'] }], labels: [], tone: {},
  } };
  const info = { name: project.package.name, kind: 'package', areas: [{ ...area, tiles: [{ ...tile, index: 0 }] }] };
  return { project, info };
}

function setup(t) {
  const { project, info } = fixture();
  const app = tabs.add(), storage = new Map(), renders = [];
  let record = structuredClone(project);
  t.mock.method(appStorage, 'getItem', key => { flushWrites(); return storage.get(key) ?? null; });
  t.mock.method(appStorage, 'setItem', (key, value) => storage.set(key, value));
  t.mock.method(appStorage, 'keys', () => [...storage.keys()]);
  t.mock.method(appStorage, 'track', (_owner, _kind, write) => write());
  t.mock.method(appStorage, 'trackDurable', async (_owner, _kind, write) => write());
  t.mock.method(app.sync, 'check', async () => {});
  t.mock.method(app.sync, 'track', () => {});
  t.mock.method(app.sync, 'imagesEdited', () => {});
  const core = {
    openFiles: async () => ({ status: 'opened', info }), openPaths: async () => ({ status: 'opened', info }),
    openProject: async () => ({ info, project, modified_ms: 0 }),
    loadProject: async json => { record = JSON.parse(json); }, projectJson: async () => JSON.stringify(record),
    setGrains: async () => {}, dispose: async () => {}, preview: async () => null,
    recompute: async (id, params) => { renders.push(structuredClone(params)); return { mosaic: record.areas[0].mosaic, placement: record.areas[0].placement }; },
    stitch: async () => { record = structuredClone(project); return { mosaic: record.areas[0].mosaic, placement: record.areas[0].placement }; },
  };
  app.core = Promise.resolve(core);
  storage.set('grainatlas.project.' + project.package.name, JSON.stringify(project));
  t.after(() => { flushWrites(); tabs.remove(app.id); });
  return { app, project, info, core, storage, renders };
}

for (const kind of ['package path', 'kept package files', 'project folder', 'gaproj path', 'gaproj file', 'local column', 'database column']) {
  test(`reopens ${kind} from saved placement without registration or changing registered rows`, async t => {
    const { app, project, renders } = setup(t);
    if (kind === 'kept package files') await openFiles(project.package.name, [], app);
    else if (kind === 'gaproj file') {
      app.pkg = fixture().info; app.screen = 'area';
      await loadProjectFile(new File([JSON.stringify(project)], 'Synthetic.gaproj'), app, true);
    } else if (kind.endsWith('column')) {
      // Both start-screen columns call Open.svelte's localOpen -> openRecent -> openPaths.
      const result = await openRecent({ id: project.uuid, name: project.package.name, when: '', areas: 1, spots: 1, package_path: project.package.path }, project, {
        canOpenPaths: true, focusExisting: () => false, status: async () => 'present', open: async location => { await openPaths([location.path], app); },
      });
      assert.equal(result.kind, 'opened');
    } else await openPaths([kind === 'project folder' ? '/synthetic/project.grainatlas' : kind === 'gaproj path' ? '/synthetic/project.gaproj' : project.package.path], app);
    assert.equal(renders.length, 1, 'render the persisted placement exactly once');
    assert.ok(app.stitched.a?.mosaic, 'Canvas has a real restored mosaic');
    assert.equal(app.screen, 'canvas');
    assert.deepEqual(app.spots, project.grainatlas_ui.spots);
    assert.deepEqual(app.grains, project.grainatlas_ui.grains);
    assert.deepEqual(app.project.areas[0].placement, project.areas[0].placement);
    assert.equal(app.dirty, false);
  });
}

test('restore uses applied settings while preserving pending stitch settings', async t => {
  const { app, project, info, renders } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project); app.screen = 'area';
  app.params.a.stitch.min_psr = 12;
  await restoreAreas(app);
  assert.equal(renders[0].stitch.min_psr, 6);
  assert.equal(app.params.a.stitch.min_psr, 12, 'restoring pixels must not discard a pending edit');
  assert.equal(app.screen, 'canvas');
});

test('reopening files in the same tab re-sends grains after the worker clears its package state', async t => {
  const { app, project, info, core } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  const calls = t.mock.method(core, 'setGrains', async () => {});
  await restoreAreas(app);
  assert.equal(calls.mock.callCount(), 1);
  await openFiles(project.package.name, [], app);
  assert.equal(calls.mock.callCount(), 2);
  assert.deepEqual(calls.mock.calls[1].arguments[1], project.grainatlas_ui.grains.a);
});

test('stitch waits for a durable placement write before completing', async t => {
  const { app, project, info, storage } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  let release, writes = 0, finished = false;
  const gate = new Promise(resolve => { release = resolve; });
  t.mock.method(appStorage, 'trackDurable', async (_owner, _kind, write) => { writes++; await gate; return write(); });
  const pending = stitch('a', app).then(result => { finished = true; return result; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(writes, 1);
  assert.equal(finished, false);
  release();
  assert.equal(await pending, true);
  const saved = JSON.parse(storage.get('grainatlas.project.' + project.package.name));
  assert.deepEqual(saved.areas[0].placement, project.areas[0].placement);
  assert.deepEqual(saved.grainatlas_ui.spots, project.grainatlas_ui.spots);
});

test('failed durable placement write does not report stitch success', async t => {
  const { app, project, info } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  t.mock.method(appStorage, 'trackDurable', async () => false);
  assert.equal(await stitch('a', app), false);
  assert.ok(app.stitched.a, 'the rendered result remains viewable');
});

test('pending stitch settings round-trip without replacing applied placement settings', async t => {
  const { app, project, info, storage, renders } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  app.params.a.stitch.min_psr = 12;
  await app.persistDurably();
  const saved = JSON.parse(storage.get('grainatlas.project.' + project.package.name));
  assert.equal(saved.areas[0].params.stitch.min_psr, 6);
  assert.equal(saved.grainatlas_ui.stitch_settings.a.min_psr, 12);
  await openFiles(project.package.name, [], app);
  assert.equal(renders[0].stitch.min_psr, 6);
  assert.equal(app.params.a.stitch.min_psr, 12);
  assert.ok(app.stitched.a);
});

test('live pixel edits keep applied registration when stitch settings are pending', async t => {
  const { app, project, info, renders } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  await restoreAreas(app);
  app.params.a.stitch.min_psr = 12;
  const previous = globalThis.window;
  globalThis.window = { setTimeout };
  try {
    liveChanged('a', 0, app);
    await liveIdle('a', app);
  } finally { globalThis.window = previous; }
  assert.equal(renders.at(-1).stitch.min_psr, 6);
  assert.equal(app.params.a.stitch.min_psr, 12);
  assert.deepEqual(app.spots, project.grainatlas_ui.spots);
});

test('legacy field defaults and key order do not require a re-stitch', () => {
  const applied = { min_psr: 6, consensus_px: 3, refine_full: true };
  assert.equal(stitchSettingsChanged(applied, { refine_full: true, min_psr: 6 }), false);
  assert.equal(stitchSettingsChanged(applied, { ...applied, min_psr: 7 }), true);
});

test('raw hash changes mark the placement stale but keep the saved mosaic viewable', async t => {
  const { app, project, info, core } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  const changed = structuredClone(project);
  changed.areas[0].tiles[0].sha256 = 'new-raw-bytes';
  t.mock.method(core, 'projectJson', async () => JSON.stringify(changed));
  await restoreAreas(app);
  assert.ok(app.stitched.a);
  assert.equal(stitchSourceChanged(app.stitchSources.a, app.project.areas[0], info.areas[0]), true);
  assert.deepEqual(app.spots, project.grainatlas_ui.spots);
  const saved = app.fullProject();
  app.adopt(saved);
  assert.equal(stitchSourceChanged(app.stitchSources.a, app.project.areas[0]), true);
});

test('a render failure in one area does not hide another saved area', async t => {
  const { app, project, info, core } = setup(t);
  project.areas.push({ ...structuredClone(project.areas[0]), id: 'b' });
  info.areas.push({ ...structuredClone(info.areas[0]), id: 'b' });
  await core.loadProject(JSON.stringify(project));
  app.project = project; app.pkg = info; app.adopt(project); app.screen = 'area'; app.areaId = 'a';
  t.mock.method(core, 'recompute', async id => {
    if (id === 'a') throw new Error('Synthetic missing tile');
    return { mosaic: project.areas[1].mosaic, placement: project.areas[1].placement };
  });
  await restoreAreas(app);
  assert.ok(app.stitched.b);
  assert.equal(app.screen, 'canvas');
  assert.equal(app.areaId, 'b');
  assert.match(app.error, /Synthetic missing tile/);
});

test('a fresh package without placement stays on Area without registering automatically', async t => {
  const { app, project, core, storage, renders } = setup(t);
  delete project.areas[0].placement; delete project.areas[0].mosaic;
  await core.loadProject(JSON.stringify(project));
  storage.clear();
  await openPaths([project.package.path], app);
  assert.equal(renders.length, 0);
  assert.deepEqual(app.stitched, {});
  assert.equal(app.screen, 'area');
});

test('saved paths recover a durably kept stitch without changing the file registered rows', async t => {
  const { app, project, info, storage, renders } = setup(t);
  app.project = project; app.pkg = info; app.adopt(project);
  app.projectPath = { kind: 'file', path: '/synthetic/project.gaproj' };
  assert.equal(await stitch('a', app), true);
  assert.ok(JSON.parse(storage.get(storedCopyKey(project.uuid, app.projectPath.path))).project.areas[0].placement);
  project.areas[0].placement = null; project.areas[0].mosaic = null;
  // Deliberately different copied rows must not leak into registered geometry.
  const raw = JSON.parse(storage.get(storedCopyKey(project.uuid, app.projectPath.path)));
  raw.project.grainatlas_ui.spots[0].x_um = 999;
  storage.set(storedCopyKey(project.uuid, app.projectPath.path), JSON.stringify(raw));
  app.packageSource = null; app.project = null; app.pkg = null; app.stitched = {};
  await openPaths(['/synthetic/project.gaproj'], app);
  assert.equal(renders.length, 1);
  assert.ok(app.stitched.a);
  assert.equal(app.spots[0].x_um, 6);
  assert.ok(app.storedCopy?.ready, 'existing element recovery choice is preserved');
});

test('stitch recovery rejects an older, moved or mismatched copy', () => {
  const { project } = fixture();
  const path = '/synthetic/project.gaproj';
  const copy = { project: structuredClone(project), path, saved_at: '2026-01-02T00:00:00Z' };
  const storage = { getItem: () => JSON.stringify(copy) };
  const modified = Date.parse('2026-01-01T00:00:00Z');
  for (const change of [() => { copy.path = '/other.gaproj'; }, () => { copy.project.uuid = 'other'; },
    () => { copy.saved_at = '2025-01-01'; }, () => { copy.saved_at = 'invalid'; },
    () => { copy.project.areas[0].tiles[0].file = 'different.png'; },
    () => { copy.project.areas[0].stage_sign = [-1, -1]; }]) {
    const original = structuredClone(copy);
    change(); assert.equal(recoverStitchResults(project, path, modified, storage), project);
    Object.assign(copy, original);
  }
});
