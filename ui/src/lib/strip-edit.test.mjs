import assert from 'node:assert/strict';
import test from 'node:test';
import { bandImageKey, bandRect, bodyOrigin, cropRect, firstFreePlace, grainChanges, keepPrintEdits, keyId, nudgeOrigin, readPrintEdits, snapGap, snapOrigin, splitPairs, storePrintEdits, stripBlock, StripHistory, targetGrains } from './strip-edit.ts';
import { answerStoredCopy, newerStoredCopy, persistProject } from './stored-copy.ts';

const key = (id, split = 0) => ({ area: 'a1', grains_hash: id, split });
const band = (id = 'b1') => ({ key: key(id), offset_mm: 3, grains: ['g1'], mosaic_rect_um: [0, 0, 100, 80] });
const strip = (id = 's1', more = {}) => ({ key: key(id), area: 'a1', sheet: 0, origin_mm: [10, 50], size_mm: [30, 25], labelled: true, bands: [band()], ...more });
const plan = (strips = [strip()]) => ({ strips, sheet_bodies: [{ body_rect_mm: [8, 40, 200, 250] }], targets: { a1: ['g1', 'g2'] } });
const grain = (id, bbox_um) => ({ id, bbox_um });

test('block and band geometry match paper mapping and the unlabelled divider', () => {
  const s = strip();
  assert.deepEqual(stripBlock(s), [10, 50, 40, 75]);
  assert.deepEqual(stripBlock({ ...s, labelled: false, shares_gap_with: key('s2') }), [7.5, 50, 40, 75]);
  assert.deepEqual(stripBlock(s, [20, 60]), [20, 60, 50, 85]);
  assert.deepEqual(bandRect(s, s.bands[0], 10), [13, 54.6, 23, 62.6]);
});

test('snapping uses body edges and a 2.5 mm gap on the same sheet', () => {
  const s = strip(), p = plan([s, strip('s2', { origin_mm: [60, 100] }), strip('s3', { origin_mm: [150, 50], sheet: 1 })]);
  assert.deepEqual(snapOrigin(p, s, 0, [9, 41]), [8, 40]);
  assert.deepEqual(snapOrigin(p, s, 0, [29, 50]), [27.5, 50]);
  assert.deepEqual(snapOrigin(p, s, 0, [93, 50]), [92.5, 50]);
  assert.deepEqual(snapOrigin(p, s, 0, [123, 50]), [123, 50]);
  assert.deepEqual(snapOrigin(p, s, 0, [9, 41], 0), [9, 41]);
  assert.deepEqual(snapOrigin(plan([strip('s1', { labelled: false })]), strip('s1', { labelled: false }), 0, [11, 50]), [10.5, 50]);
});

test('first free place fills from the top left and permits exact body fits', () => {
  assert.deepEqual(firstFreePlace([0, 0, 100, 100], [100, 100], []), [0, 0]);
  assert.deepEqual(firstFreePlace([0, 0, 100, 100], [30, 30], [[0, 0, 40, 40]]), [42.5, 0]);
  assert.deepEqual(firstFreePlace([0, 0, 100, 100], [80, 30], [[0, 0, 40, 40]]), [0, 42.5]);
  assert.equal(firstFreePlace([0, 0, 100, 100], [101, 30], []), null);
  assert.equal(firstFreePlace([0, 0, 100, 100], [20, 20], [[0, 0, 100, 100]]), null);
});

test('crop edges choose whole-grain gaps, including overlapping boxes and image limits', () => {
  const gs = [grain('g1', [20, 10, 40, 30]), grain('g2', [35, 10, 55, 30]), grain('g3', [70, 120, 90, 140])];
  assert.equal(snapGap(38, 0, [0, 80], gs, 0, 100), 55);
  assert.equal(snapGap(25, 0, [0, 80], gs, 0, 100), 20);
  assert.equal(snapGap(80, 0, [0, 80], gs, 0, 100), 80);
  assert.deepEqual(cropRect([0, 0, 100, 80], 0, 38, [0, 0, 200, 200], gs), [55, 0, 100, 80]);
  assert.deepEqual(cropRect([0, 0, 100, 80], 3, 20, [0, 0, 200, 200], gs), [0, 0, 100, 10]);
  assert.deepEqual(cropRect([0, 0, 100, 80], 2, 300, [0, 0, 200, 200], gs), [0, 0, 200, 80]);
  assert.equal(snapGap(20, 0, [0, 80], [grain('g1', [-1, 0, 101, 80])], 0, 100), null);
});

