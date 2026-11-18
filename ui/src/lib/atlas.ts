import { neutralTone, storedBundleTones } from './bundle-tone';
import { canWrite } from './presentation';
import { keepPrintEdits } from './strip-edit';
// Atlas bundles (upblab/docs/WORKFLOW.md section 3.1): save as a `<project_id>.atlas/`
// folder or a zip, publish to UPbDB. The core writes the bytes (identical in the browser
// and the desktop app); this file only moves them.

import { beforeCheckedAction } from './checks';
import { token as shellToken } from '@lab/bridge';
import type { BundleOptions, Project, Published } from './core/types';
import { download } from './files';
import { normalizeServer, type AtlasPublishOut } from './upbdb';
import { uploadImages, type PublishBundle, type PublishJob } from './publish-engine';
import type { AtlasJson } from './remote';
import { pixelSignature } from './sync-pixels';
import { notifyUpbLab } from './upblab-notice';
import { current, type AppState } from './store.svelte';

/** Options for the core from Settings: tile quality, the package's place on the lab storage. */
export function bundleOptions(p: Project, app: AppState = current()): BundleOptions {
  const root = app.link.nasRoot.trim().replace(/[\\/]+$/, '');
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/';
  return { quality: app.link.quality, exported: new Date().toISOString(), nas_path: root ? `${root}${sep}${p.package.name}` : null };
}

/** The bearer token: the shell's when hosted, else the one pasted in Settings. */
export async function authToken(app: AppState = current(), server: string | null = app.remote?.server ?? app.server): Promise<string | null> {
  try {
    const t = await shellToken();
    if (t) return t;
  } catch {
    /* not hosted */
  }
  return app.sessionToken && normalizeServer(app.sessionServer) === normalizeServer(server) ? app.sessionToken : app.link.token.trim() || null;
}

/** Entries of a STORE-only zip (what the core writes). */
export function unzipStored(z: Uint8Array): { name: string; data: Uint8Array }[] {
  const dv = new DataView(z.buffer, z.byteOffset, z.byteLength);
  let end = z.length - 22;
  while (end >= 0 && dv.getUint32(end, true) !== 0x06054b50) end--;
  if (end < 0) throw new Error('not a zip');
  const n = dv.getUint16(end + 10, true);
  let at = dv.getUint32(end + 16, true);
  const out: { name: string; data: Uint8Array }[] = [];
  const dec = new TextDecoder();
  for (let i = 0; i < n; i++) {
    const size = dv.getUint32(at + 20, true);
    const nameLen = dv.getUint16(at + 28, true);
    const skip = nameLen + dv.getUint16(at + 30, true) + dv.getUint16(at + 32, true);
    const local = dv.getUint32(at + 42, true);
    const name = dec.decode(z.subarray(at + 46, at + 46 + nameLen));
    const start = local + 30 + dv.getUint16(local + 26, true) + dv.getUint16(local + 28, true);
    out.push({ name, data: z.subarray(start, start + size) });
    at += 46 + skip;
  }
  return out;
}

export async function sha256Hex(b: Uint8Array): Promise<string> {
  if (!globalThis.crypto?.subtle) throw new Error('Image checksums need a secure connection.');
  const d = await crypto.subtle.digest('SHA-256', b as BufferSource);
  return Array.from(new Uint8Array(d), (x) => x.toString(16).padStart(2, '0')).join('');
}

/** True where the browser can write a folder (Chromium's File System Access API). */
export function canSaveFolder(): boolean {
  return typeof window !== 'undefined' && 'showDirectoryPicker' in window;
}

async function bundleZip(app: AppState, projectUuid?: string, options?: BundleOptions, background = false): Promise<{ project: Project; zip: Uint8Array } | null> {
  const { liveIdle } = await import('./actions');
  for (const id of Object.keys(app.stitched)) await liveIdle(id, app);
  const p = app.fullProject();
  if (!p) return null;
  if (projectUuid) p.uuid = projectUuid;
  const core = await app.core;
  const build = () => core.atlasZip(p, options ?? bundleOptions(p, app));
  const zip = background ? await build() : await app.run('Building the atlas bundle', build);
  return zip ? { project: p, zip } : null;
}

