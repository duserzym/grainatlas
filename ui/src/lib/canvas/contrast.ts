export type Contrast = 'dark' | 'light';

/** Display luma, with a dead band so a textured edge doesn't flicker. */
export function spotContrast(luma: number | null, previous: Contrast = 'light'): Contrast {
  if (luma == null || !Number.isFinite(luma)) return previous;
  if (luma > 0.55) return 'dark';
  if (luma < 0.45) return 'light';
  return previous;
}

/** Mean displayed RGB luma in a circumference ±2 CSS px (a disc for tiny rings).
 * Read only intersecting, canvas-clamped boxes, each at most 64×64 device pixels.
 * No SVG overlays are included. Null means the ring is entirely off canvas.
 */
export function ringLuminance(canvas: HTMLCanvasElement, sx: number, sy: number, r: number, dpr: number): number | null {
  if (![sx, sy, r, dpr].every(Number.isFinite) || r < 0 || dpr <= 0) return null;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const cx = sx * dpr, cy = sy * dpr;
  const inner2 = Math.max(0, (r - 2) * dpr) ** 2, outer = (r + 2) * dpr, outer2 = outer ** 2;
  const x0 = Math.max(0, Math.floor(cx - outer)), x1 = Math.min(canvas.width, Math.ceil(cx + outer));
  const y0 = Math.max(0, Math.floor(cy - outer)), y1 = Math.min(canvas.height, Math.ceil(cy + outer));
  let sum = 0, count = 0;
  try {
    for (let y = y0; y < y1; y += 64) for (let x = x0; x < x1; x += 64) {
      const w = Math.min(64, x1 - x), h = Math.min(64, y1 - y);
      const near2 = Math.max(x - cx, 0, cx - (x + w)) ** 2 + Math.max(y - cy, 0, cy - (y + h)) ** 2;
      const far2 = Math.max(Math.abs(x - cx), Math.abs(x + w - cx)) ** 2 + Math.max(Math.abs(y - cy), Math.abs(y + h - cy)) ** 2;
      if (near2 > outer2 || far2 < inner2) continue;
      const { data } = ctx.getImageData(x, y, w, h);
      for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) {
        const d2 = (x + i + 0.5 - cx) ** 2 + (y + j + 0.5 - cy) ** 2;
        if (d2 < inner2 || d2 > outer2) continue;
        const p = (j * w + i) * 4;
        sum += (0.2126 * data[p] + 0.7152 * data[p + 1] + 0.0722 * data[p + 2]) / 255;
        count++;
      }
    }
  } catch {
    // A tainted or unavailable canvas must never interrupt a placement gesture.
    return null;
  }
  return count ? sum / count : null;
}
