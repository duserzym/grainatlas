// Pure option and bounded browser-write handling, shared by the dialog and node tests.
import type { Core, GrainExportOptions } from './core/core';
import type { Project, Progress, ToneParams } from './core/types';
export type ExportScope = 'selected' | 'area' | 'mount';
export interface ExportChoice { scope: ExportScope; pixel_values: 'original' | 'displayed'; format: 'png' | 'tiff'; margin_um: number; include_crops: boolean; include_masks: boolean }
export function exportOptions(choice: ExportChoice, area: string | null, selected: string[], tone: Record<string, ToneParams>, exported: string): GrainExportOptions {
  if (!Number.isFinite(choice.margin_um) || choice.margin_um < 0) throw new Error('Margin must be at least 0 µm.');
  if (choice.scope === 'selected' && !selected.length) throw new Error('Select at least one grain.');
  if (choice.scope !== 'mount' && !area) throw new Error('Choose an area.');
  return { pixel_values: choice.pixel_values, format: choice.format, margin_um: choice.margin_um,
    include_crops: choice.include_crops, include_masks: choice.include_masks,
    selected: choice.scope === 'selected' ? [...selected] : null, area_id: choice.scope === 'mount' ? null : area,
    display_tone: structuredClone(tone), exported };
}
export interface ExportSink { zipStream: boolean; write(data: Uint8Array): Promise<void>; finish(): Promise<void> }
export async function writeGrainExport(core: Core, project: Project, options: GrainExportOptions, sink: ExportSink, progress: (p: Progress) => void, cancelled: () => boolean): Promise<void> {
  if (!core.beginGrainExport || !core.grainExportBatch || !core.abortGrainExport) throw new Error('Grain export is unavailable.');
  const check = () => { if (cancelled()) throw new Error('cancelled'); };
  try {
    check(); await core.beginGrainExport(project, options);
    for (;;) {
      check();
      const batch = await core.grainExportBatch(sink.zipStream, progress);
      check(); await sink.write(batch.data); check();
      if (batch.done) break;
    }
    await sink.finish();
  } finally { await core.abortGrainExport(); }
}
export interface ExportDirectory {
  getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<ExportDirectory>;
  getFileHandle(name: string, options?: { create?: boolean }): Promise<{ createWritable(): Promise<{ write(data: BufferSource): Promise<void>; close(): Promise<void>; abort?(): Promise<void> }> }>;
  removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
}
export async function writeDirectoryFile(root: ExportDirectory, name: string, data: Uint8Array): Promise<void> {
  const parts = name.split('/');
  if (parts.some(p => !p || p === '..' || p === '.' || p.includes('\\'))) throw new Error('Invalid export file path.');
  let dir = root;
  for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part, { create: true });
  const stream = await (await dir.getFileHandle(parts.at(-1)!, { create: true })).createWritable();
  try { await stream.write(data as BufferSource); await stream.close(); }
  catch (error) { await stream.abort?.(); throw error; }
}
/** Batches use the core's STORE ZIP format. Process views of one batch, then release it. */
export async function writeFolderBatch(root: ExportDirectory, data: Uint8Array): Promise<void> {
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const dec = new TextDecoder();
  let at = 0;
  while (at + 4 <= data.length && dv.getUint32(at, true) === 0x04034b50) {
    if (at + 30 > data.length || dv.getUint16(at + 8, true) !== 0) throw new Error('Invalid grain export ZIP batch.');
    const size = dv.getUint32(at + 18, true), n = dv.getUint16(at + 26, true), extra = dv.getUint16(at + 28, true);
    const start = at + 30 + n + extra;
    if (start + size > data.length) throw new Error('Truncated grain export ZIP batch.');
    await writeDirectoryFile(root, dec.decode(data.subarray(at + 30, at + 30 + n)), data.subarray(start, start + size));
    at = start + size;
  }
  if (at + 4 > data.length || ![0x02014b50, 0x06054b50].includes(dv.getUint32(at, true))) throw new Error('Invalid grain export ZIP batch.');
}
