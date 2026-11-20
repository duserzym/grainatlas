import type { Core } from './core/core';
import type { AreaImageEdits, MosaicJson, SheetRect, ToneParams } from './core/types';
import { TileCache } from './canvas/tiles';
import { applyAffine, inverseAffine, levelsLut, pointBounds, type Affine } from './image-edits';

/** The exposed printBandImage is a strip preview and requires a sheet job. Use
 * the viewer's local pyramid loader and LUT instead, without starting a print job. */
export class PickListCrops {
  private tiles: TileCache;
  private luts = new Map<string, Uint8Array>();
  private core: Pick<Core, 'pyramidTile' | 'toneLut'>;
  constructor(core: Pick<Core, 'pyramidTile' | 'toneLut'>) { this.core = core; this.tiles = new TileCache(core); }
  dispose() { this.tiles.clear(); }
  async image(area: string, rect: SheetRect, mosaic: MosaicJson, tone: ToneParams, edits: AreaImageEdits, transform: Affine, paperSize: number): Promise<string> {
    if (!mosaic.levels.length) throw new Error('Grain image pyramid is unavailable');
    let lut = this.luts.get(area);
    if (!lut) { lut = levelsLut(await this.core.toneLut(tone), edits); this.luts.set(area, lut); }
    this.tiles.setLut(lut);
    const [x0, y0, x1, y1] = rect, rw = x1 - x0, rh = y1 - y0, u = mosaic.um_per_px;
    const target = Math.ceil(paperSize * 300 / 25.4);
    const resolution = Math.min(1 / u, target / Math.max(rw, rh));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.ceil(rw * resolution)); canvas.height = Math.max(1, Math.ceil(rh * resolution));
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('Grain crop canvas is unavailable');
    ctx.fillStyle = 'white'; ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.scale(canvas.width / rw, canvas.height / rh); ctx.translate(-x0, -y0); ctx.transform(...transform);
    const inv = inverseAffine(transform), bounds = pointBounds([[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(p => applyAffine(inv, p as [number, number])));
    const level = Math.min(mosaic.levels.length - 1, Math.max(0, Math.floor(Math.log2(1 / (resolution * u)))));
    const factor = 2 ** level, span = mosaic.tile_size * factor * u, info = mosaic.levels[level];
    const c0 = Math.max(0, Math.floor(bounds[0] / span)), c1 = Math.min(info.cols - 1, Math.ceil(bounds[2] / span) - 1);
    const r0 = Math.max(0, Math.floor(bounds[1] / span)), r1 = Math.min(info.rows - 1, Math.ceil(bounds[3] / span) - 1);
    for (let row = r0; row <= r1; row++) for (let col = c0; col <= c1; col++) {
      const bitmap = await this.tiles.ready(area, level, col, row);
      ctx.drawImage(bitmap, col * span, row * span, bitmap.width * factor * u, bitmap.height * factor * u);
    }
    return canvas.toDataURL('image/png');
  }
}
