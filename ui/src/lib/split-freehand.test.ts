import { describe, expect, it } from 'vitest';
import { appendPathPoint, extendPath, simplifyPath, type Point } from './split-path';
import { contains, grainForCut, polygonGrain, splitPolygon, type Polygon } from './polygon-edit';
import { reassignLineage, splitChildren } from './grain-lineage';
import type { Grain, Spot } from './core/types';

const square: Polygon = [[0, 0], [100, 0], [100, 100], [0, 100]];
const area = (p: Polygon) => polygonGrain(p).area_um2;
const parent = (): Grain => ({ ...polygonGrain(square), id: 'parent', number: 7, label: 'A/7', sample: 'invented' });

describe('split stroke helpers', () => {
  it('drops samples less than a screen pixel apart in micrometres', () => {
    const path: Point[] = [[0, 0]];
    expect(appendPathPoint(path, [.6, .7], 1)).toBe(false);
    expect(appendPathPoint(path, [0, 1], 1)).toBe(true);
    expect(appendPathPoint(path, [.2, 1.1], 1)).toBe(false);
    expect(path).toEqual([[0, 0], [0, 1]]);
  });
  it('RDP preserves bends and endpoints and removes points within tolerance', () => {
    const path: Point[] = [[0, 0], [1, .1], [2, 0], [3, 3], [4, 3.1], [5, 3]];
    expect(simplifyPath(path, .5)).toEqual([[0, 0], [2, 0], [3, 3], [5, 3]]);
    expect(simplifyPath([[0, 0], [1, 0], [2, 0]], .5)).toEqual([[0, 0], [2, 0]]);
    expect(simplifyPath([], .5)).toEqual([]);
    expect(simplifyPath([[1, 1], [1, 1], [1, 1]], .5)).toEqual([[1, 1], [1, 1]]);
    expect(path).toHaveLength(6);
  });
  it('RDP handles many samples without recursive stack growth', () => {
    const path: Point[] = Array.from({ length: 20_000 }, (_, i) => [i, Math.sin(i / 1000)]);
    const simple = simplifyPath(path, .5);
    expect(simple[0]).toEqual(path[0]);
    expect(simple.at(-1)).toEqual(path.at(-1));
    expect(simple.length).toBeLessThan(30);
  });
  it('extends open ends along their own segments and removes duplicate samples', () => {
    const path = extendPath([[30, 30], [30, 30], [50, 50], [70, 30]], [0, 0, 100, 100]);
    expect(path[0][0]).toBeLessThan(0);
    expect(path[0][1]).toBeLessThan(0);
    expect(path.at(-1)![0]).toBeGreaterThan(99);
    expect(path.at(-1)![1]).toBeLessThan(0);
    expect(() => extendPath([[1, 1]], [0, 0, 100, 100])).toThrow('two finite points');
  });
});

