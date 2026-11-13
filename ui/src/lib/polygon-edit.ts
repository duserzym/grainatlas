// Geometry-only editing of simple, implicitly closed polygons in micrometres.
// Boolean results must have one outer ring and no holes: a grain stores one ring.
import type { Grain } from './core/types.ts';
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
export function splitPolygon(polygon: Polygon, line: [number, number, number, number]): [Polygon, Polygon] {
  const p = ring(polygon), a: Point = [line[0], line[1]], b: Point = [line[2], line[3]], dir = sub(b, a);
  if (same(a, b) || contains(p, a, false) || contains(p, b, false)) throw new Error('Draw the cut all the way across the grain.');
  const hits = p.flatMap((c, i) => intersections(a, b, c, p[(i + 1) % p.length])).sort((a, b) => a - b).filter((t, i, ts) => !i || Math.abs(t - ts[i - 1]) > EPS);
  if (hits.length !== 2 || !contains(p, lerp(a, b, (hits[0] + hits[1]) / 2), false)) throw new Error('The cut must cross the grain fully in two places.');
  const side = (q: Point) => cross(dir, sub(q, a));
  function half(sign: number): Polygon {
    const out: Polygon = [];
    for (let i = 0; i < p.length; i++) {
      const c = p[i], d = p[(i + 1) % p.length], sc = side(c) * sign, sd = side(d) * sign;
      if (sc >= -EPS) out.push(c);
      if ((sc > EPS && sd < -EPS) || (sc < -EPS && sd > EPS)) out.push(lerp(c, d, sc / (sc - sd)));
    }
    return ring(out);
  }
  return [half(1), half(-1)].sort((a, b) => Math.abs(signedArea(b)) - Math.abs(signedArea(a))) as [Polygon, Polygon];
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
