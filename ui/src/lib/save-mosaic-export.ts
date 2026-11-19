import { current } from './store.svelte';
import { download } from './files';
import type { Core, MosaicExportOptions, MosaicExportPlan } from './core/core';
import type { Project } from './core/types';
import { MOSAIC_BLOB_LIMIT, mosaicBytes, mosaicFolderSink, mosaicZipSink, writeMosaicExport, type MosaicDirectory, type MosaicWritable, type MosaicSink } from './mosaic-export';
export async function saveMosaicExport(core: Core, project: Project, options: MosaicExportOptions, plan: MosaicExportPlan): Promise<boolean> {
  const app = current(); if (app.busy) return false;
  let cancelled = false;
  if (core.kind === 'tauri') {
    app.cancelTask = () => { cancelled = true; core.cancelMosaicExport?.(); };
    try { const path = await app.run('Exporting mosaic', progress => core.saveMosaicExport!(project, options, progress)); if (path) app.notice = `Saved ${path}`; return !!path; }
    finally { app.cancelTask = null; }
  }
  const name = `mosaic-export-${new Date().toISOString().replace(/[:.]/g, '-')}-${crypto.randomUUID().slice(0, 8)}`;
  const picker = window as unknown as {
    showDirectoryPicker?: (options: object) => Promise<MosaicDirectory>;
    showSaveFilePicker?: (options: object) => Promise<{ createWritable(): Promise<MosaicWritable> }>;
  };
  let sink: MosaicSink;
  // Pick synchronously from the click, before worker work consumes user activation.
  try {
    if (picker.showDirectoryPicker) {
      const parent = await picker.showDirectoryPicker({ mode: 'readwrite' });
      const dir = await parent.getDirectoryHandle(name, { create: true });
      sink = mosaicFolderSink(dir, () => parent.removeEntry(name, { recursive: true }));
    } else if (picker.showSaveFilePicker) {
      const file = await picker.showSaveFilePicker({ suggestedName: `${name}.zip`, types: [{ description: 'Mosaic images and coordinates', accept: { 'application/zip': ['.zip'] } }] });
      const stream = await file.createWritable();
      sink = mosaicZipSink(b => stream.write(b), () => stream.close(), () => stream.abort());
    } else {
      if (mosaicBytes(plan) >= MOSAIC_BLOB_LIMIT) throw new Error('This browser can download mosaics under 512 MiB. Use the desktop app or Chromium for larger exports.');
      const parts: Blob[] = []; let bytes = 0;
      sink = mosaicZipSink(async b => { bytes += b.length; if (bytes >= MOSAIC_BLOB_LIMIT) throw new Error('The mosaic download reached the 512 MiB browser limit.'); parts.push(new Blob([b as BlobPart])); }, async () => { download(new Blob(parts, { type: 'application/zip' }), `${name}.zip`, 'application/zip'); parts.length = 0; }, async () => { parts.length = 0; });
    }
  } catch (error) { if (error instanceof DOMException && error.name === 'AbortError') return false; throw error; }
  app.cancelTask = () => { cancelled = true; };
  try {
    const saved = await app.run('Exporting mosaic', async progress => { await writeMosaicExport(core, project, options, sink, progress, () => cancelled, plan); return true; });
    if (saved) app.notice = `Saved ${name}`;
    return !!saved;
  } finally { app.cancelTask = null; }
}