describe('polyline polygon splitting', () => {
  it('retains an S curve and conserves area for both ring orientations', () => {
    const path: Point[] = [[-10, 50], [25, 30], [75, 70], [110, 50]];
    for (const polygon of [square, [...square].reverse()]) {
      const parts = splitPolygon(polygon, path);
      expect(parts).toHaveLength(2);
      expect(parts.reduce((s, p) => s + area(p), 0)).toBeCloseTo(10_000);
      const upper = parts.find(p => contains(p, [50, 10]))!;
      expect(contains(upper, [25, 45])).toBe(false);
      expect(contains(upper, [75, 55])).toBe(true);
    }
  });
  it('extends a two-point path inside the grain to reproduce a straight cut', () => {
    expect(splitPolygon(square, [[50, 40], [50, 60]]).map(area)).toEqual([5000, 5000]);
    expect(splitPolygon(square, [[0, 0], [100, 100]]).map(area)).toEqual([5000, 5000]);
    expect(splitPolygon(square, [[50, 50], [50, 50], [50, 60]]).map(area)).toEqual([5000, 5000]);
  });
  it('extends a freehand path whose two ends are inside', () => {
    const parts = splitPolygon(square, [[30, 30], [50, 50], [70, 30]]);
    expect(parts).toHaveLength(2);
    expect(parts.reduce((s, p) => s + area(p), 0)).toBeCloseTo(10_000);
  });
  it('two interior runs produce three rings, sorted by area', () => {
    const parts = splitPolygon(square, [[-10, 30], [110, 30], [110, 70], [-10, 70]]);
    expect(parts.map(area)).toEqual([4000, 3000, 3000]);
  });
  it('a concave polygon can have multiple crossings along one segment', () => {
    const p: Polygon = [[0, 0], [100, 0], [100, 100], [70, 100], [70, 30], [30, 30], [30, 100], [0, 100]];
    const parts = splitPolygon(p, [[-10, 50], [110, 50]]);
    expect(parts).toHaveLength(3);
    expect(parts.reduce((s, q) => s + area(q), 0)).toBeCloseTo(area(p));
  });
  it('discards pieces below 5 percent and refuses fewer than two survivors', () => {
    const parts = splitPolygon(square, [[3, -10], [3, 110], [50, 110], [50, -10]]);
    expect(parts.map(area)).toEqual([5000, 4700]);
    expect(() => splitPolygon(square, [[3, -10], [3, 110]])).toThrow('all the way across');
    expect(splitPolygon(square, [[5, -10], [5, 110]]).map(area)).toEqual([9500, 500]);
  });
  it('rejects self intersections and retracing inside, while allowing crossings outside', () => {
    expect(() => splitPolygon(square, [[-10, 20], [90, 80], [10, 80], [110, 20]])).toThrow('crosses itself inside');
    expect(() => splitPolygon(square, [[-10, 50], [80, 50], [20, 50], [110, 60]])).toThrow('crosses itself inside');
    expect(() => splitPolygon(square, [[-10, 50], [110, 50], [-20, 50]])).toThrow('crosses itself inside');
    expect(splitPolygon(square, [[-10, 30], [110, 30], [120, -10], [120, 110]])).toHaveLength(2);
  });
  it('an interior boundary touch separates its two runs into three pieces', () => {
    const parts = splitPolygon(square, [[-10, 50], [50, 0], [110, 50]]);
    expect(parts).toHaveLength(3);
    expect(parts.reduce((s, p) => s + area(p), 0)).toBeCloseTo(10_000);
  });
  it('misses and tangencies do not split', () => {
    for (const path of [[[0, -10], [100, -10]], [[-10, 10], [10, -10]], [[0, 0], [100, 0]]] as Point[][]) {
      expect(() => splitPolygon(square, path)).toThrow('all the way across');
    }
  });
  it('skips far grain polygons before intersection work without changing the chosen grain', () => {
    const grain = parent(), path: Point[] = [[-10, 50], [110, 50]];
    let polygonReads = 0;
    const far = { ...grain, id: 'far', bbox_um: [1000, 1000, 1100, 1100] as Grain['bbox_um'],
      get polygon_um(): Polygon { polygonReads++; return square.map(([x, y]) => [x + 1000, y + 1000]); } };
    const expected = grainForCut([grain], path);
    expect(grainForCut([far, grain], path, [far.id])).toBe(expected);
    expect(polygonReads).toBe(0);
  });
  it('uses polygon extents when a legacy grain has no bbox', () => {
    const grain = parent(), far = { ...polygonGrain(square.map(([x, y]) => [x + 1000, y + 1000])), id: 'far' };
    Reflect.deleteProperty(grain, 'bbox_um');
    Reflect.deleteProperty(far, 'bbox_um');
    expect(grainForCut([far, grain], [[-10, 50], [110, 50]], [far.id])).toBe(grain);
    expect(grainForCut([grain], [[50, -10], [50, 110]])).toBe(grain);
  });
  it('a straight cut drawn across the second square never picks its neighbour by extension', () => {
    const first = parent(), second = { ...polygonGrain(square.map(([x, y]) => [x + 200, y])), id: 'second' };
    const path: Point[] = [[190, 50], [310, 50]];
    for (const grains of [[first, second], [second, first]]) {
      for (const selected of [[], [first.id]]) expect(grainForCut(grains, path, selected)?.id).toBe(second.id);
    }
    // Both extended ends cross grains, but the drawn path is entirely in the gap.
    expect(grainForCut([first, second], [[120, 50], [180, 50]], [first.id])).toBeUndefined();
    expect(grainForCut([first], [[-10, 0], [110, 0]], [first.id])).toBeUndefined();
  });
  it('picks a grain when the drawn path only starts inside and extension completes the cut', () => {
    const grain = parent(), path: Point[] = [[50, 50], [110, 50]];
    expect(grainForCut([grain], path)?.id).toBe(grain.id);
    expect(splitPolygon(grain.polygon_um, path).map(area)).toEqual([5000, 5000]);
  });
  it('ranks entered grains by drawn length, with selection taking priority', () => {
    const first = parent(), second = { ...polygonGrain(square.map(([x, y]) => [x + 200, y])), id: 'second' };
    const path: Point[] = [[-10, 50], [240, 50]];
    expect(grainForCut([second, first], path)?.id).toBe(first.id);
    expect(grainForCut([first, second], path, [second.id])?.id).toBe(second.id);
  });
  it('tries the next ranked grain when an entered grain has no valid extended split', () => {
    const first = parent(), second = { ...polygonGrain(square.map(([x, y]) => [x + 200, y + 48])), id: 'second' };
    // Only 2 percent of the second grain would survive above the cut.
    expect(grainForCut([second, first], [[-10, 50], [310, 50]], [second.id])?.id).toBe(first.id);
  });
  it('chooses crossed grains with selected ones first and reports bad cuts on that grain', () => {
    const a = parent(), b = { ...parent(), id: 'b' };
    const miss = { ...polygonGrain(square.map(([x, y]) => [x + 200, y])), id: 'miss' };
    const cut: Point[] = [[50, -10], [50, 110]];
    expect(grainForCut([a, b], cut, ['b'])?.id).toBe('b');
    expect(grainForCut([miss, a], cut, ['miss'])?.id).toBe(a.id);
    expect(grainForCut([a], [[-10, -10], [-10, 110]])).toBeUndefined();
    expect(grainForCut([a], [[3, -10], [3, 110]])?.id).toBe(a.id);
  });
});

