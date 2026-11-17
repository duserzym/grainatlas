import assert from 'node:assert/strict';
import test from 'node:test';
import { spotSample } from './grain-owners.ts';

const duplicate = { duplicate_of: 'G1' };

test('spot sample follows the clicked grain and only warns for a different owner sample', () => {
  assert.deepEqual(spotSample('S1', 'S2', duplicate, 'S1', 'S2'), { sample: 'S1', conflict: null });
  assert.deepEqual(spotSample('S1', 'S2', duplicate, 'S2', 'S1'), { sample: 'S1', conflict: 'S2' });
  assert.deepEqual(spotSample('', 'S1', duplicate, null, 'S1'), { sample: 'S1', conflict: null });
  assert.deepEqual(spotSample(null, 'S1', duplicate, '', 'S2'), { sample: 'S1', conflict: 'S2' });
  assert.deepEqual(spotSample(undefined, 'S1', null, 'S2', 'S2'), { sample: 'S1', conflict: null });
  assert.deepEqual(spotSample('S1', 'S2', { duplicate_of: null }, 'S2', 'S2'), { sample: 'S1', conflict: null });
});
