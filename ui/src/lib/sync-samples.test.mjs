import assert from 'node:assert/strict';
import test from 'node:test';
import { uiSpots } from './sync.ts';

const spot = { id: 'P1', area_id: 'A1', grain_id: 'G1', name: 'S1_1' };
const grain = { id: 'G1', area_id: 'A1', number: 2, sample: 'S1' };

test('server grain sample wins over a stale local spot sample', () => {
  const [result] = uiSpots([spot], [grain], [{ ...spot, sample: 'S2', n: 3, comment: 'C1' }], { A1: 'S3' });
  assert.equal(result.sample, 'S1');
  assert.equal(result.grain_number, 2);
  assert.equal(result.n, 3);
  assert.equal(result.comment, 'C1');
});

test('without a grain sample keep the local sample, then fall back to the area', () => {
  for (const sample of [undefined, null, '']) {
    const grains = [{ ...grain, sample }];
    assert.equal(uiSpots([spot], grains, [{ ...spot, sample: 'S2' }], { A1: 'S3' })[0].sample, 'S2');
    assert.equal(uiSpots([spot], grains, [], { A1: 'S3' })[0].sample, 'S3');
  }
  assert.equal(uiSpots([spot], [], [], { A1: 'S3' })[0].sample, 'S3');
  assert.equal(uiSpots([spot], [], [ { ...spot, sample: '' } ], { A1: 'S3' })[0].sample, '');
  assert.equal(uiSpots([{ ...spot, grain_id: null }], [{ ...grain, id: null, sample: 'S2' }], [], { A1: 'S3' })[0].sample, 'S3');
});

test('large server lists resolve every sample without changing input records', () => {
  const grains = Array.from({ length: 5000 }, (_, i) => ({ ...grain, id: `G${i}`, sample: i % 2 ? 'S1' : null }));
  const spots = grains.map((g, i) => ({ ...spot, id: `P${i}`, grain_id: g.id }));
  const previous = spots.map(s => ({ ...s, sample: 'S2' }));
  const result = uiSpots(spots, grains, previous, { A1: 'S3' });
  assert.equal(result.length, 5000);
  result.forEach((s, i) => assert.equal(s.sample, i % 2 ? 'S1' : 'S2'));
  assert.ok(previous.every(s => s.sample === 'S2'));
});
