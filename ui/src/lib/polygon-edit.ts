// Geometry-only editing of simple, implicitly closed polygons in micrometres.
// Boolean results must have one outer ring and no holes: a grain stores one ring.
import type { Grain } from './core/types.ts';
import { extendPath } from './split-path.ts';
export type Point = [number, number];
export type Polygon = Point[];
const EPS = 1e-7;
const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];
const sub = (a: Point, b: Point): Point => [a[0] - b[0], a[1] - b[1]];
const lerp = (a: Point, b: Point, t: number): Point => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
const same = (a: Point, b: Point) => Math.hypot(a[0] - b[0], a[1] - b[1]) < EPS;
function signedArea(p: Polygon) { return p.reduce((s, a, i) => s + cross(a, p[(i + 1) % p.length]), 0) / 2; }
function clean(p: Polygon): Polygon {
  let out = p.filter((a, i) => !same(a, p[(i + 1) % p.length]));
  out = out.filter((b, i) => Math.abs(cross(sub(b, out[(i + out.length - 1) % out.length]), sub(out[(i + 1) % out.length], b))) > EPS);
  return out;
}
function ring(p: Polygon): Polygon {
  const q = clean(p);
  if (q.length < 3 || q.some(p => !p.every(Number.isFinite)) || Math.abs(signedArea(q)) < EPS) throw new Error('Draw an outline with an area.');
  // Refuse self intersections rather than save an invalid server geometry.
  for (let i = 0; i < q.length; i++) for (let j = i + 2; j < q.length; j++) {
    if (i === 0 && j === q.length - 1) continue;
    if (intersections(q[i], q[(i + 1) % q.length], q[j], q[(j + 1) % q.length]).length) throw new Error('The outline crosses itself.');
  }
  return signedArea(q) > 0 ? q : [...q].reverse();
}
function intersections(a: Point, b: Point, c: Point, d: Point): number[] {
  const r = sub(b, a), s = sub(d, c), ca = sub(c, a), det = cross(r, s);
  if (Math.abs(det) > EPS) {
    const t = cross(ca, s) / det, u = cross(ca, r) / det;
    return t >= -EPS && t <= 1 + EPS && u >= -EPS && u <= 1 + EPS ? [Math.min(1, Math.max(0, t))] : [];
  }
  if (Math.abs(cross(ca, r)) > EPS) return [];
  const rr = r[0] ** 2 + r[1] ** 2;
  return [c, d].map(p => ((p[0] - a[0]) * r[0] + (p[1] - a[1]) * r[1]) / rr).filter(t => t >= -EPS && t <= 1 + EPS).map(t => Math.min(1, Math.max(0, t)));
}
export function contains(p: Polygon, at: Point, boundary = true): boolean {
  let inside = false;
  for (let i = 0, j = p.length - 1; i < p.length; j = i++) {
    const a = p[j], b = p[i];
    if (Math.abs(cross(sub(b, a), sub(at, a))) < EPS && at[0] >= Math.min(a[0], b[0]) - EPS && at[0] <= Math.max(a[0], b[0]) + EPS && at[1] >= Math.min(a[1], b[1]) - EPS && at[1] <= Math.max(a[1], b[1]) + EPS) return boundary;
    if ((a[1] > at[1]) !== (b[1] > at[1]) && at[0] < (b[0] - a[0]) * (at[1] - a[1]) / (b[1] - a[1]) + a[0]) inside = !inside;
  }
  return inside;
}
const cutAcross = 'Draw the cut all the way across the grain.';
function pointBounds(points: Point[]): Grain['bbox_um'] {
  const bounds: Grain['bbox_um'] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const [x, y] of points) {
    bounds[0] = Math.min(bounds[0], x); bounds[1] = Math.min(bounds[1], y);
    bounds[2] = Math.max(bounds[2], x); bounds[3] = Math.max(bounds[3], y);
  }
  return bounds;
}
function extendedCut(p: Polygon, path: Point[]): Point[] {
  return extendPath(path, pointBounds(p));
}
/** Each run starts and ends on the boundary; exterior stretches separate runs. */
function insideRuns(p: Polygon, path: Point[]): Point[][] {
  const runs: Point[][] = [];
  let run: Point[] = [];
  const finish = () => { if (run.length > 1) runs.push(run); run = []; };
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1], b = path[i];
    const ts = [0, 1, ...p.flatMap((c, j) => intersections(a, b, c, p[(j + 1) % p.length]))]
      .sort((a, b) => a - b).filter((t, j, ts) => !j || t - ts[j - 1] > EPS);
    for (let j = 1; j < ts.length; j++) {
      const c = lerp(a, b, ts[j - 1]), d = lerp(a, b, ts[j]);
      if (same(c, d)) continue;
      if (contains(p, lerp(c, d, .5), false)) {
        if (!run.length) run.push(c);
        run.push(d);
        // A boundary touch can separate pieces even without an exterior gap.
        if (!contains(p, d, false)) finish();
      } else finish();
    }
  }
  finish();
  return runs;
}
function refuseCrossing(p: Polygon, path: Point[]) {
  for (let i = 1; i < path.length; i++) for (let j = i + 1; j < path.length; j++) {
    const a = path[i - 1], b = path[i], c = path[j - 1], d = path[j];
    const hits = [...intersections(a, b, c, d).map(t => lerp(a, b, t)), ...intersections(c, d, a, b).map(t => lerp(c, d, t))];
    const crossing = hits.some(q => !(j === i + 1 && same(q, b)) && contains(p, q, false));
    // An overlap can span the whole grain with both overlap endpoints outside.
    const ordered = hits.slice().sort((q, r) => Math.hypot(q[0] - a[0], q[1] - a[1]) - Math.hypot(r[0] - a[0], r[1] - a[1]));
    const overlap = ordered.length > 1 && !same(ordered[0], ordered.at(-1)!)
      && insideRuns(p, [ordered[0], ordered.at(-1)!]).length > 0;
    if (crossing || overlap) {
      throw new Error('The cut crosses itself inside the grain. Draw a path that does not cross itself.');
    }
  }
}
function splitRun(p: Polygon, run: Point[]): Polygon[] {
  const a = run[0], b = run.at(-1)!;
  if (same(a, b)) throw new Error('The cut returns to the same boundary point. Draw across the grain.');
  const boundary: Point[] = [];
  for (let i = 0; i < p.length; i++) {
    const c = p[i], d = p[(i + 1) % p.length], v = sub(d, c), length2 = v[0] ** 2 + v[1] ** 2;
    const insert = [a, b].map(q => ({ q, t: ((q[0] - c[0]) * v[0] + (q[1] - c[1]) * v[1]) / length2 }))
      .filter(({ q, t }) => t > EPS && t < 1 - EPS && Math.abs(cross(v, sub(q, c))) < EPS)
      .sort((a, b) => a.t - b.t);
    boundary.push(c, ...insert.map(({ q }) => q));
  }
  const ia = boundary.findIndex(q => same(q, a)), ib = boundary.findIndex(q => same(q, b));
  if (ia < 0 || ib < 0) throw new Error(cutAcross);
  function arc(start: number, end: number): Point[] {
    const out = [boundary[start]];
    for (let i = (start + 1) % boundary.length; i !== end; i = (i + 1) % boundary.length) out.push(boundary[i]);
    out.push(boundary[end]);
    return out;
  }
  const interior = run.slice(1, -1);
  return [ring([...arc(ia, ib), ...interior.slice().reverse()]), ring([...arc(ib, ia), ...interior])];
}
/** Cut the ring along every interior run, preserving the shape of the drawn path. */
export function splitPolygon(polygon: Polygon, path: Point[]): Polygon[] {
  const p = ring(polygon), extended = extendedCut(p, path);
  refuseCrossing(p, extended);
  const runs = insideRuns(p, extended);
  let parts = [p];
  for (const run of runs) {
    const probe = lerp(run[0], run[1], .5);
    const index = parts.findIndex(q => contains(q, probe, false));
    if (index < 0) throw new Error(cutAcross);
    parts.splice(index, 1, ...splitRun(parts[index], run));
  }
  const minimum = Math.abs(signedArea(p)) * .05;
  parts = parts.filter(q => Math.abs(signedArea(q)) + EPS >= minimum)
    .sort((a, b) => Math.abs(signedArea(b)) - Math.abs(signedArea(a)));
  if (parts.length < 2) throw new Error(cutAcross);
  return parts;
}
/** Only the drawn path chooses the grain; extensions are used to split it.
 * Prefer selected grains, then the greatest drawn length strictly inside. */