test('only plan targets snap crops; leaving and entering include context targets', () => {
  const grains = { a1: [grain('g1', [0, 0, 20, 20]), grain('g2', [50, 0, 70, 20]), grain('g3', [25, 0, 45, 20])] };
  const targets = targetGrains(plan(), 'a1', grains);
  assert.deepEqual(targets.map(g => g.id), ['g1', 'g2']);
  const changes = grainChanges([0, 0, 40, 40], [30, 0, 80, 40], targets);
  assert.deepEqual(changes.leaving.map(g => g.id), ['g1']);
  assert.deepEqual(changes.entering.map(g => g.id), ['g2']);
});

test('history branches, confirmation count, automatic and undo share one edit list', () => {
  const h = new StripHistory(), move = { op: 'move', strip: key('s1'), sheet: 0, origin_mm: [20, 60] };
  h.push(move); h.push({ op: 'hide', strip: key('s1') });
  h.load(structuredClone(h.edits)); assert.equal(h.past.length, 2);
  assert.equal(h.count, 2);
  h.undo(); assert.equal(h.count, 1);
  h.redo(); assert.equal(h.count, 2);
  h.automatic(); assert.equal(h.count, 0);
  h.undo(); assert.equal(h.count, 2);
  h.undo(); h.push({ op: 'repack' }); h.redo();
  assert.deepEqual(h.edits, [move, { op: 'repack' }]);
  h.rebase([0]); assert.deepEqual(h.edits, [{ op: 'repack' }]);
  assert.equal(h.past.length, 0);
});

test('a refused thumbnail move is absent from the count, saves, undo and redo', () => {
  const h = new StripHistory();
  const label = { op: 'label', strip: key('s1'), text: 'zr-1' };
  const refused = { op: 'move', strip: key('s1'), sheet: 1, origin_mm: null };
  const p = { ...plan(), params: { packing: 'packed' }, um_per_mm: 4 };
  const app = { project: { uuid: 'p1' }, readOnly: false, dirty: false };
  h.push(label); h.push(refused);
  h.reject(h.edits, [1]);
  assert.equal(h.count, 1);
  assert.deepEqual(h.edits, [label]);
  assert.deepEqual(h.past, [[]]);
  storePrintEdits(app, p, h.edits);
  assert.deepEqual(app.project.print_sheets.edits, [label]);
  h.undo(); assert.deepEqual(h.edits, []);
  h.redo(); assert.deepEqual(h.edits, [label]);
  h.redo(); assert.deepEqual(h.edits, [label]);

  h.automatic(); h.push(refused); h.reject(h.edits, [0]);
  assert.equal(h.count, 0);
  storePrintEdits(app, p, h.edits);
  assert.equal(Object.hasOwn(app.project, 'print_sheets'), false);
  h.undo(); assert.deepEqual(h.edits, [label]);
  h.redo(); assert.deepEqual(h.edits, []);
});

