import assert from 'node:assert/strict';
import test from 'node:test';
import { PrintLRU, printFingerprint } from './print-cache.ts';

test('fingerprint tracks immutable data and every small output input', () => {
  const input = { identity: ['tab', 'project', 'revision'], controls: { style: 'sheets', dpi: 300 },
    areas: ['a', 'mosaic-hash'], tone: { gamma: 1 }, grains: { a: [{ id: 'g', polygon: [] }] },
    spots: [], labels: [], edits: { edits: [] } };
  const key = printFingerprint(input);
  assert.equal(printFingerprint({ ...input }), key);
  for (const name of ['identity', 'controls', 'areas', 'tone', 'grains', 'spots', 'labels', 'edits']) {
    const next = { ...input, [name]: name === 'grains' ? { a: [...input.grains.a] } : name === 'spots' || name === 'labels' ? [] : { changed: true } };
    assert.notEqual(printFingerprint(next), key, name);
  }
  const poisonous = { toJSON() { throw new Error('must not traverse grains'); } };
  assert.doesNotThrow(() => printFingerprint({ ...input, grains: { a: poisonous } }));
  assert.equal(printFingerprint({ ...input, grains: { b: input.grains.a, a: input.grains.a } }),
    printFingerprint({ ...input, grains: { a: input.grains.a, b: input.grains.a } }));
});

test('LRU enforces count and byte bounds and transfers ownership on take', () => {
  const disposed = [];
  const cache = new PrintLRU(2, 10, value => disposed.push(value));
  cache.put('a', 1, 'a', 4); cache.put('b', 2, 'b', 4);
  const a = cache.take('a'); cache.put('a', 1, a, 4);
  cache.put('c', 1, 'c', 4);
  assert.deepEqual(disposed, ['b']);
  cache.put('d', 1, 'd', 8);
  assert.deepEqual(disposed, ['b', 'a', 'c']);
  assert.equal(cache.bytes, 8);
  assert.equal(cache.put('huge', 1, 'huge', 11), false);
  assert.equal(disposed.at(-1), 'huge');
  assert.equal(cache.take('d'), 'd'); assert.equal(cache.bytes, 0);
  cache.put('e', 1, 'e', 2); cache.put('f', 2, 'f', 2);
  cache.clearOwner(1, true);
  assert.equal(cache.take('e'), undefined);
  assert.equal(cache.put('late', 1, 'late', 1), false);
  assert.equal(cache.take('f'), 'f');
});

test('nested tone, control and saved-strip mutations invalidate without scanning geometry', () => {
  const input = { identity: { tab: 1, project: 'p', revision: 1 }, controls: { whole: { orientation: 'auto' }, sheet: { enhance: { denoise: true } } },
    areas: { a: { sha256: 'pixels', sample: 'A' } }, tone: { a: { gamma: 1 } }, grains: { a: [] }, edits: { edits: [] } };
  let previous = printFingerprint(input);
  for (const change of [() => input.tone.a.gamma++, () => input.controls.whole.orientation = 'portrait',
    () => input.controls.sheet.enhance.denoise = false, () => input.edits.edits.push({ op: 'repack' }),
    () => input.areas.a.sha256 = 'new-pixels', () => input.identity.revision++, () => input.identity.project = 'other']) {
    change(); const key = printFingerprint(input); assert.notEqual(key, previous); previous = key;
  }
});

test('replacement revokes the old entry and closing one project leaves others intact', () => {
  const disposed = [];
  const cache = new PrintLRU(2, 100, value => disposed.push(value));
  cache.put('one', 1, 'old', 10); cache.put('one', 1, 'new', 20); cache.put('two', 2, 'other', 30);
  assert.deepEqual(disposed, ['old']); assert.equal(cache.bytes, 50);
  cache.clearOwner(1, true);
  assert.deepEqual(disposed, ['old', 'new']); assert.equal(cache.bytes, 30);
  assert.equal(cache.take('two'), 'other'); assert.equal(cache.bytes, 0);
});

test('a finishing full-quality page reweighs only its still-cached result', () => {
  const disposed = [];
  const cache = new PrintLRU(2, 100, value => disposed.push(value));
  const a = {}, b = {}, replaced = {};
  cache.put('a', 1, a, 20); cache.put('b', 2, b, 20);
  cache.reweigh('b', b, 90);
  assert.deepEqual(disposed, [a]); assert.equal(cache.bytes, 90);
  cache.take('b'); cache.reweigh('b', b, 200);
  assert.equal(cache.bytes, 0); assert.deepEqual(disposed, [a]);
  cache.put('b', 2, replaced, 10); cache.reweigh('b', b, 200);
  assert.equal(cache.bytes, 10);
  cache.reweigh('b', replaced, 101);
  assert.equal(cache.bytes, 0); assert.deepEqual(disposed, [a, replaced]);
});
