import assert from 'node:assert/strict';
import test from 'node:test';
import '../../../scripts/perf/store-loader.mjs';
import { syncFixture } from '../../../scripts/perf/sync-fixture.ts';
import { SpotSizeEdit } from './spot-size-edit.ts';
const { current } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const { flushWrites } = await import('./deferred-writes.ts');
const copy = value => JSON.parse(JSON.stringify(value));

test('mixed registered sizes redraw/index, persist to the sync outbox and undo/redo exactly in one transaction', async () => {
  const { SpotIndex } = await import('./canvas-pointer.ts');
  const memory = new Map(), app = current(), project = syncFixture();
  project.grainatlas_ui.spots[0].diameter_um = 15;
  project.grainatlas_ui.spots[1].diameter_um = 40;
  appStorage.getItem = key => { flushWrites(); return memory.get(key) ?? null; };
  appStorage.setItem = (key, value) => memory.set(key, value);
  app.loadingProject = true; app.project = project; app.adopt(project); app.viewMode = false;
  app.link.syncAutomatically = false; app.loadingProject = false; app.clearHistory();
  const base = copy(app.sync.elements), before = copy(app.spots), unrelated = JSON.stringify({ grains: app.grains, labels: app.labels });
  app.sync.draft = { uuid: project.uuid, name: project.name, mount: project.upbdb.mount_id, server: 'https://example.invalid', base, base_revision: 1, synced: true, pending: null, pixels_base: null, pixels_changed: false, updated_at: 'fixed', changes: { spots: 0, grains: 0, labels: 0 } };
  const edit = new SpotSizeEdit(), ids = before.slice(0, 2).map(s => s.id), area = before[0].area_id;
  edit.change(app, project.uuid, area, ids, 45, 0);
  edit.change(app, project.uuid, area, ids, 46, 1);
  app.flushPersistence();
  assert.equal(app.spots[0].diameter_um, 46); assert.equal(app.spots[1].diameter_um, 46);
  for (let i = 0; i < 2; i++) assert.deepEqual(copy(app.spots[i]), { ...before[i], diameter_um: 46 });
  assert.deepEqual(app.spots.slice(2), before.slice(2));
  assert.equal(JSON.stringify({ grains: app.grains, labels: app.labels }), unrelated);
  assert.deepEqual(app.sync.draft.changes, { spots: 2, grains: 0, labels: 0 });
  const durable = JSON.parse(memory.get(`grainatlas.draft.${project.uuid}`));
  for (const id of ids) assert.equal(durable.pending.spots.find(s => s.id === id).diameter_um, 46);
  // Same immutable-list invalidation used by Canvas's derived index.
  const first = app.spots[0], x = first.x_um + 22;
  assert.equal(new SpotIndex([first]).hit(x, first.y_um, .01)?.id, first.id);
  app.undo(); app.flushPersistence();
  assert.deepEqual(copy(app.spots), before); assert.equal(app.canUndo, false);
  assert.deepEqual(app.sync.draft.changes, { spots: 0, grains: 0, labels: 0 });
  assert.equal(new SpotIndex([app.spots[0]]).hit(x, first.y_um, .01), null);
  app.redo(); app.flushPersistence();
  assert.equal(app.spots[0].diameter_um, 46);
  // A grain/label checkpoint leaves the spots reference unchanged: it still breaks a size run.
  app.checkpoint(); app.labels = [...app.labels]; app.persist();
  edit.change(app, project.uuid, area, ids, 47, 2);
  app.undo(); app.flushPersistence(); assert.equal(app.spots[0].diameter_um, 46);
  app.undo(); app.undo(); app.flushPersistence(); assert.deepEqual(copy(app.spots), before);
});
