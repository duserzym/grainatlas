// Invented coordinates only. Run with Node's built-in TypeScript support:
// node --test ui/src/lib/mount-inset.test.mjs
import assert from 'node:assert/strict';
import test from 'node:test';
import { reviewGroups } from './shellcmd.ts';
import { groupRegions, mosaicToMount, mountLayout } from './mount-inset.ts';

const group = (spots, name = 'Sample A') => ({ name, color: '#ab23cd', spots });
const spot = (id, area_id, x_um, y_um) => ({ id, area_id, x_um, y_um });
const area = (id, stage = null, w = 1000, h = 600) => ({
  id, name: id, sample: '', pixel_size_nm: 1000, stage_sign: [-1, 1], placement: null,
  tiles: [{ w, h, pixel_size_nm: 1000, stage_nm: stage, pos_um: [0, 0] }],
});
const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-7, `${a} ≈ ${b}`);
const extent = (points, axis) => [Math.min(...points.map(p => p[axis])), Math.max(...points.map(p => p[axis]))];
const contains = (hull, p) => hull.every((a, i) => {
  const b = hull[(i + 1) % hull.length];
  return (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-7;
});

test('reviewGroups treats absent or malformed envelopes as empty', () => {
  for (const value of [undefined, null, [], 'groups', 3]) assert.deepEqual(reviewGroups(value), {});
  assert.deepEqual(reviewGroups({ 'bad project': [group(['s'])], p: {}, q: null }), {});
});

test('reviewGroups retains valid neighbours, normalises colour and copies ids', () => {
  const valid = { name: 'A < B', color: '#AB23CD', spots: ['s-1', 's-2'] };
  const result = reviewGroups({ p: [null, valid, group([], 'Empty')] });
  assert.deepEqual(result, { p: [{ ...valid, color: '#ab23cd' }, group([], 'Empty')] });
  valid.spots.push('s-3');
  assert.deepEqual(result.p[0].spots, ['s-1', 's-2']);
  // These keys are own data properties, never prototype lookups.
  const special = reviewGroups(JSON.parse('{"__proto__":[{"name":"A","color":"#123456","spots":[]}]}'));
  assert.equal(Object.hasOwn(special, '__proto__'), false); // project ids must start with a letter or digit
});

test('reviewGroups rejects malformed groups and enforces inclusive limits', () => {
  const invalid = [
    { ...group([]), name: 4 }, group([], 'x'.repeat(201)),
    ...['red', '#abc', '#12345678', '#gg0000', null].map(color => ({ ...group([]), color })),
    { ...group([]), spots: 's' }, group([5]), group([null]), group(['bad id']), group(Array(1)),
    group(Array(10001).fill('s')),
  ];
  assert.deepEqual(reviewGroups({ p: invalid }), { p: [] });
  assert.equal(reviewGroups({ p: Array(257).fill(group([])) }).p, undefined);
  assert.equal(reviewGroups({ p: Array(256).fill(group([])) }).p.length, 256);
  const valid = reviewGroups({ p: [group(Array(10000).fill('s'), 'x'.repeat(200))] }).p[0];
  assert.equal(valid.spots.length, 10000);
  assert.equal(valid.name.length, 200);
  assert.equal(Object.keys(reviewGroups(Object.fromEntries(Array.from({ length: 65 }, (_, i) => [`p${i}`, []])))).length, 64);
  assert.deepEqual(reviewGroups(undefined), {}); // next command without groups replaces the whole value
});

test('one point gives a padded circle; two points give a capsule in the row fallback', () => {
  const areas = [area('a'), area('b')], layout = mountLayout(areas);
  assert.equal(layout.positioned, false);
  const groups = [group(['s1']), group(['s2', 's3'], 'Sample B')];
  const spots = [spot('s1', 'a', 10, 20), spot('s2', 'b', 50, 60), spot('s3', 'b', 150, 60)];
  const snapshot = structuredClone({ groups, spots, areas, layout });
  const result = groupRegions(groups, spots, areas, layout);
  const pad = Math.max(40, .04 * (layout.bounds[2] - layout.bounds[0]));
  const circle = result[0].regions[0];
  assert.equal(circle.hull.length, 24);
  for (const p of circle.hull) near(Math.hypot(p[0] - 10, p[1] - 20), pad);
  near(result[0].anchor[0], 10); near(result[0].anchor[1], 20);
  const capsule = result[1].regions[0], x = layout.areas[1].box[0];
  assert.deepEqual(capsule.points, [[x + 50, 60], [x + 150, 60]]);
  const [min, max] = extent(capsule.hull, 0);
  near(min, x + 50 - pad); near(max, x + 150 + pad);
  near(result[1].anchor[0], x + 100); near(result[1].anchor[1], 60);
  assert.deepEqual({ groups, spots, areas, layout }, snapshot);
});

test('stage positions and solved placements use mosaicToMount; padding has a 40 µm floor', () => {
  const a = area('a', [-1000000, 2000000], 100, 100);
  a.placement = { width: 100, height: 100, tiles: [{ pos: [25, 35], w: 100, h: 100 }] };
  const layout = mountLayout([a]);
  assert.equal(layout.positioned, true);
  const [g] = groupRegions([group(['s'])], [spot('s', 'a', 75, 85)], [a], layout);
  assert.deepEqual(g.regions[0].points, [[1000, 2000]]);
  assert.deepEqual(g.regions[0].points[0], mosaicToMount(a, layout, 75, 85));
  const [lo, hi] = extent(g.regions[0].hull, 0);
  near(lo, 960); near(hi, 1040);
});

test('groups split at area boundaries and label the largest polygon by area, not spot count', () => {
  const areas = [area('a'), area('b')], layout = mountLayout(areas);
  const spots = [spot('a1', 'a', 10, 10), spot('a2', 'a', 11, 11), spot('a3', 'a', 10, 11),
    spot('b1', 'b', 100, 100), spot('b2', 'b', 500, 100)];
  const [g] = groupRegions([group(spots.map(s => s.id))], spots, areas, layout);
  assert.equal(g.regions.length, 2);
  assert.equal(g.anchorArea, 'b');
  near(g.anchor[0], layout.areas[1].box[0] + 300); near(g.anchor[1], 100);
  for (const r of g.regions) {
    assert.ok(r.points.every(p => contains(r.hull, p)));
    assert.ok(r.hull.flat().every(Number.isFinite));
  }
});

test('missing, nonfinite and unmappable spots are skipped; ids match focus case rules', () => {
  const areas = [area('a')], layout = mountLayout(areas);
  const spots = [spot('s', 'a', 20, 30), spot('null', 'a', null, 1), spot('none', 'a', 1, undefined),
    spot('nan', 'a', NaN, 1), spot('inf', 'a', 1, Infinity), spot('unknown', 'absent', 1, 1)];
  const [g] = groupRegions([group(['S', 's', 'missing', ...spots.slice(1).map(s => s.id)]), group(['missing'])], spots, areas, layout);
  assert.deepEqual(g.regions[0].points, [[20, 30]]);
  assert.deepEqual(groupRegions([group(['s'])], spots, areas, { ...layout, areas: [] }), []);
  assert.deepEqual(groupRegions([], spots, areas, layout), []);
  assert.deepEqual(groupRegions([group([])], [], [], mountLayout([])), []);
  assert.deepEqual(groupRegions([group(['s'])], spots, areas, { ...layout, positioned: true }), []);
});

test('convex padding handles coincident, collinear and unordered points', () => {
  const areas = [area('a')], layout = mountLayout(areas);
  for (const points of [[[0, 0], [0, 0]], [[0, 0], [50, 50], [100, 100]],
    [[100, 100], [0, 0], [0, 100], [50, 50], [100, 0]]]) {
    const spots = points.map(([x, y], i) => spot(`s${i}`, 'a', x, y));
    const [g] = groupRegions([group(spots.map(s => s.id))], spots, areas, layout);
    const hull = g.regions[0].hull;
    assert.ok(points.every(p => contains(hull, p)));
    assert.ok(contains(hull, g.anchor));
    assert.ok(hull.flat().every(Number.isFinite));
    assert.ok(g.anchor.every(Number.isFinite));
    assert.equal(new Set(hull.map(p => p.join(','))).size, hull.length);
  }
});
