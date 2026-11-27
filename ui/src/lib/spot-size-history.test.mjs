import assert from 'node:assert/strict';
import test from 'node:test';
import '../../../scripts/perf/store-loader.mjs';
import { syncFixture } from '../../../scripts/perf/sync-fixture.ts';
import { SpotSizeEdit } from './spot-size-edit.ts';
import { SizeEditor } from './size-editor.ts';
import { selectionSize } from './spot-size.ts';
import { pendingElements } from './drafts.ts';
const { current } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const { flushWrites } = await import('./deferred-writes.ts');
const copy = value => JSON.parse(JSON.stringify(value));

for (const control of ['toolbar', 'presets', 'stepper', 'floating panel']) {
  for (const deselect of [false, true]) test(`${control}: group 25→20, next placement, auto-selection and resize${deselect ? ' after clearing selection' : ' while selected'}`, () => {
    const app = current(), project = syncFixture();
    const memory = new Map();
    appStorage.getItem = key => { flushWrites(); return memory.get(key) ?? null; };
    appStorage.setItem = (key, value) => memory.set(key, value);
    for (const spot of project.grainatlas_ui.spots) spot.diameter_um = 25;
    app.loadingProject = true; app.project = project; app.adopt(project); app.viewMode = false;
    app.link.syncAutomatically = false; app.areaId = app.spots[0].area_id;
    app.loadingProject = false; app.clearHistory();
    app.sync.draft = { uuid: project.uuid, name: project.name, mount: project.upbdb.mount_id, server: 'https://example.invalid',
      base: copy(app.sync.elements), base_revision: 1, synced: true, pending: null, pixels_base: null,
      pixels_changed: false, updated_at: 'fixed', changes: { spots: 0, grains: 0, labels: 0 } };
    const assertOutbox = () => {
      app.flushPersistence();
      const durable = JSON.parse(memory.get(`grainatlas.draft.${project.uuid}`));
      assert.deepEqual(pendingElements(durable.pending ?? durable.base).spots,
        pendingElements({ spots: copy(app.spots), grains: [], labels: [] }).spots);
    };
    app.selection = app.spots.slice(0, 2).map(s => s.id);
    const before = copy(app.spots), edit = new SpotSizeEdit();
    let tick = 0;
    const change = diameter => {
      const selected = app.spots.filter(s => app.selection.includes(s.id));
      const write = d => edit.change(app, project.uuid, app.areaId, selected.map(s => s.id), d, tick++);
      const value = selectionSize(selected, app.diameter).value;
      if (control === 'presets') write(diameter);
      else {
        const field = new SizeEditor(), target = JSON.stringify(app.selection);
        field.sync(value, target); field.focus(target, write);
        if (control === 'stepper') {
          for (let d = value; d !== diameter;) {
            const previous = d; d += Math.sign(diameter - d);
            assert.equal(field.commit(String(d), previous, target), d);
          }
        } else assert.equal(field.commit(String(diameter), value, target), diameter);
      }
    };
    change(20);
    const grouped = copy(app.spots);
    assert.deepEqual(grouped, before.map((s, i) => i < 2 ? { ...s, diameter_um: 20 } : s));
    assertOutbox();
    if (deselect) { app.selection = []; change(25); }
    change(20);
    // Canvas.makeSpot reads this value even while the old selection remains.
    assert.equal(app.diameter, 20);
    for (let n = 0; n < 2; n++) {
      const spot = { ...before[0], id: `synthetic-new-${n}`, diameter_um: app.diameter, created: `2026-11-27T00:00:0${n}Z` };
      app.checkpoint(); app.spots = [...app.spots, spot]; app.selection = [spot.id]; app.persist();
      assert.equal(app.spots.at(-1).diameter_um, 20);
      assertOutbox();
    }
    const placed = copy(app.spots);
    change(15);
    const resized = copy(app.spots);
    assert.deepEqual(resized, placed.map((s, i) => i === placed.length - 1 ? { ...s, diameter_um: 15 } : s));
    assertOutbox();
    app.undo(); assert.deepEqual(copy(app.spots), placed);
    app.undo(); assert.deepEqual(copy(app.spots), placed.slice(0, -1));
    app.undo(); assert.deepEqual(copy(app.spots), grouped);
    app.undo(); assert.deepEqual(copy(app.spots), before); assert.equal(app.canUndo, false);
    assertOutbox();
    for (let i = 0; i < 4; i++) app.redo();
    assert.deepEqual(copy(app.spots), resized);
    assertOutbox();
  });
}

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
