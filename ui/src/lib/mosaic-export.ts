import type { Core, MosaicExportChunk, MosaicExportOptions, MosaicExportPlan } from './core/core';
import type { Project, Progress } from './core/types';
export const MOSAIC_BLOB_LIMIT = 512 * 1024 * 1024;
export function mosaicCanStream(picker: { showDirectoryPicker?: unknown; showSaveFilePicker?: unknown }): boolean {
  return typeof picker.showDirectoryPicker === 'function' || typeof picker.showSaveFilePicker === 'function';
}
export function mosaicEstimate(width: number, height: number, bits: number, channels = 1): { bytes: number; big: boolean } {
  if (![width, height].every(v => Number.isSafeInteger(v) && v > 0 && v <= 0xffffffff) || ![8, 16].includes(bits) || ![1, 3].includes(channels)) throw new Error('Invalid mosaic dimensions or sample type.');
  const tiles = Math.ceil(width / 512) * Math.ceil(height / 512), pixels = tiles * 512 * 512 * channels * bits / 8;
  const big = pixels + tiles * 16 + 1048576 >= 3500000000;
  const bytes = pixels + tiles * (big ? 16 : 8) + 1048576;
  if (!Number.isSafeInteger(bytes)) throw new Error('Mosaic is too large.');
  return { bytes, big };
}
export function mosaicName(name: string): string {
  let s = [...name].slice(0, 100).map(c => /[\p{Cc}<>:"/\\|?*]/u.test(c) ? '_' : c).join('').replace(/^[ .]+|[ .]+$/g, '') || 'mosaic';
  if (/^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|$)/i.test(s)) s = `_${s}`;
  return s;
}
export const MOSAIC_RASTER_LIMIT = 65535;
export function mosaicFitFactor(width: number, height: number): number {
  if (![width, height].every(v => Number.isSafeInteger(v) && v > 0 && v <= 0xffffffff)) throw new Error('Invalid mosaic size.');
  let factor = 1; while (Math.ceil(width / factor) > MOSAIC_RASTER_LIMIT || Math.ceil(height / factor) > MOSAIC_RASTER_LIMIT) factor *= 2;
  return factor;
}
export function mosaicFilename(mount: string, area: string | null, format: 'tiff' | 'png' | 'jpg'): string {
  const part = (name: string) => { let out = ''; for (const c of mosaicName(name)) { if (new TextEncoder().encode(out + c).length > 80) break; out += c; } return out; };
  return `${part(mount)}_${area === null ? 'mount' : part(area)}_mosaic.${format}`;
}
export function mosaicRasterEstimate(width: number, height: number, bits: number, format: 'png' | 'jpg'): number {
  mosaicFitFactor(width, height); if (![8, 16].includes(bits) || format === 'jpg' && bits !== 8) throw new Error('Invalid image depth.');
  return width * height * (bits / 8) * (format === 'jpg' ? 8 : 2) + 1048576;
}
export function mosaicBytes(plan: MosaicExportPlan): number { return plan.images.reduce((sum, a) => sum + a.estimated_bytes, 0); }
export function sizeLabel(bytes: number): string { return bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(2)} GiB` : `${(bytes / 1024 ** 2).toFixed(1)} MiB`; }
export interface MosaicSink { write(chunk: MosaicExportChunk): Promise<void>; finish(): Promise<void>; abort(): Promise<void> }
/** Backpressure: only one worker chunk and one pending write are live. */
export async function writeMosaicExport(core: Core, project: Project, options: MosaicExportOptions, sink: MosaicSink, progress: (p: Progress) => void, cancelled: () => boolean, expected?: MosaicExportPlan): Promise<void> {
  if (!core.beginMosaicExport || !core.mosaicExportNext || !core.abortMosaicExport) throw new Error('Mosaic export is unavailable.');
  const check = () => { if (cancelled()) throw new Error('cancelled'); };
  try {
    check(); const plan = await core.beginMosaicExport(project, options);
    if (expected && JSON.stringify(plan) !== JSON.stringify(expected)) throw new Error('The mosaic changed. Review the estimate and export again.');
    for (;;) {
      check(); const chunk = await core.mosaicExportNext(); check();
      if (chunk.data) await sink.write(chunk);
      const unit = options.format && options.format !== 'tiff' ? 'strips' : 'tiles';
      progress({ stage: `Exporting mosaic: ${chunk.tiles_done} / ${chunk.tiles_total} ${unit}`, fraction: chunk.tiles_total ? chunk.tiles_done / chunk.tiles_total : 0 });
      check(); if (chunk.done) break;
    }
    await sink.finish();
  } catch (error) {
    try { await sink.abort(); } catch (cleanup) { throw new Error(`${error instanceof Error ? error.message : error}; cleanup failed: ${cleanup instanceof Error ? cleanup.message : cleanup}`); }
    throw error;
  } finally { await core.abortMosaicExport(); }
}
export interface MosaicWritable { write(data: Uint8Array): Promise<void>; close(): Promise<void>; abort(): Promise<void> }
export interface MosaicDirectory {
  getDirectoryHandle(name: string, options: { create: boolean }): Promise<MosaicDirectory>;
  getFileHandle(name: string, options: { create: boolean }): Promise<{ createWritable(): Promise<MosaicWritable> }>;
  removeEntry(name: string, options?: { recursive: boolean }): Promise<void>;
}
export function mosaicFolderSink(dir: MosaicDirectory, cleanup: () => Promise<void>): MosaicSink {
  let stream: MosaicWritable | null = null, name = '';
  return {
    async write(chunk) {
      if (!chunk.name || !chunk.data || /[/\\]/.test(chunk.name) || ['.', '..'].includes(chunk.name)) throw new Error('Invalid mosaic filename.');
      if (!stream) { name = chunk.name; stream = await (await dir.getFileHandle(name, { create: true })).createWritable(); }
      if (name !== chunk.name) throw new Error('Unfinished mosaic file.');
      await stream.write(chunk.data);
      if (chunk.last) { await stream.close(); stream = null; }
    },
    async finish() { if (stream) throw new Error('Unfinished mosaic file.'); },
    async abort() { try { await stream?.abort(); } finally { stream = null; await cleanup(); } },
  };
}
// ZIP64 STORE streaming. TIFF and sidecars stay together, even with only a save
// picker. CRC and offsets cover the chunk stream; no image is assembled here.
const crcTable = Uint32Array.from({ length: 256 }, (_, i) => { for (let k = 0; k < 8; k++) i = i & 1 ? 0xedb88320 ^ (i >>> 1) : i >>> 1; return i >>> 0; });
function record(size: number, values: [number, number, number | bigint][]): Uint8Array {
  const b = new Uint8Array(size), v = new DataView(b.buffer);
  for (const [at, bytes, n] of values) { if (bytes === 8) v.setBigUint64(at, BigInt(n), true); else if (bytes === 4) v.setUint32(at, Number(n), true); else v.setUint16(at, Number(n), true); }
  return b;
}
export function mosaicZipSink(write: (b: Uint8Array) => Promise<void>, close: () => Promise<void>, abort: () => Promise<void>): MosaicSink {
  type Entry = { name: Uint8Array; offset: bigint; size: bigint; crc: number };
  let offset = 0n, entry: Entry | null = null;
  const entries: Entry[] = [];
  const put = async (b: Uint8Array) => { await write(b); offset += BigInt(b.length); };
  return {
    async write(chunk) {
      if (!chunk.name || !chunk.data) throw new Error('Missing mosaic data.');
      if (!entry) {
        const name = new TextEncoder().encode(chunk.name); entry = { name, offset, size: 0n, crc: 0xffffffff };
        await put(record(30, [[0,4,0x04034b50],[4,2,45],[6,2,0x0808],[12,2,0x21],[18,4,0xffffffff],[22,4,0xffffffff],[26,2,name.length],[28,2,20]]));
        await put(name); await put(record(20, [[0,2,1],[2,2,16],[4,8,0n],[12,8,0n]]));
      }
      if (new TextDecoder().decode(entry.name) !== chunk.name) throw new Error('Unfinished ZIP entry.');
      // Keep cancel and paint events responsive even for wide PNG strips.
      for (let start = 0; start < chunk.data.length; start += 262144) {
        for (const b of chunk.data.subarray(start, start + 262144)) entry.crc = crcTable[(entry.crc ^ b) & 255] ^ (entry.crc >>> 8);
        if (start + 262144 < chunk.data.length) await new Promise<void>(resolve => setTimeout(resolve, 0));
      }
      entry.size += BigInt(chunk.data.length); await put(chunk.data);
      if (chunk.last) { entry.crc = (entry.crc ^ 0xffffffff) >>> 0; await put(record(24, [[0,4,0x08074b50],[4,4,entry.crc],[8,8,entry.size],[16,8,entry.size]])); entries.push(entry); entry = null; }
    },
    async finish() {
      if (entry) throw new Error('Unfinished ZIP entry.');
      const central = offset;
      for (const e of entries) {
        await put(record(46, [[0,4,0x02014b50],[4,2,45],[6,2,45],[8,2,0x0808],[14,2,0x21],[16,4,e.crc],[20,4,0xffffffff],[24,4,0xffffffff],[28,2,e.name.length],[30,2,28],[42,4,0xffffffff]]));
        await put(e.name); await put(record(28, [[0,2,1],[2,2,24],[4,8,e.size],[12,8,e.size],[20,8,e.offset]]));
      }
      const size = offset - central, end = offset;
      await put(record(56, [[0,4,0x06064b50],[4,8,44n],[12,2,45],[14,2,45],[24,8,BigInt(entries.length)],[32,8,BigInt(entries.length)],[40,8,size],[48,8,central]]));
      await put(record(20, [[0,4,0x07064b50],[8,8,end],[16,4,1]]));
      await put(record(22, [[0,4,0x06054b50],[8,2,0xffff],[10,2,0xffff],[12,4,0xffffffff],[16,4,0xffffffff]])); await close();
    },
    abort,
  };
}
