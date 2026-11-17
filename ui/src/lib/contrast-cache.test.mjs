import assert from 'node:assert/strict';
import test from 'node:test';
import { RingContrasts } from './canvas/contrast.ts';

test('contrast reads only new or moved/resized rings, preserving hysteresis', () => {
  const cache = new RingContrasts(), reads = [];
  let luma = 0.7;
  const sample = (...geometry) => { reads.push(geometry); return luma; };
  const a = { id: 'a', x: 10, y: 20, r: 5 }, b = { id: 'b', x: 30, y: 20, r: 5 };
  assert.deepEqual(cache.update([a], 0, sample), { a: 'dark' });
  cache.update([{ ...a }], 0, sample);
  assert.equal(reads.length, 1);
  cache.update([a, b], 0, sample);
  assert.equal(reads.length, 2);
  luma = 0.5;
  assert.deepEqual(cache.update([{ ...a, x: 11 }, b], 0, sample), { a: 'dark', b: 'dark' });
  assert.equal(reads.length, 3);
  cache.update([{ ...a, x: 11, r: 6 }, b], 0, sample);
  assert.equal(reads.length, 4);
  luma = 0.2;
  assert.deepEqual(cache.update([a, b], 1, sample), { a: 'light', b: 'light' });
  assert.equal(reads.length, 6, 'a new pixel scene resamples all visible rings');
  cache.update([], 1, sample);
  luma = 0.5;
  assert.deepEqual(cache.update([a], 1, sample), { a: 'light' });
});