export function grainForCut(grains: Grain[], path: Point[], selected: string[] = []): Grain | undefined {
  const ids = new Set(selected), drawnBounds = pointBounds(path);
  // Reject distant grains before inspecting their polygon edges. Keep touching
  // bboxes here so horizontal/vertical strokes still reach the strict inside test.
  const candidates = grains.filter(grain => {
    const bounds = grain.bbox_um ?? pointBounds(grain.polygon_um);
    return bounds[0] <= drawnBounds[2] && bounds[2] >= drawnBounds[0]
      && bounds[1] <= drawnBounds[3] && bounds[3] >= drawnBounds[1];
  }).map(grain => ({ grain, length: insideRuns(grain.polygon_um, path)
    .reduce((sum, run) => sum + run.slice(1).reduce((length, p, i) => length + Math.hypot(p[0] - run[i][0], p[1] - run[i][1]), 0), 0) }))
    .filter(candidate => candidate.length > 0)
    .sort((a, b) => Number(ids.has(b.grain.id)) - Number(ids.has(a.grain.id)) || b.length - a.length);
  for (const { grain } of candidates) {
    try { splitPolygon(grain.polygon_um, path); return grain; } catch { /* try the next entered outline */ }
  }
  // Preserve the precise split error in the caller when no entered grain can
  // be split (for example a self-crossing cut or a discarded tiny sliver).
  return candidates[0]?.grain;
}
/** Split all edges at intersections, keep boundary segments with material on their left,
 * then trace. Offset probes also resolve coincident edges and containment. */