test('coalesced refusals remove only the reported edits and keep useful history steps', () => {
  const h = new StripHistory();
  const split = { op: 'split', band: key('b1'), axis: 'x', at_um: 40 };
  const refusedSplit = { ...split }; // Equal values do not identify the refused operation.
  const refusedMove = { op: 'move', strip: key('s1'), sheet: 1, origin_mm: null };
  // An applied edit with validation problems still belongs in history.
  const overlappingMove = { op: 'move', strip: key('s1'), sheet: 0, origin_mm: [0, 0] };
  for (const edit of [split, refusedSplit, refusedMove, overlappingMove]) h.push(edit);
  h.reject(h.edits, [2, 1]);
  assert.deepEqual(h.edits, [split, overlappingMove]);
  assert.deepEqual(splitPairs(plan(), h.edits), [[key('b1', 1), key('b1', 2)]]);
  assert.deepEqual(h.past, [[], [split]]);
  h.undo(); assert.deepEqual(h.edits, [split]);
  h.undo(); assert.deepEqual(h.edits, []);
  h.redo(); h.redo(); assert.deepEqual(h.edits, [split, overlappingMove]);
});

test('stale render refusals follow edit identity through newer input and undo/redo', () => {
  const label = { op: 'label', strip: key('s1'), text: 'zr-1' };
  const refused = { op: 'move', strip: key('s1'), sheet: 1, origin_mm: null };
  const hide = { op: 'hide', strip: key('s1') };
  for (const undoWhileRendering of [false, true]) {
    const h = new StripHistory();
    h.push(label); h.push(refused);
    const rendered = h.edits;
    if (undoWhileRendering) h.undo(); else h.push(hide);
    h.reject(rendered, [1]);
    assert.deepEqual(h.edits, undoWhileRendering ? [label] : [label, hide]);
    assert.deepEqual(h.future, []);
    h.undo();
    assert.deepEqual(h.edits, undoWhileRendering ? [] : [label]);
    h.redo();
    assert.deepEqual(h.edits, undoWhileRendering ? [label] : [label, hide]);
    assert.equal(rendered[1], refused, 'the submitted snapshot stays intact');
  }
});

test('join pairs follow split counters and ignore refused edits', () => {
  const base = plan([strip('s1', { bands: [band('b1')] })]);
  const edits = [{ op: 'split', band: key('b1'), axis: 'x', at_um: 40 }, { op: 'split', band: key('b1', 1), axis: 'y', at_um: 30 }];
  assert.deepEqual(splitPairs(base, edits), [[key('b1', 1), key('b1', 2)], [key('b1', 3), key('b1', 4)]]);
  assert.deepEqual(splitPairs(base, edits, [1]), [[key('b1', 1), key('b1', 2)]]);
  assert.deepEqual(splitPairs(base, [...edits, { op: 'join', band_a: key('b1', 3), band_b: key('b1', 4) }]), [[key('b1', 1), key('b1', 2)]]);
  assert.notEqual(keyId(key('a/b')), keyId({ area: 'a1/a', grains_hash: 'b', split: 0 }));
});


test('pointer drops cannot enter the header; nudges keep exact paper units', () => {
  const s = strip(), p = plan();
  assert.deepEqual(bodyOrigin(p, s, [0, 0]), [8, 40]);
  assert.deepEqual(bodyOrigin(p, s, [300, 300]), [170, 225]);
  assert.deepEqual(bodyOrigin(p, { ...s, labelled: false }, [0, 0]), [10.5, 40]);
  assert.deepEqual(nudgeOrigin([10, 50], [1, 0]), [10.5, 50]);
  assert.deepEqual(nudgeOrigin([10, 50], [0, -1], true), [10, 45]);
});


test('band preview cache keys include the source rectangle and the stable band key', () => {
  const b = band();
  assert.equal(bandImageKey(b), bandImageKey({ ...b }));
  assert.notEqual(bandImageKey(b), bandImageKey({ ...b, mosaic_rect_um: [0, 0, 90, 80] }));
  assert.notEqual(bandImageKey(b), bandImageKey({ ...b, key: key('b1', 1) }));
});

