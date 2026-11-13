import { reportStorageWrite } from './storage-failure.ts';

type Owner = Parameters<typeof reportStorageWrite>[0];
type Write = Parameters<typeof reportStorageWrite>[1];
type Legacy = Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'>;
export interface Migration { key: string; sha256: string; length: number; migrated_at: string; deleted?: boolean }
interface Options {
  indexedDB?: () => IDBFactory | undefined;
  localStorage?: () => Legacy;
  storageManager?: () => StorageManager | undefined;
}

/** Synchronous write-ahead copies in localStorage, kept only until IndexedDB commits the write
 * (owned by this module; never a legacy key). They close the few milliseconds in which a
 * crash could lose the last edit, whenever localStorage still has room. */
export const JOURNAL_PREFIX = 'grainatlas.journal.';

export function isAppStorageKey(key: string): boolean {
  return ['grainatlas.project.', 'grainatlas.copy.', 'grainatlas.draft.'].some(prefix => key.startsWith(prefix))
    || ['grainatlas.drafts', 'grainatlas.discarded-drafts', 'grainatlas.recent'].includes(key);
}
function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => { req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error); });
}
function completed(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error ?? new Error('Storage transaction aborted'));
    tx.onerror = () => { /* onabort is the definitive result */ };
  });
}
async function transaction(db: IDBDatabase, work: (tx: IDBTransaction) => void): Promise<void> {
  const tx = db.transaction(['items', 'migrated'], 'readwrite');
  const done = completed(tx);
  try { work(tx); }
  catch (error) {
    try { tx.abort(); } catch { /* already finished */ }
    await done.catch(() => {});
    throw error;
  }
  await done;
}
function inTransaction(tx: IDBTransaction, work: () => void): void {
  try { work(); } catch { tx.abort(); }
}
async function sha256(value: string): Promise<string> {
  const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value));
  return Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, '0')).join('');
}

/** Synchronous session view; only committed transactions count as durable writes.
 * Legacy values are immutable. If carry-over fails, reads retain the legacy value
 * and writes stay in memory with a storage warning until a later IDB write works.
 */
export class AppStorage {
  private values = new Map<string, string>();
  private fallback = new Set<string>();
  private idbKeys = new Set<string>();
  private db: IDBDatabase | null = null;
  private loading?: Promise<void>;
  private queue: Promise<unknown> = Promise.resolve();
  private collecting: Promise<boolean>[] | null = null;
  private permission?: Promise<boolean>;
  private legacy: () => Legacy;
  private factory: () => IDBFactory | undefined;
  private manager: () => StorageManager | undefined;
  persistenceGranted = false;

