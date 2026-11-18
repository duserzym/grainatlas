import { isDesktop } from './core/core.ts';

/** Keyed reads only: never enumerate accounts or fall back to a last-used mirror.
 * Browser builds use the same paths as IndexedDB keys; desktop owns the app-data root. */
export interface MirrorStorage {
  getItem(key: string): string | null | Promise<string | null>;
  setItem(key: string, value: string): void | Promise<void>;
}
class MountMirror implements MirrorStorage {
  private opening?: Promise<IDBDatabase>;
  private queue: Promise<void> = Promise.resolve();
  private db() {
    return this.opening ??= new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open('grainatlas-mirror', 1);
      request.onupgradeneeded = () => request.result.createObjectStore('items');
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error('Mirror storage is blocked'));
    });
  }
  async getItem(key: string) {
    await this.queue;
    if (isDesktop()) {
      const { invoke } = await import('@tauri-apps/api/core');
      return invoke<string | null>('mirror_read', { key });
    }
    const db = await this.db();
    return new Promise<string | null>((resolve, reject) => {
      const request = db.transaction('items').objectStore('items').get(key);
      request.onsuccess = () => resolve(typeof request.result === 'string' ? request.result : null);
      request.onerror = () => reject(request.error);
    });
  }
  setItem(key: string, value: string) {
    const write = this.queue.then(async () => {
      if (isDesktop()) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('mirror_write', { key, contents: value });
      } else {
        const db = await this.db();
        await new Promise<void>((resolve, reject) => {
          const tx = db.transaction('items', 'readwrite');
          tx.objectStore('items').put(value, key);
          tx.oncomplete = () => resolve();
          tx.onabort = () => reject(tx.error);
          tx.onerror = () => { /* onabort reports the failure */ };
        });
      }
    });
    this.queue = write.catch(() => {});
    return write;
  }
}
export const mountMirror: MirrorStorage = new MountMirror();
