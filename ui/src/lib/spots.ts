// Spot helpers for the canvas: naming (same rules as grainatlas-core
// spots::format_name), snapping, hit tests and the stage mapping.

import type { AreaRecord, Grain, Spot } from './core/types';

export function formatName(pattern: string, sample: string, grain: number | null, n: number): string {
  return pattern.replace(/\{(\w+)(?::(0\d+))?\}/g, (m, key: string, width?: string) => {
    let v: string;
    if (key === 'sample') return sample;
    else if (key === 'grain') v = String(grain ?? 0);
    else if (key === 'n') v = String(n);
    else return m;
    return width ? v.padStart(Number(width), '0') : v;
  });
}

export function nextN(spots: Spot[]): number {
  return spots.reduce((m, s) => Math.max(m, s.n || 0), 0) + 1;
}

export function inside(poly: [number, number][], px: number, py: number) {
  let c = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [ax, ay] = poly[i], [bx, by] = poly[j];
    if (ay > py !== by > py && px < ((bx - ax) * (py - ay)) / (by - ay) + ax) c = !c;
  }
  return c;
}

/** The grain under a point; artifacts only when asked (selection), never for spot linking. */
export function grainAt(gs: Grain[], x: number, y: number, artifacts = false): Grain | null {
  for (const g of gs) {
    if (g.artifact && !artifacts) continue;
    const [x0, y0, x1, y1] = g.bbox_um;
    if (x < x0 || y < y0 || x > x1 || y > y1) continue;
    if (inside(g.polygon_um, x, y)) return g;
  }
  return null;
}

export interface SnapTarget { x: number; y: number; what: string }

/** Snap targets: grain centroids, long-axis ends (kept inside by the spot radius), previous spot. */
export function snapTargets(grains: Grain[], previous: Spot | null, diameter: number): SnapTarget[] {
  const out: SnapTarget[] = [];
  for (const g of grains) {
    out.push({ x: g.centroid_um[0], y: g.centroid_um[1], what: `grain ${g.number} centre` });
    const a = (g.orientation_deg * Math.PI) / 180;
    const r = Math.max(0, g.major_um / 2 - diameter / 2 - 2);
    if (r > diameter / 2) {
      out.push({ x: g.centroid_um[0] + r * Math.cos(a), y: g.centroid_um[1] + r * Math.sin(a), what: `grain ${g.number} tip` });
      out.push({ x: g.centroid_um[0] - r * Math.cos(a), y: g.centroid_um[1] - r * Math.sin(a), what: `grain ${g.number} tip` });
    }
  }
  if (previous) out.push({ x: previous.x_um, y: previous.y_um, what: `previous spot ${previous.name}` });
  return out;
}

export function snap(x: number, y: number, targets: SnapTarget[], radiusUm: number): SnapTarget | null {
  let best: SnapTarget | null = null;
  let bd = radiusUm;
  for (const t of targets) {
    const d = Math.hypot(t.x - x, t.y - y);
    if (d < bd) {
      bd = d;
      best = t;
    }
  }
  return best;
}

/** Stage position (nm) of a mosaic point (um), through the nearest tile (same as AreaRecord::stage_of). */
export function stageOf(area: AreaRecord | null, x: number, y: number): [number, number] | null {
  const pl = area?.placement;
  if (!area || !pl) return null;
  const k = area.pixel_size_nm / 1000;
  let best: [number, [number, number]] | null = null;
  pl.tiles.forEach((t, i) => {
    const st = area.tiles[i]?.stage_nm;
    if (!st) return;
    const cx = (t.pos[0] + t.w / 2) * k, cy = (t.pos[1] + t.h / 2) * k;
    const d = Math.hypot(x - cx, y - cy);
    const s: [number, number] = [st[0] + area.stage_sign[0] * (x - cx) * 1000, st[1] + area.stage_sign[1] * (y - cy) * 1000];
    if (!best || d < best[0]) best = [d, s];
  });
  return best ? (best as [number, [number, number]])[1] : null;
}

/** Distance (um) from a point to a spot's shape. */
export function spotDistance(s: Spot, x: number, y: number): number {
  if (s.kind === 'line' && s.x2_um != null && s.y2_um != null) {
    const dx = s.x2_um - s.x_um, dy = s.y2_um - s.y_um;
    const t = Math.max(0, Math.min(1, ((x - s.x_um) * dx + (y - s.y_um) * dy) / (dx * dx + dy * dy || 1)));
    return Math.max(0, Math.hypot(x - (s.x_um + t * dx), y - (s.y_um + t * dy)) - s.diameter_um / 2);
  }
  if (s.kind === 'raster' && s.x2_um != null && s.y2_um != null) {
    const [x0, x1] = [Math.min(s.x_um, s.x2_um), Math.max(s.x_um, s.x2_um)];
    const [y0, y1] = [Math.min(s.y_um, s.y2_um), Math.max(s.y_um, s.y2_um)];
    const dx = Math.max(x0 - x, 0, x - x1), dy = Math.max(y0 - y, 0, y - y1);
    return Math.hypot(dx, dy);
  }
  return Math.max(0, Math.hypot(x - s.x_um, y - s.y_um) - s.diameter_um / 2);
}

export function newId(): string {
  // UUIDv7: 48-bit ms timestamp, version 7, variant 10, random rest (RFC 9562).
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  const ms = Date.now();
  for (let i = 0; i < 6; i++) b[i] = Math.floor(ms / 2 ** (8 * (5 - i))) & 0xff;
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (v) => v.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
