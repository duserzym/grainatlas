import { isHosted } from '@lab/bridge';
import { current } from './store.svelte';
import { canWrite } from './presentation';
import { download } from './files';
import { bundlePixels } from './bundle-tone';
import { loadPublishedPixels } from './published-pixels';
import { exportOptions, writeGrainExport, writeFolderBatch, writeDirectoryFile, type ExportChoice, type ExportDirectory } from './grain-export';

export async function saveGrainExport(choice: ExportChoice, area: string | null, selected: string[]): Promise<void> {
  if (!canWrite()) return;
  const app = current();
  if (app.busy) return;
  // A mount opened from the server measures its published images: wait for the
  // background load (it shows its own progress) instead of failing in the core.
  if (app.remote && !bundlePixels(app) && !(await loadPublishedPixels(app)))
    throw new Error('The published images are not loaded, so the grains cannot be measured. Open the mount for editing and try again when the server can be reached.');
  if (app.busy) return;
  const core = await app.core;
  // Pick before running the job. Hosted downloads follow the existing files.ts path.
  let parent: ExportDirectory | null = null;
  if (core.kind === 'wasm' && !isHosted() && 'showDirectoryPicker' in window) {
    try { parent = await (window as unknown as { showDirectoryPicker(o: object): Promise<ExportDirectory> }).showDirectoryPicker({ mode: 'readwrite' }); }
    catch (error) { if (error instanceof DOMException && error.name === 'AbortError') return; throw error; }
  }
  const { liveIdle } = await import('./actions');
  for (const id of Object.keys(app.stitched)) await liveIdle(id, app);
  const project = app.fullProject();
  if (!project) return;
  const options = exportOptions(choice, area, selected, JSON.parse(JSON.stringify(app.tone)), new Date().toISOString());
  if (core.kind === 'tauri') {
    const where = await app.run('Exporting grains', p => core.saveGrainExport!(project, options, p));
    if (where) app.notice = `Saved ${where}`;
    return;
  }
  let cancelled = false;
  // Cooperative batches preserve published bundle sources when cancelling in wasm.
  app.cancelTask = () => { cancelled = true; };
  const name = `grain-export-${options.exported.replace(/[:.]/g, '-')}-${crypto.randomUUID().slice(0, 8)}`;
  try {
    const saved = await app.run('Exporting grains', async progress => {
      const dir = parent ? await parent.getDirectoryHandle(name, { create: true }) : null;
      const marker = new TextEncoder().encode('Incomplete grain export. Delete this folder if the export was cancelled or failed.\n');
      if (dir) await writeDirectoryFile(dir, 'INCOMPLETE.txt', marker);
      const parts: Blob[] = [];
      await writeGrainExport(core, project, options, {
        zipStream: !dir,
        write: async data => { if (dir) await writeFolderBatch(dir, data); else if (data.length) parts.push(new Blob([data as BlobPart])); },
        finish: async () => { if (dir) await dir.removeEntry('INCOMPLETE.txt'); else download(new Blob(parts, { type: 'application/zip' }), `${name}.zip`, 'application/zip'); },
      }, progress, () => cancelled);
      return true;
    });
    if (saved) app.notice = `Saved ${name}${parent ? '' : '.zip'}`;
  } finally { app.cancelTask = null; }
}
