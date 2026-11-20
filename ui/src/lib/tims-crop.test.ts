import { afterEach, describe, expect, it, vi } from 'vitest';
import { PickListCrops } from './tims-crop';
import type { MosaicJson, ToneParams } from './core/types';
import { emptyImageEdits, identity } from './image-edits';

const disposables: PickListCrops[] = [];
afterEach(() => { for (const crops of disposables.splice(0)) crops.dispose(); vi.unstubAllGlobals(); });
function setup() {
  const contexts: { drawImage: ReturnType<typeof vi.fn>; transform: ReturnType<typeof vi.fn> }[] = [];
  const bitmaps: { width: number; height: number; data: Uint8ClampedArray; close: ReturnType<typeof vi.fn> }[] = [];
  vi.stubGlobal('ImageData', class { constructor(public data: Uint8ClampedArray, public width: number, public height: number) {} });
  vi.stubGlobal('createImageBitmap', vi.fn(async (data: ImageData) => { const bitmap = { ...data, width: data.width, height: data.height, close: vi.fn() }; bitmaps.push(bitmap); return bitmap; }));
  vi.stubGlobal('document', { createElement: vi.fn(() => {
    const ctx = { fillStyle: '', fillRect: vi.fn(), scale: vi.fn(), translate: vi.fn(), transform: vi.fn(), drawImage: vi.fn() };
    contexts.push(ctx);
    return { width: 0, height: 0, getContext: () => ctx, toDataURL: () => 'data:image/png;base64,local' };
  }) });
  const core = { pyramidTile: vi.fn(async (_area: string, _level: number, _col: number, _row: number) => ({ w: 256, h: 256, data: new Uint8Array(256 * 256).fill(100) })), toneLut: vi.fn(async (_tone: ToneParams) => Uint8Array.from({ length: 256 }, (_, i) => 255 - i)) };
  const crops = new PickListCrops(core); disposables.push(crops);
  return { crops, core, contexts, bitmaps };
}
const mosaic: MosaicJson = { w: 1024, h: 1024, um_per_px: 1, tile_size: 256, origin_um: [0, 0], sha256: '', format: 'png', levels: [{ level: 0, w: 1024, h: 1024, cols: 4, rows: 4, scale: 1 }, { level: 1, w: 512, h: 512, cols: 2, rows: 2, scale: 2 }] };
const tone = { gamma: 1.3 } as ToneParams;
describe('canvas pixels for pick-list crops', () => {
  it('shares the viewer tile cache, tone LUT and image levels across crops, and releases bitmaps', async () => {
    const { crops, core, bitmaps } = setup(), edits = { ...emptyImageEdits(), levels: { black: 50, white: 200 } };
    expect(await crops.image('area', [100, 100, 200, 200], mosaic, tone, edits, identity, 35)).toBe('data:image/png;base64,local');
    await crops.image('area', [120, 120, 220, 220], mosaic, tone, edits, identity, 35);
    expect(core.pyramidTile).toHaveBeenCalledTimes(1); expect(core.pyramidTile).toHaveBeenCalledWith('area', 0, 0, 0);
    expect(core.toneLut).toHaveBeenCalledTimes(1); expect(core.toneLut).toHaveBeenCalledWith(tone);
    const displayed = Math.round((155 - 50) * 255 / 150);
    expect([...bitmaps[0].data.slice(0, 4)]).toEqual([displayed, displayed, displayed, 255]);
    crops.dispose(); expect(bitmaps[0].close).toHaveBeenCalledTimes(1);
  });
  it('uses a bounded pyramid level for a large crop and applies the display frame transform', async () => {
    const { crops, core, contexts } = setup();
    await crops.image('area', [0, 0, 1024, 1024], mosaic, tone, emptyImageEdits(), [0, 1, -1, 0, 1024, 0], 35);
    expect(core.pyramidTile).toHaveBeenCalledTimes(4); expect(core.pyramidTile.mock.calls.every(call => call[1] === 1)).toBe(true);
    expect(contexts[0].transform).toHaveBeenCalledWith(0, 1, -1, 0, 1024, 0);
  });
  it('reports a missing local tile instead of printing an empty image', async () => {
    const { crops, core } = setup(); core.pyramidTile.mockRejectedValueOnce(new Error('Missing pixels'));
    await expect(crops.image('area', [100, 100, 200, 200], mosaic, tone, emptyImageEdits(), identity, 35)).rejects.toThrow('could not be loaded');
  });
});
