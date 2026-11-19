import type { AreaImageEdits, Grain, ImageEdits, Project, MosaicJson } from './core/types.ts';
export type Point = [number, number];
export type Affine = [number, number, number, number, number, number];
export const identity: Affine = [1, 0, 0, 1, 0, 0];
export const emptyImageEdits = (): AreaImageEdits => ({ rotate_deg: 0, flip_h: false, flip_v: false, masks: [] });
/** Image-pixel appearance, shared with the Rust print sampler (mount µm, not screen px). */
export const maskAppearance = { background:224, ink:144, edgeUm:6, pitchUm:18, lineUm:2 } as const;
/** Bound overview work independently of the number of full-resolution tiles. */
export async function overviewHistogram(source: { pyramidTile(area: string, level: number, col: number, row: number): Promise<{ data: Uint8Array }> }, area: string, mosaic: MosaicJson, cancelled = () => false): Promise<Uint32Array | null> {
  const level = mosaic.levels.at(-1);
  if (!level || !level.cols || !level.rows) return null;
  const bins = new Uint32Array(256);
  const columns = [...new Set([0, level.cols - 1])], rows = [...new Set([0, level.rows - 1])];
  for (const row of rows) for (const col of columns) {
    if (cancelled()) return null;
    const tile = await source.pyramidTile(area, level.level, col, row);
    if (cancelled()) return null;
    for (const value of tile.data) bins[value]++;
  }
  return bins;
}
export function readImageEdits(value: unknown): ImageEdits | null {
  if (!value || typeof value !== 'object') return null;
  const v = value as ImageEdits;
  const point = (p: unknown): p is Point => Array.isArray(p) && p.length === 2 && p.every(Number.isFinite);
  const points = (p: unknown, min: number): p is Point[] => Array.isArray(p) && p.length >= min && p.every(point);
  if (v.version !== 1 || !v.areas || typeof v.areas !== 'object' || Array.isArray(v.areas)) return null;
  for (const e of Object.values(v.areas)) {
    if (!e || !Number.isFinite(e.rotate_deg) || typeof e.flip_h !== 'boolean' || typeof e.flip_v !== 'boolean' || !Array.isArray(e.masks)) return null;
    if (e.crop && (!['rect', 'polygon'].includes(e.crop.kind) || !points(e.crop.points_um, e.crop.kind === 'rect' ? 2 : 3))) return null;
    if (e.levels && ![e.levels.black, e.levels.white].every(n => Number.isInteger(n) && n >= 0 && n <= 255)) return null;
    if (e.masks.some(m => !points(m.points_um, 1) || !Number.isFinite(m.radius_um) || m.radius_um <= 0)) return null;
  }
  return v;
}
export const areaImageEdits = (project: Project | null, area: string | null): AreaImageEdits => (area && readImageEdits(project?.image_edits)?.areas[area]) || emptyImageEdits();
export function pointBounds(points: Point[]): [number, number, number, number] {
  return points.reduce(([x0, y0, x1, y1], [x, y]) => [Math.min(x0, x), Math.min(y0, y), Math.max(x1, x), Math.max(y1, y)], [Infinity, Infinity, -Infinity, -Infinity]);
}
export function segmentDistance(p: Point, a: Point, b: Point): number {
  const dx = b[0] - a[0], dy = b[1] - a[1], n = dx * dx + dy * dy;
  const t = n ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / n)) : 0;
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}
export function imagePointVisible(p: Point, e: AreaImageEdits): boolean {
  const c = e.crop;
  if (c?.kind === 'rect') {
    const b = pointBounds(c.points_um);
    if (p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3]) return false;
  } else if (c) {
    let inside = false, edge = false;
    c.points_um.forEach((a, i) => {
      const b = c.points_um[(i + 1) % c.points_um.length];
      edge ||= segmentDistance(p, a, b) <= 1e-8;
      if ((a[1] > p[1]) !== (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]) inside = !inside;
    });
    if (!inside && !edge) return false;
  }
  return !e.masks.some(m => m.points_um.some((a, i) => segmentDistance(p, a, m.points_um[i + 1] ?? a) <= m.radius_um));
}
export const printableGrains = (grains: Grain[], e: AreaImageEdits) => grains.filter(g => imagePointVisible(g.centroid_um, e));
export const applyAffine = (m: Affine, [x, y]: Point): Point => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
export function inverseAffine(m: Affine): Affine {
  const d = m[0] * m[3] - m[1] * m[2];
  return [m[3] / d, -m[1] / d, -m[2] / d, m[0] / d, (m[2] * m[5] - m[3] * m[4]) / d, (m[1] * m[4] - m[0] * m[5]) / d];
}
export function composeAffine(a: Affine, b: Affine): Affine {
  const o = applyAffine(a, [b[4], b[5]]);
  return [a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1], a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3], ...o];
}
/** Rotation then view navigation, with flips in the original mount axes. */
export function imageTransform(e: AreaImageEdits, size: Point, umPerPx = 1): Affine {
  const b = e.crop ? pointBounds(e.crop.points_um) : [0, 0, size[0] * umPerPx, size[1] * umPerPx];
  const x = (b[0] + b[2]) / 2 / umPerPx, y = (b[1] + b[3]) / 2 / umPerPx;
  const r = e.rotate_deg * Math.PI / 180, c = Math.cos(r), s = Math.sin(r), h = e.flip_h ? -1 : 1, v = e.flip_v ? -1 : 1;
  return [c * h, s * h, -s * v, c * v, x - c * h * x + s * v * y, y - s * h * x - c * v * y];
}
export function levelsLut(lut: Uint8Array, e: AreaImageEdits): Uint8Array {
  const l = e.levels;
  return l && l.white > l.black ? lut.map(v => Math.max(0, Math.min(255, Math.round((v - l.black) * 255 / (l.white - l.black))))) : lut;
}
export function storeImageEdits(app: { project: Project | null; readOnly: boolean; checkpoint(): void; persist(): void }, area: string, e: AreaImageEdits) {
  if (!app.project || app.readOnly) return;
  const edits = readImageEdits(app.project.image_edits) ?? { version: 1, areas: {} };
  const areas={...edits.areas};
  if(!e.crop && !e.rotate_deg && !e.flip_h && !e.flip_v && !e.levels && !e.masks.length) delete areas[area];
  else areas[area]=structuredClone(e);
  const next:ImageEdits|undefined=Object.keys(areas).length?{version:1,areas}:undefined;
  if(JSON.stringify(app.project.image_edits)===JSON.stringify(next))return;
  app.checkpoint();
  if(next)app.project.image_edits=next;
  else delete app.project.image_edits;
  app.persist();
}
export function imageRecipe(e: AreaImageEdits): string[] {
  return [e.crop && `${e.crop.kind === 'rect' ? 'Rectangle' : 'Polygon'} crop`, e.rotate_deg && `Rotate ${e.rotate_deg.toFixed(1)}°`, e.flip_h && 'Flip horizontal', e.flip_v && 'Flip vertical', e.levels && `Levels ${e.levels.black} → ${e.levels.white}`, e.masks.length && `Mask · ${e.masks.length} ${e.masks.length===1?'stroke':'strokes'}`].filter(Boolean) as string[];
}
/** A disposable copy in the print display frame. Never pass this back to storage or sync. */
export function printViewGrains(grains: Grain[], e: AreaImageEdits, sizeUm: Point): Grain[] {
  const m=imageTransform(e,sizeUm);
  const b=pointBounds([[0,0],[sizeUm[0],0],[0,sizeUm[1]],sizeUm].map(p=>applyAffine(m,p as Point)));
  m[4]-=b[0];m[5]-=b[1];
  return printableGrains(grains,e).map(g=>{
    const b=g.bbox_um;
    return {...g,centroid_um:applyAffine(m,g.centroid_um),polygon_um:g.polygon_um.map(p=>applyAffine(m,p)),bbox_um:pointBounds([[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]].map(p=>applyAffine(m,p as Point)))};
  });
}
