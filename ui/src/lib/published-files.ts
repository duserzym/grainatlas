import type { Remote } from './remote';
import { atlasUrl } from './upbdb.ts';
import { imageCache } from './tile-cache.ts';

type Address = Pick<Remote, 'server' | 'projectId' | 'token' | 'access' | 'version'>;
export function fileRequest(r: Address, path: string): [string, RequestInit] {
  const query = new URLSearchParams();
  if (r.version !== undefined) query.set('version', String(r.version));
  if (r.access) query.set('access', r.access);
  const suffix = query.size ? `?${query}` : '';
  return [atlasUrl(r.server, r.projectId, path) + suffix, {
    cache: r.version === undefined ? 'no-cache' : 'default',
    headers: r.token ? { Authorization: `Bearer ${r.token}` } : {},
  }];
}

const indexes = new WeakMap<Remote['atlas'], Map<string, string>>();
const pending = new Map<string, { promise: Promise<ArrayBuffer>; controller: AbortController; users: number }>();

/** Shared by background assembly and every published pyramid view. */
export async function publishedFile(remote: Remote, path: string, signal?: AbortSignal): Promise<ArrayBuffer> {
  signal?.throwIfAborted();
  let index = indexes.get(remote.atlas);
  if (!index) { index = new Map(remote.atlas.files.map(f => [f.path, f.sha256])); indexes.set(remote.atlas, index); }
  const hash = index.get(path) || undefined;
  const [url, init] = fileRequest(remote, path);
  // Include authorization: concurrent readers with different credentials must not share failures.
  const key = `${url}\n${remote.token ?? ''}\n${hash ?? ''}`;
  let job = pending.get(key);
  if (!job) {
    const controller = new AbortController();
    const promise = imageCache.load(hash, async () => {
      controller.signal.throwIfAborted();
      const response = await fetch(url, { ...init, signal: controller.signal });
      if (!response.ok) throw new Error(`Published file ${path}: ${response.status}`);
      return response.arrayBuffer();
    });
    job = { promise, controller, users: 0 };
    pending.set(key, job);
    const active = job;
    void promise.finally(() => { if (pending.get(key) === active) pending.delete(key); }).catch(() => {});
  }
  const active = job;
  active.users++;
  let abort: () => void = () => {};
  const stopped = new Promise<never>((_, reject) => {
    abort = () => reject(signal?.reason ?? new DOMException('Stopped', 'AbortError'));
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
  });
  try { return await Promise.race([active.promise, stopped]); }
  finally {
    signal?.removeEventListener('abort', abort);
    if (--active.users === 0) {
      active.controller.abort();
      if (pending.get(key) === active) pending.delete(key);
    }
  }
}