test('strip edits dirty the project, snapshot the generating parameters and follow undo, redo and automatic', () => {
  const p = { ...plan(), params: { packing: 'packed', um_per_mm: 'auto' }, um_per_mm: 8 };
  const project = { uuid: 'p1', grainatlas_ui: { grains: { a1: [grain('g1', [0, 0, 20, 20])] }, spots: [{ id: 's1' }], labels: [{ id: 'l1' }] }, upbdb: { mount_id: 'm1' }, extra_key: [42] };
  const app = { project: structuredClone(project), readOnly: false, dirty: false };
  const h = new StripHistory(), move = { op: 'move', strip: key('s1'), sheet: 1, origin_mm: [20, 60] };
  h.push(move); storePrintEdits(app, p, h.edits);
  assert.equal(app.dirty, true);
  assert.deepEqual(app.project.print_sheets, { params: p.params, um_per_mm: 8, edits: [move] });
  p.params.packing = 'lines'; move.origin_mm[0] = 99;
  assert.equal(app.project.print_sheets.params.packing, 'packed');
  assert.equal(app.project.print_sheets.edits[0].origin_mm[0], 20);
  h.undo(); storePrintEdits(app, p, h.edits); assert.equal(Object.hasOwn(app.project, 'print_sheets'), false);
  h.redo(); storePrintEdits(app, p, h.edits); assert.equal(app.project.print_sheets.edits.length, 1);
  h.automatic(); storePrintEdits(app, p, h.edits); assert.deepEqual(app.project, project);
});

test('read-only projects never change and replay leaves stored edits intact until the next edit', () => {
  const p = { ...plan(), params: { packing: 'packed', um_per_mm: 4 }, um_per_mm: 4 };
  const stored = { params: p.params, um_per_mm: 4, edits: [{ op: 'hide', strip: key('missing') }, { op: 'label', strip: key('s1'), text: 'zr-1' }] };
  const app = { project: { uuid: 'p1', print_sheets: structuredClone(stored) }, dirty: false, readOnly: true };
  storePrintEdits(app, p, []); assert.equal(app.dirty, false); assert.deepEqual(app.project.print_sheets, stored);
  storePrintEdits(app, p, [{ op: 'repack' }]); assert.deepEqual(app.project.print_sheets, stored);
  const h = new StripHistory(); h.load(stored.edits); h.rebase([0]);
  assert.deepEqual(app.project.print_sheets, stored); assert.equal(app.dirty, false);
  app.readOnly = false; h.push({ op: 'repack' }); storePrintEdits(app, p, h.edits);
  assert.equal(app.dirty, true); assert.deepEqual(app.project.print_sheets.edits, [stored.edits[1], { op: 'repack' }]);
  assert.deepEqual(h.past, [[stored.edits[1]]]);
});

test('core refreshes keep UI-held strip edits, including a cleared list, only for the same project', () => {
  const current = { uuid: 'p1', print_sheets: { params: { packing: 'packed' }, um_per_mm: 8, edits: [{ op: 'repack' }] } };
  assert.deepEqual(keepPrintEdits({ uuid: 'p1', name: 'mount A' }, current), { ...current, name: 'mount A' });
  assert.deepEqual(keepPrintEdits(current, { uuid: 'p1' }), { uuid: 'p1' });
  assert.deepEqual(keepPrintEdits({ uuid: 'p2' }, current), { uuid: 'p2' });
});

