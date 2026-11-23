import type { MirrorStorage } from './mount-mirror';
import type { AtlasJson } from './remote';
import type { Project } from './core/types';
import type { ReadElements } from './sync';

export interface OfflineBundle { atlas: AtlasJson; project: Project; elements: ReadElements | null; revision: number | null; at: string; version?: number }
export const BUNDLE_LIMIT = 20;
export const BUNDLE_BYTES = 50 * 1024 * 1024;
export const SINGLE_BUNDLE_BYTES = 25 * 1024 * 1024;
export function bundleKey(mirror: string, account: string, project: string) { return `mirror/${mirror}/${account}/bundles/${project.toLowerCase()}.json`; }
const indexKey = (mirror: string, account: string) => bundleKey(mirror, account, 'index');
interface Entry { project: string; bytes: number }
async function index(storage: MirrorStorage, mirror: string, account: string): Promise<Entry[]> {
  try { const rows = JSON.parse(await storage.getItem(indexKey(mirror, account)) ?? '[]'); return Array.isArray(rows) ? rows.filter(e => typeof e?.project === 'string' && Number.isFinite(e.bytes) && e.bytes > 0) : []; } catch { return []; }
}
// Serialize read/modify/write across consumers; no eviction can race an active cache write.
let queue: Promise<unknown> = Promise.resolve();
function serial<T>(fn: () => Promise<T>): Promise<T> {
  const task = queue.then(fn); queue = task.catch(() => {}); return task;
}
export function bundleProjects(storage: MirrorStorage, mirror: string, account: string) {
  return serial(async () => (await index(storage, mirror, account)).map(e => e.project));
}
export function cacheBundle(storage: MirrorStorage, mirror: string, account: string, record: OfflineBundle) {
  return serial(async () => {
    const value = JSON.stringify(record), bytes = new TextEncoder().encode(value).length;
    if (bytes > SINGLE_BUNDLE_BYTES || !storage.removeItem) return false;
    const project = record.atlas.project_id.toLowerCase();
    let rows = (await index(storage, mirror, account)).filter(e => e.project !== project);
    // Evict before writing: even transient storage stays within the budget.
    while (rows.length >= BUNDLE_LIMIT || rows.reduce((n, e) => n + e.bytes, bytes) + new TextEncoder().encode(JSON.stringify([...rows, { project, bytes }])).length > BUNDLE_BYTES) {
      const old = rows.shift(); if (!old) break;
      await storage.removeItem(bundleKey(mirror, account, old.project));
      await storage.setItem(indexKey(mirror, account), JSON.stringify(rows));
    }
    await storage.setItem(bundleKey(mirror, account, project), value);
    rows = [...rows, { project, bytes }];
    try { await storage.setItem(indexKey(mirror, account), JSON.stringify(rows)); }
    catch (error) {
      // A failed index write must not leave an unbounded, untracked bundle behind.
      await storage.removeItem(bundleKey(mirror, account, project));
      throw error;
    }
    return true;
  });
}
export function readBundle(storage: MirrorStorage, mirror: string, account: string, project: string): Promise<OfflineBundle | null> {
  return serial(async () => {
    try {
      project = project.toLowerCase();
      const rows = await index(storage, mirror, account), entry = rows.find(e => e.project === project);
      if (!entry) return null;
      const record = JSON.parse(await storage.getItem(bundleKey(mirror, account, project)) ?? 'null') as OfflineBundle | null;
      if (!record || record.atlas?.project_id.toLowerCase() !== project || record.project?.uuid.toLowerCase() !== project) return null;
      await storage.setItem(indexKey(mirror, account), JSON.stringify([...rows.filter(e => e !== entry), entry]));
      return record;
    } catch { return null; }
  });
}
