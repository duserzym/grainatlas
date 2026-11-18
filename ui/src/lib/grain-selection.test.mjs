// Seeded, invented grains only. No images or project fixtures.
import assert from 'node:assert/strict';
import test from 'node:test';
import { canvasBox, grainGrid } from './grain-selection.ts';
import { matrix, toScreen } from './canvas/view.ts';

const grain = (id, x, y, rx = 5, ry = 5) => Object.freeze({ id, centroid_um: Object.freeze([x, y]),
  bbox_um: Object.freeze([x - rx, y - ry, x + rx, y + ry]) });
const view = { cx: 0, cy: 0, w: 1440, h: 900, zoom: 1, rot: 0 };
const ids = gs => gs.map(g => g.id);
const rect = (x0, y0, x1, y1) => ({ bounds: [x0, y0, x1, y1], contains: (x, y) => x >= x0 && x <= x1 && y >= y0 && y <= y1 });

test('uniform grid matches a brute-force filter on 5000 random grains and rotated boxes', () => {
  let seed = 123456;
  const random = () => ((seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32);
  const grains = Object.freeze(Array.from({ length: 5000 }, (_, i) =>
    grain(`grain-${i}`, random() * 12000 - 6000, random() * 8000 - 4000, 1 + random() * 200, 1 + random() * 200)));
  const grid = grainGrid(grains);
  for (let i = 0; i < 150; i++) {
    const v = { ...view, cx: random() * 2000 - 1000, cy: random() * 2000 - 1000,
      zoom: 0.05 + random(), rot: (random() - .5) * Math.PI * 2 };
    const start = [random() * v.w, random() * v.h], end = [random() * v.w, random() * v.h];
    const umPerPx = 2;
    // Independent oracle: the old Canvas rule tests screen-space centroids.
    const expected = grains.filter(g => {
      const [x, y] = toScreen(v, ...g.centroid_um.map(n => n / umPerPx));
      return x >= Math.min(start[0], end[0]) && x <= Math.max(start[0], end[0])
        && y >= Math.min(start[1], end[1]) && y <= Math.max(start[1], end[1]);
    });
    assert.deepEqual(ids(grid.query(canvasBox(v, umPerPx, start, end))), ids(expected), `query ${i}`);
  }
});

test('grid deduplicates bounding boxes spanning cells and tests centroids, with inclusive edges', () => {
  const grains = Object.freeze([grain('wide', 0, 0, 1000, 1000), grain('edge', 10, 20), grain('outside', 21, 20, 100)]);
  assert.deepEqual(ids(grainGrid(grains).query(rect(0, 0, 20, 20))), ['wide', 'edge']);
  assert.deepEqual(grainGrid(grains).query(rect(2000, 2000, 3000, 3000)), []);
  assert.deepEqual(grainGrid(grains).query(rect(-3000, -3000, -2000, -2000)), []);
  assert.deepEqual(grainGrid([]).query(rect(-10, -10, 10, 10)), []);
  assert.deepEqual(ids(grainGrid([grain('point', 0, 0, 0, 0)]).query(rect(0, 0, 0, 0))), ['point']);
});

test('grid cache reuses array identity and rebuilds for edited or restored arrays', () => {
  const grains = Object.freeze([grain('a', 0, 0)]), original = grainGrid(grains);
  assert.equal(grainGrid(grains), original);
  const edited = Object.freeze([grain('b', 0, 0)]);
  assert.notEqual(grainGrid(edited), original);
  assert.deepEqual(ids(grainGrid(edited).query(rect(-1, -1, 1, 1))), ['b']);
  assert.deepEqual(ids(grainGrid(grains).query(rect(-1, -1, 1, 1))), ['a']);
});

test('screen box handles reversed drags, rotation, boundaries and zero-width boxes', () => {
  const v = { ...view, zoom: 2, rot: Math.PI / 4 }, scale = 2;
  const box = canvasBox(v, scale, [900, 600], [700, 400]);
  for (const screen of [[700, 400], [900, 600], [800, 500]]) {
    // Inverse matrix computed independently of canvasBox.
    const dx = screen[0] - v.w / 2, dy = screen[1] - v.h / 2;
    assert.ok(box.contains((dx + dy) / Math.SQRT2, (-dx + dy) / Math.SQRT2));
  }
  assert.equal(box.contains(10000, 10000), false);
  const line = canvasBox(view, 1, [720, 450], [720, 500]);
  assert.ok(line.contains(0, 25));
  assert.equal(line.contains(1, 25), false);
});

test('a box starting on an SVG-rendered centroid includes its edge despite transform precision', () => {
  const v = { cx: 3687, cy: 895, w: 1440, h: 800, zoom: 1440 / 7374 * .94, rot: 0 };
  const umPerPx = 2, centre = [2949.6, 447.5];
  // Model the SVG renderer's float transform independently of the inverse in canvasBox.
  const [a, b, c, d, e, f] = matrix(v).map(Math.fround);
  const start = [a * centre[0] / umPerPx + c * centre[1] / umPerPx + e,
    b * centre[0] / umPerPx + d * centre[1] / umPerPx + f];
  const end = [start[0] + 300, start[1] + 150];
  const grains = [grain('start', ...centre), grain('same-column', centre[0], centre[1] + 100),
    grain('outside', centre[0] - .001 * umPerPx / v.zoom, centre[1] + 100)];
  for (const [from, to] of [[start, end], [end, start]]) {
    assert.deepEqual(ids(grainGrid(grains).query(canvasBox(v, umPerPx, from, to))), ['start', 'same-column']);
  }
});
