// Pyramid tile cache: raw grey tiles from the core, display bitmaps with the
// black / white / gamma LUT applied. LUT changes re-map cached raw tiles
// without another trip to the core. `clear()` starts a new generation: a
// request still in flight from before (old pixels) is dropped when it lands,
// so a re-tone can never leave stale tiles behind.

import type { Core, RawImage } from '../core/core';

export const BITMAP_BUDGET = 96 * 1024 * 1024;
export const RAW_BUDGET = 32 * 1024 * 1024;

export class TileCache {
  private raw = new Map<string, RawImage>();
  private bmp = new Map<string, ImageBitmap>();
  private inflight = new Map<string, Promise<void>>();
  private lut: Uint8Array = identity();
  private lutKey = 'id';
  private gen = 0;
  private bitmapBytes = 0;
  private rawBytes = 0;
  onLoad: () => void = () => {};

  private core: Pick<Core, 'pyramidTile'>;
  private bitmapBudget: number;
  private rawBudget: number;

  constructor(core: Pick<Core, 'pyramidTile'>, bitmapBudget = BITMAP_BUDGET, rawBudget = RAW_BUDGET) {
    this.core = core;
    this.bitmapBudget = bitmapBudget;
    this.rawBudget = rawBudget;
  }

  memoryReport() {
    return { bitmapBytes: this.bitmapBytes, rawBytes: this.rawBytes };
  }

  /** Only use already cached tiles for coarser-level fallback. Never request a level here. */
  peek(area: string, level: number, col: number, row: number): ImageBitmap | null {
    const k = `${area}/${level}/${col}/${row}`;
    const b = this.bmp.get(k);
    if (!b) return null;
    this.bmp.delete(k);
    this.bmp.set(k, b);
    const r = this.raw.get(k);
    if (r) { this.raw.delete(k); this.raw.set(k, r); }
    return b;
  }

  setLut(lut: Uint8Array) {
    const key = Array.from(lut.subarray(0, 256)).join(',');
    if (key === this.lutKey) return;
    this.lut = lut;
    this.lutKey = key;
    for (const b of this.bmp.values()) b.close();
    this.bmp.clear();
    this.bitmapBytes = 0;
    this.onLoad();
  }

  clear() {
    this.gen++;
    for (const b of this.bmp.values()) b.close();
    this.bmp.clear();
    this.bitmapBytes = 0;
    this.raw.clear();
    this.rawBytes = 0;
    this.inflight.clear();
  }

  get(area: string, level: number, col: number, row: number): ImageBitmap | null {
    const k = `${area}/${level}/${col}/${row}`;
    const b = this.peek(area, level, col, row);
    if (b) return b;
    if (!this.inflight.has(k)) {
      const p: Promise<void> = this.load(k, area, level, col, row).finally(() => {
        if (this.inflight.get(k) === p) this.inflight.delete(k);
      });
      this.inflight.set(k, p);
    }
    return null;
  }

  private async load(k: string, area: string, level: number, col: number, row: number) {
    const gen = this.gen;
    let r = this.raw.get(k);
    if (!r) {
      try {
        r = await this.core.pyramidTile(area, level, col, row);
      } catch {
        return;
      }
      if (gen !== this.gen) return;
      this.rawBytes += r.w * r.h;
      this.raw.set(k, r);
      while (this.rawBytes > this.rawBudget) {
        const first = this.raw.keys().next().value!;
        const old = this.raw.get(first)!;
        this.rawBytes -= old.w * old.h;
        this.raw.delete(first);
      }
    }
    if (this.raw.has(k)) { this.raw.delete(k); this.raw.set(k, r); }
    const lut = this.lut;
    const rgba = new Uint8ClampedArray(r.w * r.h * 4);
    const d = r.data;
    for (let i = 0, j = 0; i < d.length; i++, j += 4) {
      const v = lut[d[i]];
      rgba[j] = v;
      rgba[j + 1] = v;
      rgba[j + 2] = v;
      rgba[j + 3] = 255;
    }
    const bmp = await createImageBitmap(new ImageData(rgba, r.w, r.h));
    if (lut !== this.lut || gen !== this.gen) {
      bmp.close();
      if (gen === this.gen) this.onLoad();
      return;
    }
    this.bmp.set(k, bmp);
    this.bitmapBytes += bmp.width * bmp.height * 4;
    while (this.bitmapBytes > this.bitmapBudget) {
      const first = this.bmp.keys().next().value!;
      const old = this.bmp.get(first)!;
      this.bitmapBytes -= old.width * old.height * 4;
      old.close();
      this.bmp.delete(first);
    }
    this.onLoad();
  }
}

function identity(): Uint8Array {
  const a = new Uint8Array(256);
  for (let i = 0; i < 256; i++) a[i] = i;
  return a;
}