export function polygonBoolean(polygons: Polygon[], operation: 'union' | 'difference' = 'union'): Polygon {
  const ps = polygons.map(ring);
  const material = (p: Point) => operation === 'union' ? ps.some(q => contains(q, p, false)) : contains(ps[0], p, false) && !ps.slice(1).some(q => contains(q, p, false));
  const edges = ps.flatMap(p => p.map((a, i) => [a, p[(i + 1) % p.length]] as [Point, Point]));
  const segments: [Point, Point][] = [];
  for (const [a, b] of edges) {
    const ts = [0, 1, ...edges.flatMap(([c, d]) => intersections(a, b, c, d))].sort((a, b) => a - b).filter((t, i, ts) => !i || t - ts[i - 1] > EPS);
    for (let i = 1; i < ts.length; i++) {
      const c = lerp(a, b, ts[i - 1]), d = lerp(a, b, ts[i]), m = lerp(c, d, .5), v = sub(d, c), length = Math.hypot(...v);
      if (length < EPS) continue;
      const offset = Math.min(1e-4, length * 1e-4), n: Point = [-v[1] / length * offset, v[0] / length * offset];
      const left = material([m[0] + n[0], m[1] + n[1]]), right = material([m[0] - n[0], m[1] - n[1]]);
      if (left === right) continue;
      const edge: [Point, Point] = left ? [c, d] : [d, c];
      if (!segments.some(([s, e]) => same(s, edge[0]) && same(e, edge[1]))) segments.push(edge);
    }
  }
  if (!segments.length) throw new Error('That cut removes the whole grain.');
  const out: Polygon = [], start = segments[0][0];
  let at = start;
  do {
    const next = segments.map((s, i) => same(s[0], at) ? i : -1).filter(i => i >= 0);
    if (next.length !== 1) throw new Error('The outlines must share an edge or overlap; a point contact cannot make one grain.');
    const [edge] = segments.splice(next[0], 1); out.push(edge[0]); at = edge[1];
  } while (!same(at, start));
  if (segments.length) throw new Error('The outlines do not touch, or the result has a hole or separate pieces.');
  return ring(out);
}
export const mergePolygons = (polygons: Polygon[]) => polygonBoolean(polygons);
export function polygonGrain(polygon: Polygon, base?: Grain | null): Grain {
  const p = ring(polygon), area = signedArea(p), xs = p.map(p => p[0]), ys = p.map(p => p[1]);
  const centroid = p.reduce<Point>((sum, a, i) => { const b = p[(i + 1) % p.length], c = cross(a, b); return [sum[0] + (a[0] + b[0]) * c / (6 * area), sum[1] + (a[1] + b[1]) * c / (6 * area)]; }, [0, 0]);
  const bbox: Grain['bbox_um'] = [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
  return { id: crypto.randomUUID(), number: 0, mean_intensity: 0, scale_bar_um: 0, ...base, polygon_um: p, centroid_um: centroid, area_um2: area, bbox_um: bbox, eq_diameter_um: Math.sqrt(4 * area / Math.PI), major_um: Math.max(bbox[2] - bbox[0], bbox[3] - bbox[1]), minor_um: Math.min(bbox[2] - bbox[0], bbox[3] - bbox[1]), orientation_deg: 0, manual: true, drawn: true };
}
