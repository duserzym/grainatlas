// Mount locator inset of the read-only viewer: where each area (one sample's line of
// grains) sits on the mount, in stage micrometres. Pure geometry, no DOM.
//
// Mount coordinates are the SEM view of the stage, signed like the mosaic:
// u = stage_sign * stage_nm / 1000. The print sheets' mount map uses the same frame
// (`area_boxes` in crates/grainatlas-core/src/layout/sheets.rs); this is its port.

import type { AreaRecord, Spot } from './core/types';
import type { ReviewGroup } from './shellcmd';
import { toMosaic, toScreen, type View } from './canvas/view.ts';

/** [x0, y0, x1, y1] in mount µm. */
export type Box = [number, number, number, number];
export type Pt = [number, number];
export type StageArea = Pick<AreaRecord, 'id' | 'name' | 'sample' | 'pixel_size_nm' | 'stage_sign' | 'tiles' | 'placement'>;

export interface InsetArea {
  id: string;
  label: string;
  box: Box;
}
export interface MountLayout {
  areas: InsetArea[];
  /** True when every box comes from recorded stage positions; false for the row fallback. */
  positioned: boolean;
  bounds: Box;
}

export const label = (a: Pick<AreaRecord, 'id' | 'name' | 'sample'>) => a.sample || a.name || a.id;

/** Stage box of an area from its tiles' stage centres and sizes; null when any tile lacks a stage position. */
export function stageBox(a: StageArea): Box | null {
  if (!a.tiles.length) return null;
  const b: Box = [Infinity, Infinity, -Infinity, -Infinity];
  for (const t of a.tiles) {
    if (!t.stage_nm) return null;
    const u = a.stage_sign[0] * t.stage_nm[0] / 1000, v = a.stage_sign[1] * t.stage_nm[1] / 1000;
    const hw = t.w * t.pixel_size_nm / 2000, hh = t.h * t.pixel_size_nm / 2000;
    b[0] = Math.min(b[0], u - hw); b[1] = Math.min(b[1], v - hh);
    b[2] = Math.max(b[2], u + hw); b[3] = Math.max(b[3], v + hh);
  }
  return Number.isFinite(b[0]) ? b : null;
}

/** Size (µm) of an area's mosaic: the stitched mosaic when known, else its placement, else its nominal tiles. */
export function areaExtent(a: StageArea, mosaic?: { w: number; h: number; um_per_px: number } | null): Pt {
  if (mosaic) return [mosaic.w * mosaic.um_per_px, mosaic.h * mosaic.um_per_px];
  const k = a.pixel_size_nm / 1000;
  if (a.placement) return [a.placement.width * k, a.placement.height * k];
  if (!a.tiles.length) return [1, 1];
  const x0 = Math.min(...a.tiles.map(t => t.pos_um[0])), y0 = Math.min(...a.tiles.map(t => t.pos_um[1]));
  const x1 = Math.max(...a.tiles.map(t => t.pos_um[0] + t.w * t.pixel_size_nm / 1000));
  const y1 = Math.max(...a.tiles.map(t => t.pos_um[1] + t.h * t.pixel_size_nm / 1000));
  return [Math.max(1, x1 - x0), Math.max(1, y1 - y0)];
}

export function union(boxes: Box[]): Box {
  if (!boxes.length) return [0, 0, 1, 1];
  return [Math.min(...boxes.map(b => b[0])), Math.min(...boxes.map(b => b[1])), Math.max(...boxes.map(b => b[2])), Math.max(...boxes.map(b => b[3]))];
}

/**
 * The mount: every area at its stage box when all areas have stage positions. Otherwise
 * (one area or more without them) no position is guessed: the areas stand left to right in
 * mount order, top-aligned, at their own size, and `positioned` is false.
 */
export function mountLayout(areas: StageArea[], extents: Record<string, Pt> = {}): MountLayout {
  const boxes = areas.map(stageBox);
  if (areas.length && boxes.every(Boolean)) {
    const out = areas.map((a, i) => ({ id: a.id, label: label(a), box: boxes[i]! }));
    return { areas: out, positioned: true, bounds: union(out.map(a => a.box)) };
  }
  const sizes = areas.map(a => extents[a.id] ?? areaExtent(a));
  const gap = Math.max(1, 0.15 * Math.max(0, ...sizes.map(s => s[1])));
  let x = 0;
  const out = areas.map((a, i) => {
    const box: Box = [x, 0, x + sizes[i][0], sizes[i][1]];
    x += sizes[i][0] + gap;
    return { id: a.id, label: label(a), box };
  });
  return { areas: out, positioned: false, bounds: union(out.map(a => a.box)) };
}

