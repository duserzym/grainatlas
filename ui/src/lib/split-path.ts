/** Pure helpers for split strokes, in the same units as the supplied points. */
export type Point = [number, number];

/** Keep samples at least one screen pixel apart (pass micrometres per screen px). */
export function appendPathPoint(path: Point[], point: Point, minDistance: number): boolean {
  const last = path.at(-1);
  if (last && Math.hypot(point[0] - last[0], point[1] - last[1]) < minDistance) return false;
  path.push(point);
  return true;
}

function distanceSquared(p: Point, a: Point, b: Point): number {
  const dx = b[0] - a[0], dy = b[1] - a[1], length = dx * dx + dy * dy;
  const t = length ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length)) : 0;
  return (p[0] - a[0] - t * dx) ** 2 + (p[1] - a[1] - t * dy) ** 2;
}

/** Iterative Ramer–Douglas–Peucker: retain the endpoints without recursion limits. */
export function simplifyPath(path: Point[], tolerance: number): Point[] {
  if (path.length < 3) return path.slice();
  const keep = new Set([0, path.length - 1]), stack = [[0, path.length - 1]];
  while (stack.length) {
    const [first, last] = stack.pop()!;
    let farthest = -1, distance = tolerance * tolerance;
    for (let i = first + 1; i < last; i++) {
      const d = distanceSquared(path[i], path[first], path[last]);
      if (d > distance) { distance = d; farthest = i; }
    }
    if (farthest >= 0) { keep.add(farthest); stack.push([first, farthest], [farthest, last]); }
  }
  return path.filter((_, i) => keep.has(i));
}

/** Continue each open end along its end segment until it leaves the bbox. */
export function extendPath(path: Point[], bounds: [number, number, number, number]): Point[] {
  if (path.length < 2 || path.some(p => !p.every(Number.isFinite))) throw new Error('A cut needs at least two finite points.');
  const points = path.filter((p, i) => !i || p[0] !== path[i - 1][0] || p[1] !== path[i - 1][1]);
  if (points.length < 2) throw new Error('Draw the cut all the way across the grain.');
  const margin = Math.max(1e-5, Math.hypot(bounds[2] - bounds[0], bounds[3] - bounds[1]) * 1e-6);
  function extend(at: Point, neighbour: Point): Point {
    const d = [at[0] - neighbour[0], at[1] - neighbour[1]];
    let exit = Infinity;
    for (const axis of [0, 1]) {
      if (d[axis] > 0) exit = Math.min(exit, (bounds[axis + 2] + margin - at[axis]) / d[axis]);
      if (d[axis] < 0) exit = Math.min(exit, (bounds[axis] - margin - at[axis]) / d[axis]);
    }
    return Number.isFinite(exit) && exit > 0 ? [at[0] + exit * d[0], at[1] + exit * d[1]] : at;
  }
  const end = points.length - 1;
  return [extend(points[0], points[1]), ...points, extend(points[end], points[end - 1])]
    .filter((p, i, ps) => !i || p[0] !== ps[i - 1][0] || p[1] !== ps[i - 1][1]);
}
