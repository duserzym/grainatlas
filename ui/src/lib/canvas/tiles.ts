// Pyramid tile cache: raw grey tiles from the core, display bitmaps with the
// black / white / gamma LUT applied. LUT changes re-map cached raw tiles
// without another trip to the core. `clear()` starts a new generation: a
// request still in flight from before (old pixels) is dropped when it lands,
// so a re-tone can never leave stale tiles behind.

import type { Core, RawImage } from '../core/core';

const MAX_TILES = 600;

export class TileCache {
  private raw = new Map<string, RawImage>();
  private bmp = new Map<string, ImageBitmap>();
  private inflight = new Map<string, Promise<void>>();
  private lut: Uint8Array = identity();
  private lutKey = 'id';
  private gen = 0;
  onLoad: () => void = () => {};

  constructor(private core: Core) {}

  setLut(lut: Uint8Array) {
    const key = Array.from(lut.subarray(0, 256)).join(',');
    if (key === this.lutKey) return;
    this.lut = lut;
    this.lutKey = key;
    for (const b of this.bmp.values()) b.close();
    this.bmp.clear();
    this.onLoad();
  }

  clear() {
    this.gen++;
    for (const b of this.bmp.values()) b.close();
    this.bmp.clear();
    this.raw.clear();
    this.inflight.clear();
  }

  get(area: string, level: number, col: number, row: number): ImageBitmap | null {
    const k = `${area}/${level}/${col}/${row}`;
    const b = this.bmp.get(k);
    if (b) {
      // LRU touch
      this.bmp.delete(k);
      this.bmp.set(k, b);
      return b;
    }
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
      this.raw.set(k, r);
    }
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
      return;
    }
    this.bmp.set(k, bmp);
    while (this.bmp.size > MAX_TILES) {
      const first = this.bmp.keys().next().value as string;
      this.bmp.get(first)?.close();
      this.bmp.delete(first);
    }
    while (this.raw.size > MAX_TILES * 2) {
      this.raw.delete(this.raw.keys().next().value as string);
    }
    this.onLoad();
  }
}

function identity(): Uint8Array {
  const a = new Uint8Array(256);
  for (let i = 0; i < 256; i++) a[i] = i;
  return a;
}
