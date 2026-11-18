// Invented ids only; selection helpers never mutate the previous selection.
import assert from 'node:assert/strict';
import test from 'node:test';
import { addMany, toggleMany, clickHow, pick } from './selection.ts';

test('box add preserves selection order, removes duplicates and updates the anchor', () => {
  const sel = Object.freeze({ ids: Object.freeze(['a', 'b']), anchor: 'a' });
  assert.deepEqual(addMany(sel, ['b', 'c', 'c', 'd']), { ids: ['a', 'b', 'c', 'd'], anchor: 'd' });
  assert.deepEqual(addMany(sel, []), sel);
  assert.deepEqual(sel.ids, ['a', 'b']);
});

test('box toggle removes selected hits and adds unselected hits exactly once', () => {
  const sel = Object.freeze({ ids: Object.freeze(['a', 'b', 'e']), anchor: 'a' });
  const result = toggleMany(sel, ['b', 'c', 'c', 'd']);
  assert.deepEqual(result, { ids: ['a', 'e', 'c', 'd'], anchor: 'd' });
  assert.deepEqual(toggleMany(sel, []), sel);
  assert.deepEqual(toggleMany(result, ['c', 'd']), { ids: ['a', 'e'], anchor: 'd' });
  assert.deepEqual(sel.ids, ['a', 'b', 'e']);
});

test('modifier clicks retain add, table range and Cmd/Ctrl toggle behaviour', () => {
  const shift = { shiftKey: true, metaKey: false, ctrlKey: false };
  assert.equal(clickHow(shift, false), 'add');
  assert.equal(clickHow(shift, true), 'range');
  for (const mod of ['metaKey', 'ctrlKey']) assert.equal(clickHow({ ...shift, [mod]: true }, false), 'toggle');
  assert.deepEqual(pick(['a', 'b', 'c'], { ids: ['a'], anchor: 'a' }, 'c', clickHow(shift, false)),
    { ids: ['a', 'c'], anchor: 'c' });
});
