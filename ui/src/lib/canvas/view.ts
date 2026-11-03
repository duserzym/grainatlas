// View transform of the infinite canvas: mosaic pixel → screen CSS pixel.
// screen = R(rot) · (p − c) · zoom + (W/2, H/2)

export interface View {
  cx: number;
  cy: number;
  zoom: number; // screen px per mosaic px
  rot: number; // radians
  w: number; // viewport CSS px
  h: number;
}

export function toScreen(v: View, x: number, y: number): [number, number] {
  const c = Math.cos(v.rot), s = Math.sin(v.rot);
  const dx = (x - v.cx) * v.zoom, dy = (y - v.cy) * v.zoom;
  return [c * dx - s * dy + v.w / 2, s * dx + c * dy + v.h / 2];
}

export function toMosaic(v: View, sx: number, sy: number): [number, number] {
  const c = Math.cos(v.rot), s = Math.sin(v.rot);
  const dx = sx - v.w / 2, dy = sy - v.h / 2;
  return [(c * dx + s * dy) / v.zoom + v.cx, (-s * dx + c * dy) / v.zoom + v.cy];
}

/** Canvas/SVG matrix (a b c d e f) for this view, times `k` (device pixel ratio for canvas). */
export function matrix(v: View, k = 1): [number, number, number, number, number, number] {
  const c = Math.cos(v.rot) * v.zoom, s = Math.sin(v.rot) * v.zoom;
  return [k * c, k * s, -k * s, k * c, k * (v.w / 2 - (c * v.cx - s * v.cy)), k * (v.h / 2 - (s * v.cx + c * v.cy))];
}

/** Zoom about a screen point, keeping the mosaic point under it fixed. */
export function zoomAt(v: View, sx: number, sy: number, factor: number, min = 1 / 64, max = 64): View {
  const z = Math.min(max, Math.max(min, v.zoom * factor));
  const [mx, my] = toMosaic(v, sx, sy);
  const nv = { ...v, zoom: z };
  const [ax, ay] = toMosaic(nv, sx, sy);
  return { ...nv, cx: nv.cx + (mx - ax), cy: nv.cy + (my - ay) };
}

/** Pan by a screen-space delta. */
export function panBy(v: View, dx: number, dy: number): View {
  const c = Math.cos(v.rot), s = Math.sin(v.rot);
  return { ...v, cx: v.cx - (c * dx + s * dy) / v.zoom, cy: v.cy - (-s * dx + c * dy) / v.zoom };
}

/** Rotate about a screen point. */
export function rotateAt(v: View, sx: number, sy: number, dRot: number): View {
  const [mx, my] = toMosaic(v, sx, sy);
  const nv = { ...v, rot: v.rot + dRot };
  const [ax, ay] = toMosaic(nv, sx, sy);
  return { ...nv, cx: nv.cx + (mx - ax), cy: nv.cy + (my - ay) };
}

export function fit(w: number, h: number, mw: number, mh: number, rot = 0): View {
  const zoom = Math.min(w / mw, h / mh) * 0.94;
  return { cx: mw / 2, cy: mh / 2, zoom: zoom > 0 && Number.isFinite(zoom) ? zoom : 1, rot, w, h };
}
