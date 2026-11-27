import { timed } from './sync-perf.ts';
import { canWrite } from './presentation.ts';
import { isDesktop } from './core/core.ts';

/** Commands deliberately accept keys, never paths. The Rust side owns the folder. */
export interface FileCommands {
  list(): Promise<{ path: string; keys: string[] }>;
  read(key: string): Promise<string | null>;
  write(key: string, contents: string): Promise<void>;
  remove(key: string): Promise<void>;
}
export async function desktopFileCommands(): Promise<FileCommands | null> {
  if (!isDesktop()) return null;
  const { invoke } = await import('@tauri-apps/api/core');
  return {
    list: () => invoke('store_list'),
    read: key => invoke('store_read', { key }),
    write: (key, contents) => timed('storage.invoke-write.handoff', () => invoke('store_write', { key, contents })),
    remove: key => invoke('store_remove', { key }),
  };
}
export interface FileMigration { key: string; sha256: string; length: number; migrated_at: string; deleted?: boolean }
interface Entry { version: 1; value: string | null; committed: boolean; carry_over?: FileMigration }
function parse(contents: string): Entry {
  const entry = JSON.parse(contents) as Entry;
  if (entry?.version !== 1 || (entry.value !== null && typeof entry.value !== 'string') || typeof entry.committed !== 'boolean') {
    throw new Error('Invalid stored copy');
  }
  return entry;
}

/** Each file contains its value and audit together. Uncommitted carry-over files
 * are never read as app values. Null is a durable deletion, so retained IDB and
 * localStorage copies cannot resurrect a deleted key on the next startup. */
export class FileStorage {
  private entries = new Map<string, Entry>();
  private commands: FileCommands;
  constructor(commands: FileCommands) { this.commands = commands; }
  async load(sources: Map<string, string | null>, handled: (key: string) => boolean, reconcileSources: ReadonlySet<string> = new Set()): Promise<{ path: string; values: Map<string, string | null> }> {
    const { path, keys } = await this.commands.list();
    const values = new Map<string, string | null>();
    for (const key of new Set([...keys.filter(handled), ...sources.keys()])) {
      const contents = await this.commands.read(key);
      let entry = contents === null ? null : parse(contents);
      // IndexedDB may have received newer fallback writes while file commands
      // were unavailable. Compare against the source fingerprint, never against
      // the (usually newer) file value. This also works with a full localStorage
      // where the emergency journal could not fit.
      let record: FileMigration | undefined;
      if (sources.has(key)) {
        const value = sources.get(key)!;
        // Preserve the original string, including whitespace and non-ASCII text.
        const bytes = new TextEncoder().encode(value ?? '');
        const hash = await crypto.subtle.digest('SHA-256', bytes);
        record = { key, sha256: Array.from(new Uint8Array(hash), b => b.toString(16).padStart(2, '0')).join(''), length: bytes.length, migrated_at: new Date().toISOString(), ...(value === null ? { deleted: true } : {}) };
      }
      const changedSource = reconcileSources.has(key) && record && (!entry?.carry_over
        || record.sha256 !== entry.carry_over.sha256 || record.length !== entry.carry_over.length
        || !!record.deleted !== !!entry.carry_over.deleted);
      if (!entry?.committed || changedSource) {
        if (!record) continue;
        if (!canWrite()) throw new Error('Storage carry-over requires editor');
        const value = sources.get(key)!;
        const staged: Entry = { version: 1, value, committed: false };
        await this.commands.write(key, JSON.stringify(staged));
        const read = await this.commands.read(key);
        if (read !== JSON.stringify(staged)) throw new Error('Stored copy differs');
        entry = { ...staged, committed: true, carry_over: record };
        const committed = JSON.stringify(entry);
        await this.commands.write(key, committed);
        if (await this.commands.read(key) !== committed) throw new Error('Carry-over record differs');
      }
      this.entries.set(key, entry);
      values.set(key, entry.value);
    }
    return { path, values };
  }
  async write(key: string, value: string | null): Promise<void> {
    const entry: Entry = { ...this.entries.get(key), version: 1, value, committed: true };
    await this.commands.write(key, JSON.stringify(entry));
    this.entries.set(key, entry);
  }
}
