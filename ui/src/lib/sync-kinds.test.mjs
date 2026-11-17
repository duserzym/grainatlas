import assert from 'node:assert/strict';
import test from 'node:test';
import { changeCounts, diffElements, diffElementKind, elementDiffer, emptyElements } from './sync.ts';

const grain = (id, number) => ({ id, area_id: 'area', number, polygon_um: [[0, 0], [10, 0], [0, 10]], centroid_um: [3, 3], area_um2: 50 });
const spot = (id, name) => ({ id, name, area_id: 'area', grain_id: 'grain', x_um: 2, y_um: 2, diameter_um: 1 });
const fixture = () => ({ grains: [grain('grain', 1), grain('retired', 2)], spots: [spot('spot', 'Synthetic_1'), { ...spot('measured', 'Synthetic_2'), acquisition_ids: ['analysis'] }], labels: [{ id: 'label', area_id: 'area', x_um: 0, y_um: 0, text: 'Synthetic' }] });

test('per-kind edits preserve counts, geometry tolerance, retirements and measured deletions', () => {
  const base = fixture();
  const mine = { grains: [{ ...base.grains[0], polygon_um: [[0, 0], [12, 0], [0, 10]] }, grain('added', 3)], spots: [{ ...base.spots[0], name: 'Synthetic_3' }, spot('new', 'Synthetic_4')], labels: [{ ...base.labels[0], text: 'Edited' }] };
  const split = Object.fromEntries(['spots', 'grains', 'labels'].map(k => [k, diffElementKind(k, base[k], mine[k])]));
  assert.deepEqual(split, diffElements(base, mine));
  // Fixed expectations from the former whole-elements diff, including protected measurements.
  assert.deepEqual(changeCounts(split), { spots: 2, grains: 3, labels: 1 });
  assert.deepEqual(split.spots.delete, []);
  assert.deepEqual(split.grains.retire, ['retired']);
  assert.deepEqual(split.grains.delete, []);
  const rounded = { ...base, spots: base.spots.map(s => ({ ...s, x_um: s.x_um + 1e-8 })) };
  assert.deepEqual(changeCounts(diffElements(base, rounded)), { spots: 0, grains: 0, labels: 0 });
});
test('spot edits and base metadata changes reuse the grains and labels diffs', () => {
  const base = fixture(), mine = structuredClone(base), diff = elementDiffer();
  const grains = diff('grains', base.grains, mine.grains), labels = diff('labels', base.labels, mine.labels);
  const spots = diff('spots', base.spots, mine.spots);
  const next = { ...mine, spots: [...mine.spots, spot('new', 'Synthetic_3')] };
  assert.notEqual(diff('spots', base.spots, next.spots), spots);
  assert.equal(diff('grains', base.grains, next.grains), grains);
  assert.equal(diff('labels', base.labels, next.labels), labels);
  assert.notEqual(diff('grains', [...base.grains], next.grains), grains);
});
test('legacy labels still adopt identities and blank artifacts remain ordinary grains', () => {
  const local = grain('grain', 1), base = { ...emptyElements(), grains: [{ ...local, id: null, label: 'area/1', artifact: '' }] };
  assert.deepEqual(changeCounts(diffElements(base, { ...emptyElements(), grains: [local] })), { spots: 0, grains: 0, labels: 0 });
});