/**
 * Mount position of a mosaic point (µm). With stage positions, through the nearest tile
 * exactly as `stageOf` (spots.ts) and the core's `AreaRecord::stage_of`; tiles without a
 * placement use their nominal `pos_um`. In the row fallback, the area's box corner plus the point.
 */
export function mosaicToMount(a: StageArea, layout: MountLayout, x: number, y: number): Pt | null {
  const at = layout.areas.find(i => i.id === a.id);
  if (!at) return null;
  if (!layout.positioned) return [at.box[0] + x, at.box[1] + y];
  const k = a.pixel_size_nm / 1000;
  let best: [number, Pt] | null = null;
  a.tiles.forEach((t, i) => {
    if (!t.stage_nm) return;
    const pl = a.placement?.tiles[i];
    const cx = pl ? (pl.pos[0] + pl.w / 2) * k : t.pos_um[0] + t.w * t.pixel_size_nm / 2000;
    const cy = pl ? (pl.pos[1] + pl.h / 2) * k : t.pos_um[1] + t.h * t.pixel_size_nm / 2000;
    const d = Math.hypot(x - cx, y - cy);
    // u = sign * stage / 1000 with stage = st + sign * (x - cx) * 1000, and sign * sign = 1.
    const p: Pt = [a.stage_sign[0] * t.stage_nm[0] / 1000 + (x - cx), a.stage_sign[1] * t.stage_nm[1] / 1000 + (y - cy)];
    if (!best || d < best[0]) best = [d, p];
  });
  return best ? (best as [number, Pt])[1] : null;
}

export interface GroupRegion {
  name: string;
  color: string;
  regions: { area: string; points: Pt[]; hull: Pt[] }[];
  /** Polygon centroid of the largest padded region, in mount µm. */
  anchor: Pt;
  anchorArea: string;
}

