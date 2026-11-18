import assert from 'node:assert/strict';
import test from 'node:test';
import { staleOpenElements, automaticSpotRemovals } from './stale-open.ts';

const base = { spots: [{ id: 'spot-one' }, { id: 'spot-two' }],
  grains: [{ id: 'grain-one' }, { id: 'old-grain', retired: true }, { id: null }],
  labels: [{ id: 'label-one' }, { id: 'old-label', retired: true }] };
const local = { spots: [{ id: 'spot-one' }], grains: [], labels: [] };

test('no removals or no registered base means no stale opening', () => {
  assert.equal(staleOpenElements(null, local), null);
  assert.equal(staleOpenElements(base, base), null);
  assert.equal(staleOpenElements({ spots: [], grains: [], labels: [] }, local), null);
});
test('opening removals return missing active registered ids in every kind', () => {
  assert.deepEqual(staleOpenElements(base, local), { spots: ['spot-two'], grains: ['grain-one'], labels: ['label-one'] });
  assert.deepEqual(staleOpenElements(base, { spots: [], grains: [], labels: [] }).spots, ['spot-one', 'spot-two']);
});
test('deletions after a user edit are not a stale opening', () => {
  assert.equal(staleOpenElements(base, local, true), null);
});
test('automatic deletion guard counts only distinct base spots and permits at most five', () => {
  const base = { spots: Array.from({ length: 8 }, (_, n) => ({ id: `spot-${n}` })), grains: [], labels: [] };
  assert.deepEqual(automaticSpotRemovals(base, ['spot-0', 'spot-1', 'spot-2', 'spot-3', 'spot-4', 'new-spot']), []);
  const ids = base.spots.slice(0, 6).map(s => s.id);
  assert.deepEqual(automaticSpotRemovals(base, [...ids, ids[0], 'new-spot']), ids);
});
