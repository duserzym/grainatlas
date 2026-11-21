import test from 'node:test';
import assert from 'node:assert/strict';
import { SizeEditor } from './size-editor.ts';
import { canvasArea, focusSpot, selectionSize } from './spot-size.ts';

test('A: the shared size-box summary uses selected sizes before the next-spot value', () => {
  const selected = [{ id: 'synthetic-one', area_id: 'a', diameter_um: 20 }];
  assert.equal(selectionSize(selected, 25).value, 20);
  assert.equal(selectionSize([], 25).value, 25);
  assert.deepEqual(selectionSize([...selected, { id: 'synthetic-two', area_id: 'a', diameter_um: 40 }], 25),
    { value: null, placeholder: 'mixed', range: '⌀ 20–40 µm', caption: '2 selected' });
  assert.equal(selectionSize([{ ...selected[0], diameter_um: null }], 25).value, null);
});

test('B: a focused size field drops its old edit when the target changes', () => {
  const field = new SizeEditor(); field.sync(20, 'one'); field.focus('one', () => assert.fail('stale callback'));
  field.sync(40, 'two'); assert.equal(field.commit('30', 40, 'two'), null);
});

test('C: requesting an unstitched area never falls back to the previous stitched area', () => {
  const a = { id: 'a', pixel_size_nm: 200, mosaic: { um_per_px: .2 } };
  const b = { id: 'b', pixel_size_nm: 500, mosaic: null };
  const stitched = { a: { mosaic: { um_per_px: .3 } } };
  assert.deepEqual(canvasArea('b', [a, b], stitched), { area: b, mosaic: null, umPerPx: .5 });
  assert.deepEqual(canvasArea('missing', [a, b], stitched), { area: null, mosaic: null, umPerPx: null });
  assert.deepEqual(canvasArea(null, [a, b], stitched), { area: null, mosaic: null, umPerPx: null });
  assert.deepEqual(canvasArea('a', [a, b], stitched), { area: a, mosaic: stitched.a.mosaic, umPerPx: .3 });
  assert.deepEqual(canvasArea('a', [a, b], {}), { area: a, mosaic: a.mosaic, umPerPx: .2 });
  const savedB = { ...b, mosaic: { um_per_px: .6 } };
  assert.deepEqual(canvasArea('b', [a, savedB], stitched), { area: savedB, mosaic: savedB.mosaic, umPerPx: .6 });
  assert.equal(canvasArea('b', [{ id: 'b' }], stitched).umPerPx, null);
  assert.equal(canvasArea('b', [b], { b: { mosaic: { um_per_px: NaN } } }).umPerPx, .5);
});

test('D: the floating panel cannot edit an off-area selection', () => {
  const here = { id: 'synthetic-here', area_id: 'a', diameter_um: 20 };
  const hovered = { id: 'synthetic-hover', area_id: 'a', diameter_um: 30 };
  assert.equal(focusSpot([], ['synthetic-off-area'], [], null), null);
  assert.equal(focusSpot([here], ['synthetic-off-area'], [], 'synthetic-off-area'), null);
  assert.equal(focusSpot([here], ['synthetic-off-area', here.id], [], null), here);
  assert.equal(focusSpot([here, hovered], [here.id], [], hovered.id), here);
  assert.equal(focusSpot([here, hovered], [], [], hovered.id), hovered);
  assert.equal(focusSpot([here, hovered], [], ['synthetic-grain'], hovered.id), null);
  assert.equal(focusSpot([here, hovered], [here.id], ['synthetic-grain'], hovered.id), here);
});