/** Saves the bundle: a zip, or a `<project_id>.atlas/` folder. */
export async function saveAtlas(asZip: boolean) {
  if (!canWrite()) return;
  const app = current();
  const core = await app.core;
  const p = app.fullProject();
  if (!p) return;
  if (core.kind === 'tauri') {
    const where = await app.run('Saving the atlas bundle', () => core.saveAtlas(p, bundleOptions(p), asZip));
    if (where) app.notice = `Saved ${where}`;
    return;
  }
  const b = await bundleZip(app);
  if (!b) return;
  if (asZip || !canSaveFolder()) {
    download(b.zip, `${p.uuid}.atlas.zip`, 'application/zip');
    return;
  }
  type Dir = { getDirectoryHandle(n: string, o?: { create?: boolean }): Promise<Dir>; getFileHandle(n: string, o?: { create?: boolean }): Promise<{ createWritable(): Promise<{ write(d: BufferSource): Promise<void>; close(): Promise<void> }> }> };
  let parent: Dir;
  try {
    parent = await (window as unknown as { showDirectoryPicker(o?: object): Promise<Dir> }).showDirectoryPicker({ mode: 'readwrite' });
  } catch {
    return; // cancelled
  }
  if (!canWrite()) return;
  await app.run('Writing the atlas folder', async () => {
    for (const { name, data } of unzipStored(b.zip)) {
      if (!canWrite()) return;
      const parts = name.split('/');
      let dir = parent;
      for (const d of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(d, { create: true });
      const w = await (await dir.getFileHandle(parts[parts.length - 1], { create: true })).createWritable();
      await w.write(data as BufferSource);
      await w.close();
    }
  });
  app.notice = `Saved the folder ${p.uuid}.atlas`;
}

/** Build in the core's worker/native task, with asynchronous digest checks between files. */
export async function backgroundBundle(app: AppState, projectUuid: string | undefined, exported: string): Promise<(PublishBundle & { project: Project }) | null> {
  const p = app.fullProject();
  if (!p) return null;
  const b = await bundleZip(app, projectUuid, { ...bundleOptions(p, app), exported }, true);
  if (!b) return null;
  const entries = unzipStored(b.zip);
  const manifest = entries.find(f => f.name.endsWith('/atlas.json') || f.name === 'atlas.json');
  if (!manifest) throw new Error('The bundle has no atlas.json.');
  const prefix = manifest.name.slice(0, -'atlas.json'.length);
  const files = entries.map(f => ({ path: f.name.slice(prefix.length), bytes: f.data }));
  const atlas = JSON.parse(new TextDecoder().decode(manifest.data)) as AtlasJson;
  const byPath = new Map(files.map(f => [f.path, f.bytes]));
  for (const file of atlas.files) {
    const bytes = byPath.get(file.path);
    if (!bytes || bytes.length !== file.size || await sha256Hex(bytes) !== file.sha256) throw new Error(`Bundle checksum differs: ${file.path}`);
  }
  return { ...b, atlas, files, atlasSha256: await sha256Hex(manifest.data) };
}

/** Explicit Publish shares the engine and success handling with quiet publishing. */
export async function publish(app: AppState = current(), projectUuid = app.sync.draft?.linked_uuid) {
  if (!canWrite() || !(await beforeCheckedAction(app, 'publish'))) return;
  await app.sync.publishImages(projectUuid);
}

export async function performPublish(app: AppState, projectUuid: string | undefined, gate: (idle?: boolean) => Promise<void>, sameProject: () => boolean) {
  const server = app.remote?.server ?? app.sync.draft?.server ?? app.server;
  const mount = app.project?.upbdb.mount_id;
  if (!server || !mount) throw new Error('Link the project to a database mount first.');
  const token = await authToken(app, server);
  // Scope persisted sessions to credentials without storing the bearer token.
  const account = await sha256Hex(new TextEncoder().encode(token ?? ''));
  const previous = app.sync.draft?.publish_job;
  const exported = previous?.exported ?? new Date().toISOString();
  await gate();
  const b = await backgroundBundle(app, projectUuid, exported);
  if (!b || !sameProject()) return;
  const sameScope = previous?.server === server && previous.mount === mount && previous.account === account;
  // A durable commit reply means only local acknowledgement remains, even if edits changed the bundle.
  if (sameScope && previous.result) {
    await finishPublish(app, b, server, mount, previous.result, previous, sameProject, () => gate(false));
    return;
  }
  let job: PublishJob;
  if (sameScope && previous.atlas_sha256 === b.atlasSha256) job = previous;
  else job = { server, mount, account, exported, atlas_sha256: b.atlasSha256, start_key: crypto.randomUUID(), commit_key: crypto.randomUUID(),
    zip_key: crypto.randomUUID(), uploaded: 0, replace_current: false, pixels_key: pixelSignature(b.project) };
  app.sync.savePublishJob(job);
  const res = await uploadImages({ server, mount, token, bundle: b, job, gate,
    checkpoint: () => app.sync.persistPublishJob(),
    save: value => { if (sameProject()) app.sync.savePublishJob(value); },
    progress: (done, total) => { if (sameProject()) app.sync.imageProgress = { done, total }; },
  });
  if (!sameProject()) return;
  const completed = { ...app.sync.draft!.publish_job!, result: res, zip_sha256: await sha256Hex(b.zip), completed_at: new Date().toISOString() };
  app.sync.savePublishJob(completed);
  await app.sync.persistPublishJob();
  // Acknowledgement still checks scope and offline guards, without waiting for our own persist.
  if (sameProject()) await finishPublish(app, b, server, mount, res, completed, sameProject, () => gate(false));
}

/** Both protocols acknowledge precisely the published snapshot, while retaining newer edits. */
async function finishPublish(app: AppState, b: PublishBundle & { project: Project }, server: string, mount: string, res: AtlasPublishOut, job: PublishJob, sameProject: () => boolean, gate: () => Promise<void>) {
  const rec: Published = {
    server,
    mount_id: mount,
    version: typeof res.version === 'number' ? res.version : null,
    atlas_sha256: job.atlas_sha256,
    zip_sha256: job.zip_sha256!,
    at: job.completed_at!,
  };
  const core = await app.core;
  if (!sameProject()) return;
  const overlays = storedBundleTones(b.project);
  const pixelsUnchanged = pixelSignature(app.fullProject()!) === job.pixels_key;
  if (app.bundle && pixelsUnchanged) {
    for (const [id, overlay] of Object.entries(overlays)) {
      const source = app.bundle[id];
      if (!source) continue;
      const mosaic = await core.bakeBundleArea(id, overlay.display);
      if (!sameProject()) return;
      app.bundle[id] = { ...source, sha256: mosaic.sha256 };
      app.stitched[id] = { ...app.stitched[id], mosaic };
      app.params[id] = { ...app.params[id], tone: neutralTone() };
      app.tone[id] = neutralTone();
      app.preview[id] = null;
      app.compare[id] = null;
    }
    app.liveKeys = {};
    app.pixelsVersion++;
  }
  if (!app.project?.upbdb.published?.some(p => p.server === server && p.atlas_sha256 === rec.atlas_sha256 && p.version === rec.version)) await core.recordPublished(rec);
  if (!sameProject()) return;
  const recorded = JSON.parse(await core.projectJson()) as Project;
  if (!sameProject()) return;
  // recordPublished updates the core link; the live UI may have changed during transfer.
  app.project = keepPrintEdits({ ...app.fullProject()!, upbdb: recorded.upbdb }, app.project);
  if (app.bundle && pixelsUnchanged && app.project?.grainatlas_ui) delete (app.project.grainatlas_ui as { bundle_tone?: unknown }).bundle_tone;
  app.persist();
  app.lastPublish = { ...rec, response: res };
  // Publishing stores images and files only; spots reach the database by sync (contract 2026-11, section 4).
  const warnings = Array.isArray(res.warnings) ? res.warnings.map(String) : [];
  const again = res.created === false;
  const label = app.sync.status?.mount_id === mount ? app.sync.status.mount_label : null;
  const check = res.spot_check;
  const differ = check ? (check.differ || 0) + (check.not_in_database || 0) : 0;
  const names = check?.names?.slice(0, 10) ?? [];
  app.notice =
    `${again ? 'Already published as' : 'Published'}${rec.version != null ? ` version ${rec.version}` : ''} to mount ${label || mount.slice(0, 8)} on ${new URL(server).host}.` +
    (differ ? ` ${differ} ${differ === 1 ? 'spot differs' : 'spots differ'} from the database${names.length ? ` (${names.join(', ')}${differ > names.length ? ' and others' : ''})` : ''}. Sync first?` : '') +
    (warnings.length ? ` Server notes: ${warnings.join('; ')}` : '');
  if (differ) app.noticeAction = { label: 'Sync', run: () => void app.sync.sync() };
  await app.sync.imagesPublished(b.project.uuid, sameProject, gate);
  if (sameProject() && app.project) notifyUpbLab(mount, b.project.uuid, app.sync.status?.revision ?? app.sync.draft?.base_revision ?? 0, server);
}