/** Counter-clockwise convex hull, with duplicate and collinear interior points removed. */
function convexHull(points: Pt[]): Pt[] {
  const sorted = [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .filter((p, i, all) => !i || p[0] !== all[i - 1][0] || p[1] !== all[i - 1][1]);
  if (sorted.length < 3) return sorted;
  const cross = (a: Pt, b: Pt, c: Pt) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const half = (ps: Pt[]) => {
    const out: Pt[] = [];
    for (const p of ps) {
      while (out.length > 1 && cross(out[out.length - 2], out[out.length - 1], p) <= 0) out.pop();
      out.push(p);
    }
    return out;
  };
  return [...half(sorted).slice(0, -1), ...half(sorted.reverse()).slice(0, -1)];
}

/** Rounded offset of the hull: a 24-sided circle for one point, a capsule for two. */
function paddedHull(points: Pt[], pad: number): Pt[] {
  return convexHull(convexHull(points).flatMap(([x, y]) => Array.from({ length: 24 }, (_, i): Pt => {
    const angle = i * Math.PI / 12;
    return [x + pad * Math.cos(angle), y + pad * Math.sin(angle)];
  })));
}

function polygonCentre(points: Pt[]): { area: number; centre: Pt } {
  // Translate first so stage coordinates far from zero do not lose precision.
  const [ox, oy] = points[0];
  let twiceArea = 0, x = 0, y = 0;
  points.forEach((p, i) => {
    const q = points[(i + 1) % points.length];
    const px = p[0] - ox, py = p[1] - oy, qx = q[0] - ox, qy = q[1] - oy;
    const cross = px * qy - qx * py;
    twiceArea += cross; x += (px + qx) * cross; y += (py + qy) * cross;
  });
  return { area: Math.abs(twiceArea) / 2, centre: [ox + x / (3 * twiceArea), oy + y / (3 * twiceArea)] };
}

/** One rounded region per occupied area; never bridge the gap between two areas. */
export function groupRegions(groups: ReviewGroup[], spots: Pick<Spot, 'id' | 'area_id' | 'x_um' | 'y_um'>[], areas: StageArea[], layout: MountLayout): GroupRegion[] {
  const byId = new Map(spots.map(s => [s.id.toLowerCase(), s]));
  const byArea = new Map(areas.map(a => [a.id, a]));
  const pad = Math.max(40, 0.04 * Math.max(layout.bounds[2] - layout.bounds[0], layout.bounds[3] - layout.bounds[1]));
  return groups.flatMap(group => {
    const points = new Map<string, Pt[]>();
    for (const id of new Set(group.spots.map(id => id.toLowerCase()))) {
      const s = byId.get(id);
      if (!s || !Number.isFinite(s.x_um) || !Number.isFinite(s.y_um)) continue;
      const a = byArea.get(s.area_id);
      const p = a ? mosaicToMount(a, layout, s.x_um, s.y_um) : null;
      if (!p || !p.every(Number.isFinite)) continue;
      if (!points.has(s.area_id)) points.set(s.area_id, []);
      points.get(s.area_id)!.push(p);
    }
    const regions = [...points].map(([area, points]) => ({ area, points, hull: paddedHull(points, pad) }));
    if (!regions.length) return [];
    const largest = regions.map(r => ({ ...polygonCentre(r.hull), id: r.area })).reduce((a, b) => b.area > a.area ? b : a);
    return [{ name: group.name, color: group.color, regions, anchor: largest.centre, anchorArea: largest.id }];
  });
}

/** Inset pixels per mount µm and offsets: inset = (u - bounds) * k + offset, centred, `pad` px inside. */
export interface Fit { k: number; ox: number; oy: number }
export function fitTransform(bounds: Box, w: number, h: number, pad = 6): Fit {
  const bw = Math.max(bounds[2] - bounds[0], 1e-9), bh = Math.max(bounds[3] - bounds[1], 1e-9);
  const k = Math.max(0, Math.min((w - 2 * pad) / bw, (h - 2 * pad) / bh));
  return { k, ox: (w - bw * k) / 2, oy: (h - bh * k) / 2 };
}
export function applyFit(f: Fit, bounds: Box, p: Pt): Pt {
  return [f.ox + (p[0] - bounds[0]) * f.k, f.oy + (p[1] - bounds[1]) * f.k];
}

/** Height of the map for a width: the mount's aspect ratio, within 0.3 and 1.25 of the width. */
export const ASPECT_MIN = 0.3, ASPECT_MAX = 1.25;
export function mapAspect(bounds: Box): number {
  const bw = bounds[2] - bounds[0], bh = bounds[3] - bounds[1];
  return Math.min(ASPECT_MAX, Math.max(ASPECT_MIN, bw > 0 ? bh / bw : 1));
}

/** The viewer's visible quad as mount points (rotation kept), or null when it shows the whole area. */
export function viewQuad(view: View, umPerPx: number, mosaicPx: Pt, map: (x: number, y: number) => Pt | null): Pt[] | null {
  const [mw, mh] = mosaicPx;
  const all = ([[0, 0], [mw, 0], [mw, mh], [0, mh]] as Pt[]).every(([x, y]) => {
    const [sx, sy] = toScreen(view, x, y);
    return sx >= -0.5 && sy >= -0.5 && sx <= view.w + 0.5 && sy <= view.h + 0.5;
  });
  if (all) return null;
  const quad: Pt[] = [];
  for (const [sx, sy] of [[0, 0], [view.w, 0], [view.w, view.h], [0, view.h]]) {
    const [x, y] = toMosaic(view, sx, sy);
    const p = map(x * umPerPx, y * umPerPx);
    if (!p) return null;
    quad.push(p);
  }
  return quad;
}

// ---- size ------------------------------------------------------------------------------
// The viewer writes nothing, so the inset's size and collapsed state live in memory; UPb Lab
// remembers them (bridge v5.1: `inset` on set-presentation, `lab.viewer.inset` back).

export const MIN_W = 120, NORMAL_W = 180, ENLARGED_W = 360, STEP = 16, MAX_SENT_W = 1200;
/** Below this image size the inset starts collapsed (UPb Lab's smallest SEM tile is about 320 x 240 px). */
export const SMALL_W = 360, SMALL_H = 270;

export interface InsetState { collapsed: boolean; width: number }

/** Without a remembered inset: normal width, collapsed on a small image area. */
export function defaultInset(viewerW: number, viewerH: number): InsetState {
  return { collapsed: viewerW < SMALL_W || viewerH < SMALL_H, width: NORMAL_W };
}

/**
 * Widest map that fits: at most half the image area's width and, with its header, the
 * free height between the viewer controls and the status bar or legend. The minimum (120 px)
 * gives way to these limits, so the inset never covers them.
 */
export function maxWidth(viewerW: number, freeH: number, aspect: number, header: number): number {
  return Math.max(0, Math.min(viewerW * 0.5, (freeH - header) / aspect));
}
/** Width drawn: the wanted width within 120 px and `max` (the stored width is not changed). */
export function shownWidth(width: number, max: number): number {
  return Math.max(0, Math.round(Math.min(max, Math.max(MIN_W, width))));
}
/** Width the user set, as the shell stores it: an integer within 120 and 1200 px. */
export function storedWidth(width: number): number {
  return Math.round(Math.min(MAX_SENT_W, Math.max(MIN_W, width)));
}
