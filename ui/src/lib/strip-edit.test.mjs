import assert from 'node:assert/strict';
import test from 'node:test';
import { bandImageKey, bandRect, bodyOrigin, cropRect, firstFreePlace, grainChanges, keyId, nudgeOrigin, snapGap, snapOrigin, splitPairs, stripBlock, StripHistory, targetGrains } from './strip-edit.ts';

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