describe('N-piece lineage', () => {
  it('creates three new identities and area-ordered suffixes, retaining sample and number', () => {
    const p = parent(), polygons = splitPolygon(square, [[-10, 30], [110, 30], [110, 70], [-10, 70]]);
    let id = 0;
    const children = splitChildren(p, polygons.reverse().map(q => ({ ...polygonGrain(q), merged_from: ['old'], split_from: 'old' })), 'A', () => `child-${++id}`);
    expect(children.map(g => g.area_um2)).toEqual([4000, 3000, 3000]);
    expect(children.map(g => g.label)).toEqual(['A/7a', 'A/7b', 'A/7c']);
    expect(children.map(g => g.id)).toEqual(['child-1', 'child-2', 'child-3']);
    for (const g of children) {
      expect(g).toMatchObject({ number: p.number, sample: p.sample, split_from: p.id });
      expect(g.merged_from).toBeUndefined();
    }
    const spot = { id: 'spot-registered', name: 'keep-this-name', grain_id: p.id, x_um: 50, y_um: 50 } as Spot;
    expect(reassignLineage([spot], [p], children)[0]).toEqual({ ...spot, grain_id: 'child-1', grain_number: p.number });
    const boundary = { ...spot, y_um: 30 };
    expect(reassignLineage([boundary], [p], children)[0].grain_id).toBe('child-1');
    expect(() => reassignLineage([{ ...spot, x_um: -1 }], [p], children)).toThrow('would lie outside');
    expect(() => splitChildren(p, [children[0]], 'A')).toThrow('at least two');
  });
  it('continues suffixes beyond z', () => {
    const children = splitChildren(parent(), Array.from({ length: 28 }, parent), 'A', () => 'synthetic');
    expect(children.slice(25).map(g => g.label)).toEqual(['A/7z', 'A/7aa', 'A/7ab']);
  });
});
