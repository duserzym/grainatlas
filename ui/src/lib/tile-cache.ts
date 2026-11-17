/** Disposable published image bytes, separate from project/draft storage. */
export const IMAGE_CACHE_CAP = 1_000_000_000;
export interface CacheEntry { hash: string; size: number; lastUsed: number }
export interface CacheStore {
  get(hash: string): Promise<ArrayBuffer | undefined>;
  metadata(hash: string): Promise<CacheEntry | undefined>;
  touch(entry: CacheEntry): Promise<void>;
  put(entry: CacheEntry, bytes: ArrayBuffer): Promise<void>;
  remove(hash: string): Promise<void>;
  oldest(): Promise<CacheEntry[]>;
  size(): Promise<number>;
  setSize(size: number): Promise<void>;
  clear(): Promise<void>;
}
export interface CacheDatabase {
  transaction<T>(write: boolean, work: (store: CacheStore) => Promise<T>): Promise<T>;
}

function request<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
}

function openDatabase(): Promise<CacheDatabase> {
  return new Promise((resolve, reject) => {
    const opening = indexedDB.open('grainatlas-image-cache', 1);
    opening.onupgradeneeded = () => {
      const db = opening.result;
      db.createObjectStore('bytes');
      db.createObjectStore('metadata', { keyPath: 'hash' }).createIndex('lastUsed', 'lastUsed');
      db.createObjectStore('totals');
    };
    opening.onerror = () => reject(opening.error);
    opening.onblocked = () => reject(new Error('Image cache is unavailable.'));
    opening.onsuccess = () => {
      const db = opening.result;
      db.onversionchange = () => db.close();
      resolve({
        async transaction<T>(write: boolean, work: (store: CacheStore) => Promise<T>): Promise<T> {
          const tx = db.transaction(['bytes', 'metadata', 'totals'], write ? 'readwrite' : 'readonly');
          const complete = new Promise<void>((yes, no) => {
            tx.oncomplete = () => yes();
            tx.onabort = tx.onerror = () => no(tx.error ?? new Error('Image cache transaction failed.'));
          });
          const bytes = tx.objectStore('bytes'), meta = tx.objectStore('metadata'), totals = tx.objectStore('totals');
          const store: CacheStore = {
            get: hash => request(bytes.get(hash)),
            metadata: hash => request(meta.get(hash)),
            touch: async entry => { await request(meta.put(entry)); },
            put: async (entry, value) => { await request(bytes.put(value, entry.hash)); await request(meta.put(entry)); },
            remove: async hash => { await request(bytes.delete(hash)); await request(meta.delete(hash)); },
            // Scan small metadata only, never all the cached image buffers.
            oldest: () => request(meta.index('lastUsed').getAll()),
            size: async () => (await request(totals.get('size'))) ?? 0,
            setSize: async size => { await request(totals.put(size, 'size')); },
            clear: async () => { await request(bytes.clear()); await request(meta.clear()); await request(totals.clear()); },
          };
          try {
            const value = await work(store);
            await complete;
            return value;
          } catch (error) {
            try { tx.abort(); } catch { /* already completed/aborted */ }
            await complete.catch(() => {});
            throw error;
          }
        },
      });
    };
  });
}

export class ImageCache {
  private database: Promise<CacheDatabase> | null = null;
  private open: () => Promise<CacheDatabase>;
  private cap: number;
  private now: () => number;
  constructor(open: () => Promise<CacheDatabase> = openDatabase, cap = IMAGE_CACHE_CAP, now = Date.now) {
    this.open = open;
    this.cap = cap;
    this.now = now;
  }
  private db() { return this.database ??= this.open(); }

  async get(hash: string): Promise<ArrayBuffer | undefined> {
    try {
      return await (await this.db()).transaction(true, async store => {
        const bytes = await store.get(hash), entry = await store.metadata(hash);
        if (bytes && entry) { await store.touch({ ...entry, lastUsed: this.now() }); return bytes; }
      });
    } catch { return undefined; }
  }

  /** Hashing/storage failures leave the network result usable, without caching it. */
  async put(hash: string, bytes: ArrayBuffer): Promise<void> {
    try {
      const digest = await crypto.subtle.digest('SHA-256', bytes);
      const actual = Array.from(new Uint8Array(digest), b => b.toString(16).padStart(2, '0')).join('');
      if (actual !== hash.toLowerCase() || bytes.byteLength > this.cap) return;
      await (await this.db()).transaction(true, async store => {
        const previous = await store.metadata(hash);
        let size = await store.size() - (previous?.size ?? 0) + bytes.byteLength;
        await store.put({ hash, size: bytes.byteLength, lastUsed: this.now() }, bytes);
        if (size > this.cap) for (const entry of await store.oldest()) {
          await store.remove(entry.hash);
          size -= entry.size;
          if (size <= this.cap * 0.8) break;
        }
        await store.setSize(size);
      });
    } catch { /* quota, private mode, unavailable crypto/IndexedDB: network only */ }
  }

  async load(hash: string | undefined, network: () => Promise<ArrayBuffer>): Promise<ArrayBuffer> {
    if (hash) { const hit = await this.get(hash); if (hit) return hit; }
    const bytes = await network();
    if (hash) await this.put(hash, bytes);
    return bytes;
  }
  async size(): Promise<number | null> {
    try { return await (await this.db()).transaction(false, store => store.size()); } catch { return null; }
  }
  async clear(): Promise<boolean> {
    try { await (await this.db()).transaction(true, store => store.clear()); return true; } catch { return false; }
  }
}
export const imageCache = new ImageCache();