  constructor(options: Options = {}) {
    this.legacy = options.localStorage ?? (() => globalThis.localStorage);
    this.factory = options.indexedDB ?? (() => globalThis.indexedDB);
    this.manager = options.storageManager ?? (() => globalThis.navigator?.storage);
  }
  ready(): Promise<void> { return this.loading ??= this.load(); }
  /** Wait for writes already queued (also useful before a deliberate reload). */
  async flushed(): Promise<void> { await this.queue; }
  private async load(): Promise<void> {
    // Request once per app startup, including when IndexedDB is unavailable.
    this.permission ??= (async () => {
      try { return this.persistenceGranted = await this.manager()?.persist?.() ?? false; } catch { return false; }
    })();
    const legacy = new Map<string, string>();
    try {
      const storage = this.legacy();
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (key && isAppStorageKey(key)) {
          const value = storage.getItem(key);
          if (value !== null) legacy.set(key, value);
        }
      }
    } catch { /* unavailable legacy storage must not block IndexedDB */ }
    for (const [key, value] of legacy) { this.values.set(key, value); this.fallback.add(key); }
    try {
      const factory = this.factory();
      if (!factory) return;
      const opening = factory.open('grainatlas', 1);
      opening.onupgradeneeded = () => {
        for (const name of ['items', 'migrated']) if (!opening.result.objectStoreNames.contains(name)) opening.result.createObjectStore(name);
      };
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        let blocked = false;
        opening.onsuccess = () => { if (blocked) opening.result.close(); else resolve(opening.result); };
        opening.onerror = () => reject(opening.error);
        // A different window must not leave startup waiting indefinitely.
        opening.onblocked = () => { blocked = true; reject(new Error('Storage upgrade blocked')); };
      });
      this.db = db;
      db.onversionchange = () => { db.close(); this.db = null; };
      const tx = db.transaction(['items', 'migrated'], 'readonly');
      const done = completed(tx);
      const [keys, values, records] = await Promise.all([
        request(tx.objectStore('items').getAllKeys()), request(tx.objectStore('items').getAll()),
        request(tx.objectStore('migrated').getAll()) as Promise<Migration[]>, done,
      ]);
      const deleted = new Set(records.filter(record => record.deleted).map(record => record.key));
      for (const key of deleted) { this.values.delete(key); this.fallback.delete(key); this.idbKeys.add(key); }
      keys.forEach((key, i) => {
        if (typeof key === 'string' && isAppStorageKey(key) && typeof values[i] === 'string') {
          this.values.set(key, values[i]); this.fallback.delete(key);
          this.idbKeys.add(key);
        }
      });
      for (const [key, value] of legacy) {
        if (!this.fallback.has(key)) continue;
        try {
          const carried = await this.carryOver(key, value);
          if (carried === null) this.values.delete(key); else this.values.set(key, carried);
          this.fallback.delete(key);
          this.idbKeys.add(key);
        } catch { /* retain the identical legacy value until verified */ }
      }
    } catch { this.db?.close(); this.db = null; }
    this.replayJournal();
  }
  /** A journal entry exists only if its IndexedDB write never committed: it is the newest value. */
  private replayJournal(): void {
    let entries: [string, string | null][] = [];
    try {
      const storage = this.legacy();
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (!key?.startsWith(JOURNAL_PREFIX)) continue;
        const target = key.slice(JOURNAL_PREFIX.length);
        if (!isAppStorageKey(target)) continue;
        const entry = JSON.parse(storage.getItem(key) ?? 'null') as { value: string | null } | null;
        if (entry) entries.push([target, entry.value]);
      }
    } catch { entries = []; }
    for (const [key, value] of entries) {
      if (value === null) this.removeItem(key); else this.setItem(key, value);
    }
  }
  private journal(key: string, value: string | null): string | null {
    const entry = JSON.stringify({ value });
    try { this.legacy().setItem(JOURNAL_PREFIX + key, entry); return entry; } catch { return null; }
  }
  private clearJournal(key: string, entry: string | null): void {
    if (entry === null) return;
    try { if (this.legacy().getItem(JOURNAL_PREFIX + key) === entry) this.legacy().removeItem(JOURNAL_PREFIX + key); } catch { /* left for the next start */ }
  }
  private async carryOver(key: string, value: string): Promise<string | null> {
    const record: Migration = { key, sha256: await sha256(value), length: value.length, migrated_at: new Date().toISOString() };
    let carried: string | null = value;
    // Another window may have saved or deleted this key since startup's scan.
    // Check again within the same transaction that performs the carry-over.
    await transaction(this.db!, tx => {
      const items = tx.objectStore('items');
      const current = items.get(key);
      current.onsuccess = () => inTransaction(tx, () => {
        if (typeof current.result === 'string') { carried = current.result; return; }
        const audit = tx.objectStore('migrated').get(key);
        audit.onsuccess = () => inTransaction(tx, () => {
          if (audit.result?.deleted) { carried = null; return; }
          items.put(value, key);
          const read = items.get(key);
          // A mismatch aborts both stores before either can commit.
          read.onsuccess = () => inTransaction(tx, () => {
            if (read.result !== value) tx.abort();
            else tx.objectStore('migrated').put(record, key);
          });
        });
      });
    });
    return carried;
  }
  getItem(key: string): string | null {
    if (!isAppStorageKey(key)) return this.legacy().getItem(key);
    if (this.fallback.has(key) || !this.loading) return this.legacy().getItem(key);
    return this.values.get(key) ?? null;
  }
  keys(): string[] {
    const keys = new Set([...this.values.keys(), ...this.fallback]);
    try {
      const storage = this.legacy();
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (key && !key.startsWith(JOURNAL_PREFIX) && (!isAppStorageKey(key) || !this.loading)) keys.add(key);
      }
    } catch { /* IndexedDB still works with localStorage blocked */ }
    return [...keys];
  }
  get length(): number { return this.keys().length; }
  key(index: number): string | null { return this.keys()[index] ?? null; }
  setItem(key: string, value: string): void {
    if (!isAppStorageKey(key)) { this.legacy().setItem(key, value); return; }
    this.values.set(key, value);
    this.enqueue(key, value, this.loading ? this.journal(key, value) : null);
  }
  removeItem(key: string): void {
    if (!isAppStorageKey(key)) { this.legacy().removeItem(key); return; }
    this.values.delete(key);
    this.enqueue(key, null, this.loading ? this.journal(key, null) : null);
  }
  private enqueue(key: string, value: string | null, journal: string | null = null): void {
    const job = this.queue.then(async () => {
      try {
        if (!this.db) {
          if (this.idbKeys.has(key)) throw new Error('IndexedDB unavailable');
          const legacy = this.legacy();
          const original = legacy.getItem(key);
          // A first copy can still be kept as before. An existing legacy
          // string can only be reused, never overwritten or removed.
          if (value === null || (original !== null && original !== value)) throw new Error('IndexedDB unavailable');
          if (original === null) legacy.setItem(key, value);
          this.fallback.add(key);
          this.clearJournal(key, journal);
          return true;
        }
        // A previously unverified legacy key still needs its audit record before
        // any new value may replace it in IndexedDB. Never mutate localStorage.
        if (this.fallback.has(key)) {
          const original = this.legacy().getItem(key);
          if (original !== null) await this.carryOver(key, original);
          this.fallback.delete(key);
          this.idbKeys.add(key);
        }
        await transaction(this.db, tx => {
          const items = tx.objectStore('items');
          if (value === null) items.delete(key); else items.put(value, key);
          const records = tx.objectStore('migrated');
          const existing = records.get(key);
          existing.onsuccess = () => inTransaction(tx, () => {
            if (existing.result || value === null) records.put({ ...existing.result, key, deleted: value === null }, key);
          });
        });
        this.fallback.delete(key);
        this.idbKeys.add(key);
        this.clearJournal(key, journal);
        return true;
      } catch { return false; }
    });
    this.queue = job;
    this.collecting?.push(job);
  }
  /** Report the whole project/draft write, including its index, after commit.
   * Capture the tab and UUID so a delayed failure cannot warn a different project.
   */
  track(app: Owner, kind: Write, write: () => boolean): boolean {
    const uuid = app.project?.uuid;
    const previous = this.collecting;
    const jobs: Promise<boolean>[] = [];
    this.collecting = jobs;
    let result: boolean;
    try { result = write(); } finally { this.collecting = previous; }
    const report = (ok: boolean) => { if (app.project?.uuid === uuid) reportStorageWrite(app, kind, ok); };
    if (!result || !jobs.length) report(result);
    if (jobs.length) void Promise.all(jobs).then(results => report(result && results.every(Boolean)));
    return result;
  }
  async usageText(): Promise<string> {
    await this.permission;
    let usage = [...this.values].reduce((total, [key, value]) => total + key.length + value.length, 0) * 2;
    let available: number | undefined;
    let persisted = this.persistenceGranted;
    try {
      const manager = this.manager();
      const estimate = await manager?.estimate?.();
      usage = estimate?.usage ?? usage;
      if (estimate?.quota !== undefined) available = Math.max(0, estimate.quota - usage);
      persisted = await manager?.persisted?.() ?? persisted;
    } catch { /* show the session's string sizes when the estimate is unavailable */ }
    const size = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${(bytes / 1e6).toFixed(1)} MB`;
    return `Saved projects in this app: ${size(usage)}${available === undefined ? '' : ` (of about ${size(available)} available)`} · ${persisted ? 'kept permanently' : 'may be cleared by the system'}`;
  }
}

export const appStorage = new AppStorage();
