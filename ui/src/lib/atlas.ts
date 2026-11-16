import { canWrite } from './presentation';
// Atlas bundles (upblab/docs/WORKFLOW.md section 3.1): save as a `<project_id>.atlas/`
// folder or a zip, publish to UPbDB. The core writes the bytes (identical in the browser
// and the desktop app); this file only moves them.

import { beforeCheckedAction } from './checks';
import { token as shellToken } from '@lab/bridge';
import type { BundleOptions, Project, Published } from './core/types';
import { download } from './files';
import { normalizeServer, publishAtlas } from './upbdb';
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
  if (!globalThis.crypto?.subtle) return '';
  const d = await crypto.subtle.digest('SHA-256', b as BufferSource);
  return Array.from(new Uint8Array(d), (x) => x.toString(16).padStart(2, '0')).join('');
}

/** True where the browser can write a folder (Chromium's File System Access API). */
export function canSaveFolder(): boolean {
  return typeof window !== 'undefined' && 'showDirectoryPicker' in window;
}

async function bundleZip(app: AppState, projectUuid?: string): Promise<{ project: Project; zip: Uint8Array } | null> {
  const p = app.fullProject();
  if (!p) return null;
  if (projectUuid) p.uuid = projectUuid;
  const core = await app.core;
  const zip = await app.run('Building the atlas bundle', () => core.atlasZip(p, bundleOptions(p, app)));
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

/** Posts the bundle to `POST /v1/mounts/{mount_id}/atlas` and records the version in the project. */
export async function publish(app: AppState = current(), projectUuid = app.sync.draft?.linked_uuid) {
  if (!canWrite()) return;
  if (!(await beforeCheckedAction(app, 'publish'))) return;
  const server = app.server;
  const mount = app.project?.upbdb.mount_id;
  if (!server) {
    app.error = 'No UPbDB server: set one in Settings.';
    return;
  }
  if (!mount) {
    app.error = 'Link the project to a mount first.';
    return;
  }
  const b = await bundleZip(app, projectUuid);
  if (!b) return;
  const tok = await authToken(app);
  const res = await app.run(`Publishing to ${new URL(server).host}`, () => publishAtlas(server, tok, mount, b.project.uuid, b.zip));
  if (!res) return;
  const atlas = unzipStored(b.zip)[0];
  const rec: Published = {
    server,
    mount_id: mount,
    version: typeof res.version === 'number' ? res.version : null,
    atlas_sha256: await sha256Hex(atlas.data),
    zip_sha256: await sha256Hex(b.zip),
    at: new Date().toISOString(),
  };
  const core = await app.core;
  await core.recordPublished(rec);
  app.project = JSON.parse(await core.projectJson());
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
  if (app.project) notifyUpbLab(mount, b.project.uuid, app.sync.status?.revision ?? app.sync.draft?.base_revision ?? 0, server);
}