test('project recovery restores strip-only edits and keeps grains, spots and labels intact', () => {
  const project = { uuid: 'p1', package: { name: 'mount A' }, areas: [], print_sheets: { params: { packing: 'packed' }, um_per_mm: 8, edits: [{ op: 'repack' }] }, grainatlas_ui: { grains: { a1: [grain('g1', [0, 0, 20, 20])] }, spots: [{ id: 's1' }], labels: [{ id: 'l1', text: 'zr-1' }] } };
  const before = structuredClone(project);
  for (const path of [null, 'mount A.gaproj']) {
    const writes = new Map(), storage = { getItem: k => writes.get(k) ?? null, setItem: (k, v) => writes.set(k, v) };
    assert.equal(persistProject(project, path, '2026-01-01T00:00:00Z', storage), true);
    const copy = JSON.parse([...writes.values()][0]);
    assert.deepEqual(path ? copy.project : copy, project);
    assert.deepEqual(project, before);
    const file = structuredClone(project); delete file.print_sheets;
    const pending = newerStoredCopy(file, path ?? 'mount A.gaproj', Date.parse('2025-12-31T00:00:00Z'), storage);
    assert.ok(pending, 'an edit-only change is offered for recovery'); pending.ready = true;
    let checkpoint = 0, persisted = 0;
    const app = { project: file, storedCopy: pending, checkpoint() { checkpoint++; }, persist() { persisted++; } };
    answerStoredCopy(app, true, storage);
    assert.equal(checkpoint, 1); assert.equal(persisted, 1); assert.equal(app.storedCopy, null);
    assert.deepEqual(app.project.print_sheets, project.print_sheets);
    const history = new StripHistory(); history.load(readPrintEdits(app.project.print_sheets).edits);
    assert.deepEqual(history.edits, project.print_sheets.edits, 'recovered edits are ready for replay');
    assert.deepEqual(app.spots, project.grainatlas_ui.spots);
    assert.deepEqual(app.grains, project.grainatlas_ui.grains);
    assert.deepEqual(app.labels, project.grainatlas_ui.labels);
    assert.deepEqual(project, before);
  }
});

test('recovery restores clearing strip edits and preserves unreadable raw edits', () => {
  for (const raw of [undefined, null, false, { params: {}, edits: [{ op: 'future_edit', value: ['zr-1', 42] }] }]) {
    const file = { uuid: 'p1', package: { name: 'mount A' }, areas: [], print_sheets: { params: {}, edits: [{ op: 'repack' }] } };
    const recovery = { ...file, print_sheets: raw };
    const writes = new Map(), storage = { getItem: k => writes.get(k) ?? null, setItem: (k, v) => writes.set(k, v) };
    persistProject(recovery, 'mount A.gaproj', '2026-01-01T00:00:00Z', storage);
    const pending = newerStoredCopy(file, 'mount A.gaproj', 0, storage); assert.ok(pending); pending.ready = true;
    const app = { project: file, storedCopy: pending, checkpoint() {}, persist() {} };
    answerStoredCopy(app, true, storage);
    assert.deepEqual(app.project.print_sheets, raw);
    assert.deepEqual(keepPrintEdits({ uuid: 'p1' }, app.project).print_sheets, raw);
  }
});

test('unreadable strip edits stay untouched until a new edit replaces them', () => {
  const valid = { params: { packing: 'packed' }, um_per_mm: 8, edits: [{ op: 'label', strip: key('s1'), text: 'zr-1' }] };
  assert.deepEqual(readPrintEdits(valid), valid);
  assert.deepEqual(readPrintEdits({ params: {}, edits: [{ op: 'repack' }] }), { params: {}, um_per_mm: 0, edits: [{ op: 'repack' }] });
  const bad = [null, false, 'zr-1', [], {}, { ...valid, edits: [{ op: 'future_edit', value: 42 }] }, { ...valid, edits: [{ op: 'move' }] }, { ...valid, edits: {} }, { ...valid, params: { packing: 'future' } }, { ...valid, params: { enhance: null } }, { ...valid, params: { spot_sizes_um: 'bad' } }, { ...valid, params: { um_per_mm: '0x10' } }];
  for (const raw of bad) {
    const before = structuredClone(raw), app = { project: { uuid: 'p1', print_sheets: raw }, readOnly: false, dirty: false };
    assert.equal(readPrintEdits(raw), null);
    assert.deepEqual(keepPrintEdits({ uuid: 'p1' }, app.project).print_sheets, before);
    assert.deepEqual(JSON.parse(JSON.stringify(app.project)).print_sheets, before); assert.equal(app.dirty, false);
    const p = { ...plan(), params: { packing: 'packed' }, um_per_mm: 8 };
    storePrintEdits(app, p, [{ op: 'repack' }]);
    assert.deepEqual(readPrintEdits(app.project.print_sheets).edits, [{ op: 'repack' }]); assert.equal(app.dirty, true);
  }
});
