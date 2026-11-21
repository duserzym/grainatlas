// Invented ids and coordinates only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { VIEWER_REGION_MAX_SPOTS } from '@lab/bridge';
import { mountName, sampleNames, sampleText, shortMountName } from './mount-inset-header.ts';
import { buildViewerRegion, sendViewerRegion } from './viewer-region.ts';
import { regionBounds, regionFitRect, regionCamera } from './viewer-region-fit.ts';
import { groupRegions, mountLayout } from './mount-inset.ts';
import { toScreen } from './canvas/view.ts';

const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-7, `${a} ≈ ${b}`);
const input = (kind = 'area') => ({ kind, project: 'project-1', mount: 'mount-1', mountLabel: 'Invented mount',
  area: kind === 'mount' ? null : 'area-1', areaLabel: kind === 'mount' ? null : 'Area One',
  sample: kind === 'sample' ? 'Sample A' : null, spots: ['spot-2', 'spot-1', 'spot-2'] });
const view = { cx: 0, cy: 0, zoom: 0.1, rot: 0, w: 1200, h: 600 };

test('mount label prefers hosted status, then published name, then project, then Mount', () => {
  assert.equal(mountName('Mount One', 'Published', 'Project'), 'Mount One');
  assert.equal(mountName(null, 'Published', 'Project'), 'Published');
  assert.equal(mountName('', '', 'Project'), 'Project');
  assert.equal(mountName('  ', null, ''), 'Mount');
  assert.equal(mountName(' Mount Two '), 'Mount Two');
});
test('sample names prefer groups, otherwise unique area labels in mount order', () => {
  const areas = [{ id: 'a', sample: 'A', name: 'Area A' }, { id: 'b', sample: '', name: 'B' }, { id: 'c', sample: 'A', name: 'C' }];
  assert.deepEqual(sampleNames([], areas), ['A', 'B']);
  assert.deepEqual(sampleNames([{ name: 'Second' }, { name: 'First' }], areas), ['Second', 'First']);
});
test('sample header shows fitting names or the count, keeping the full title', () => {
  assert.deepEqual(sampleText(['A', 'B'], 'Area', 24), { text: 'A, B', title: 'A, B' });
  assert.deepEqual(sampleText(['SampleA', 'SampleB'], 'Area', 54), { text: '2 samples', title: 'SampleA, SampleB' });
  assert.equal(sampleText(['Long sample name'], 'Area', 48).text, '1 sample');
  assert.deepEqual(sampleText([], 'Current area', 72), { text: 'Current area', title: 'Current area' });
  assert.equal(sampleText(['Wide'], '', 10, () => 9).text, 'Wide');
});
test('sample caption stays empty when even the count or fallback cannot fit, retaining its title', () => {
  for (const available of [0, 1, 5, 53]) {
    assert.deepEqual(sampleText(['SampleA', 'SampleB'], 'Area', available), { text: '', title: 'SampleA, SampleB' });
  }
  assert.deepEqual(sampleText([], 'Current area', 1), { text: '', title: 'Current area' });
  const measure = text => text === '2 samples' ? 40 : 100;
  assert.equal(sampleText(['A', 'B'], '', 39, measure).text, '');
  assert.equal(sampleText(['A', 'B'], '', 40, measure).text, '2 samples');
});
test('collapsed mount label is limited to 22 characters', () => {
  assert.equal(shortMountName('Mount A'), 'Mount A');
  assert.equal(Array.from(shortMountName('A'.repeat(60))).length, 22);
  assert.ok(shortMountName('A'.repeat(60)).endsWith('…'));
});
test('region bounds use only finite spots from the clicked area and matching Review ids', () => {
  const spots = [
    { id: 's1', area_id: 'a', x_um: 10, y_um: -20 }, { id: 's2', area_id: 'a', x_um: 610, y_um: 40 },
    { id: 's3', area_id: 'b', x_um: 9000, y_um: 9000 }, { id: 's4', area_id: 'a', x_um: NaN, y_um: 0 },
  ];
  assert.deepEqual(regionBounds(spots, ['S1', 's2', 's3', 's4', 'missing'], 'a'), [10, -20, 610, 40]);
  assert.equal(regionBounds(spots, ['missing'], 'a'), null);
});
test('fit rectangle pads each edge 15 percent and keeps a minimum 300 µm width', () => {
  assert.deepEqual(regionFitRect([0, 0, 1000, 600]), [-150, -90, 1150, 690]);
  assert.deepEqual(regionFitRect([20, 30, 20, 30]), [-130, -120, 170, 180]);
  assert.deepEqual(regionFitRect([0, 0, 10, 1000]), [-145, -150, 155, 1150]);
});
test('region camera may zoom out for a large region and caps zoom at the default centerOnUm', () => {
  const small = regionCamera(regionFitRect([20, 30, 20, 30]), view, 2);
  assert.deepEqual(small, { x: 10, y: 15, zoom: 2 });
  const large = regionCamera([-5000, -100, 5000, 100], view, 1);
  near(large.zoom, 0.12);
  const zoomed = regionCamera(regionFitRect([0, 0, 0, 0]), { ...view, zoom: 20 }, 1);
  near(zoomed.zoom, 2); // Fit can zoom out from an already close view.
});
test('fit accounts for non-square viewports, image orientation and rotation', () => {
  const v = { ...view, w: 320, h: 240, rot: Math.PI / 3, image: [0, 1, -1, 0, 600, 0] };
  const rect = regionFitRect([0, 0, 1000, 50]);
  const next = regionCamera(rect, v, 1);
  // centerOn applies the image transform to the untransformed centre.
  const fitted = { ...v, cx: 600 - next.y, cy: next.x, zoom: next.zoom };
  for (const [x, y] of [[rect[0], rect[1]], [rect[2], rect[1]], [rect[2], rect[3]], [rect[0], rect[3]]]) {
    const [sx, sy] = toScreen(fitted, x, y);
    assert.ok(sx >= -1e-7 && sx <= v.w + 1e-7 && sy >= -1e-7 && sy <= v.h + 1e-7);
  }
});
test('sample geometry retains whole-mount Review ids while each fit stays in one area', () => {
  const areas = ['a', 'b'].map(id => ({ id, name: id, sample: '', tiles: [], pixel_size_nm: 1000, stage_sign: [1, 1], placement: null }));
  const spots = [{ id: 's1', area_id: 'a', x_um: 0, y_um: 0 }, { id: 's2', area_id: 'b', x_um: 100, y_um: 200 }];
  const [g] = groupRegions([{ name: 'Sample A', color: '#123456', spots: ['s2', 's1'] }], spots, areas, mountLayout(areas));
  assert.deepEqual(g.spots, ['s2', 's1']);
  assert.equal(g.regions.length, 2);
  assert.deepEqual(regionBounds(spots, g.spots, 'a'), [0, 0, 0, 0]);
});
test('region events carry all three kinds and mount events always have empty spots', () => {
  for (const kind of ['mount', 'area', 'sample']) {
    const event = buildViewerRegion(input(kind));
    assert.equal(event.type, 'lab.viewer.region'); assert.equal(event.kind, kind);
    assert.deepEqual(event.spots, kind === 'mount' ? [] : ['spot-2', 'spot-1']);
    assert.equal(event.area, kind === 'mount' ? null : 'area-1');
    assert.equal(event.sample, kind === 'sample' ? 'Sample A' : null);
  }
  assert.equal(buildViewerRegion({ ...input(), mount: null }).mount, null);
});
test('event ids dedupe case-insensitively, preserving first spelling and stable order', () => {
  assert.deepEqual(buildViewerRegion({ ...input('sample'), spots: ['S2', 's1', 's2', 's1'] }).spots, ['S2', 's1']);
});
test('large lists cap after dedupe at VIEWER_REGION_MAX_SPOTS', () => {
  const ids = Array.from({ length: VIEWER_REGION_MAX_SPOTS + 1000 }, (_, i) => `s${i}`);
  const event = buildViewerRegion({ ...input(), spots: ['s0', ...ids] });
  assert.deepEqual(event.spots, ids.slice(0, VIEWER_REGION_MAX_SPOTS));
});
test('bridge validation rejects incomplete region events', () => {
  assert.equal(buildViewerRegion({ ...input('sample'), sample: null }), null);
  assert.equal(buildViewerRegion({ ...input(), area: null }), null);
  assert.equal(buildViewerRegion({ ...input(), spots: [''] }), null);
});
test('send only from a hosted viewer; malformed events never send', () => {
  const sent = [], send = e => sent.push(e);
  assert.equal(sendViewerRegion(input(), { current: 'editor' }, true, send), false);
  assert.equal(sendViewerRegion(input(), { current: 'viewer' }, false, send), false);
  assert.equal(sendViewerRegion({ ...input(), project: '' }, { current: 'viewer' }, true, send), false);
  assert.equal(sent.length, 0);
  assert.equal(sendViewerRegion(input('sample'), { current: 'viewer' }, true, send), true);
  assert.equal(sent.length, 1); assert.equal(sent[0].kind, 'sample');
});
