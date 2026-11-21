import type { Spot } from './core/types';
import type { View } from './canvas/view';
import { toScreen } from './canvas/view.ts';
import type { Box } from './mount-inset';

/** Mosaic µm, restricted to the clicked area (the summary may span the whole mount). */
export function regionBounds(spots: Pick<Spot, 'id' | 'area_id' | 'x_um' | 'y_um'>[], ids: readonly string[], area: string): Box | null {
  const wanted = new Set(ids.map(id => id.toLowerCase()));
  let box: Box | null = null;
  for (const s of spots) {
    if (s.area_id !== area || !wanted.has(s.id.toLowerCase()) || !Number.isFinite(s.x_um) || !Number.isFinite(s.y_um)) continue;
    if (!box) box = [s.x_um, s.y_um, s.x_um, s.y_um];
    else { box[0] = Math.min(box[0], s.x_um); box[1] = Math.min(box[1], s.y_um); box[2] = Math.max(box[2], s.x_um); box[3] = Math.max(box[3], s.y_um); }
  }
  return box;
}

/** Fifteen percent on each side, at least 300 µm across; also keep a single spot legible. */
export function regionFitRect([x0, y0, x1, y1]: Box): Box {
  const cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
  const hw = Math.max(300, (x1 - x0) * 1.3) / 2, hh = Math.max(300, (y1 - y0) * 1.3) / 2;
  return [cx - hw, cy - hh, cx + hw, cy + hh];
}

/** Fit in the current rotation/image orientation, capped at centerOnUm's default framing. */
export function regionCamera(rect: Box, view: View, umPerPx: number): { x: number; y: number; zoom: number } {
  const [x0, y0, x1, y1] = rect.map(n => n / umPerPx);
  const unit = { ...view, zoom: 1 };
  const corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(([x, y]) => toScreen(unit, x, y));
  const w = Math.max(...corners.map(p => p[0])) - Math.min(...corners.map(p => p[0]));
  const h = Math.max(...corners.map(p => p[1])) - Math.min(...corners.map(p => p[1]));
  const defaultZoom = Math.max(view.zoom, Math.min(view.w, view.h) / (200 / umPerPx) / 3);
  return { x: (x0 + x1) / 2, y: (y0 + y1) / 2, zoom: Math.min(view.w / w, view.h / h, defaultZoom) };
}
