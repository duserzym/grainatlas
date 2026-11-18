import { idempotentWrite } from './idempotency.ts';
import { fetchWithDeadline } from './http.ts';
import { oversightSession, serverFetch, ServerReadOnlyError } from './oversight.ts';
import { publishAtlas, type AtlasPublishOut } from './upbdb.ts';
import type { AtlasJson } from './remote';

export const PART_BYTES = 8 * 1024 * 1024;
export interface PublishJob {
  server: string; mount: string; account: string; upload_id?: string;
  atlas_sha256: string; start_key: string; commit_key: string; zip_key: string;
  uploaded: number; expires_at?: string; exported: string; pixels_key: string | null;
  result?: AtlasPublishOut; zip_sha256?: string; completed_at?: string;
  replace_current: boolean; decision?: 'ask' | 'keep';
}
export interface PublishBundle {
  atlas: AtlasJson; atlasSha256: string; files: { path: string; bytes: Uint8Array }[];
  zip: Uint8Array;
}
export class PublishError extends Error {
  status: number; code: string; raw: string; retryAfter: number; missing: string[];
  constructor(status: number, code: string, raw: string, retryAfter = 0, missing: string[] = []) {
    super(`Publishing images: ${status || 'network error'}${raw ? ` · ${raw.slice(0, 300)}` : ''}`);
    this.status = status; this.code = code; this.raw = raw; this.retryAfter = retryAfter; this.missing = missing;
  }
  get transient() { return !this.status || this.status >= 500 || this.status === 429 || this.code === 'idempotency_in_progress'; }
}
export const waitFor = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
export function retryDelay(failures: number, random = Math.random()) {
  return Math.min(300_000, 5000 * 2 ** Math.min(failures, 6) * (0.8 + random * 0.4));
}
export function retryAfter(response: Response): number {
  const value = response.headers.get('Retry-After');
  if (!value) return 0;
  const seconds = Number(value);
  return Number.isFinite(seconds) ? Math.max(0, seconds * 1000) : Math.max(0, Date.parse(value) - Date.now()) || 0;
}
async function failure(r: Response) {
  let raw: string;
  try { raw = await r.text(); }
  catch (error) { throw new PublishError(0, '', String(error)); }
  let body; try { body = JSON.parse(raw); } catch { /* plain reply */ }
  return new PublishError(r.status, body?.code ?? body?.detail?.code ?? '', raw, retryAfter(r), body?.missing ?? body?.detail?.missing ?? []);
}
async function responseJson(r: Response) {
  try { return await r.json(); }
  catch (error) { throw new PublishError(0, '', String(error)); }
}
const capabilities = new Map<string, Promise<boolean>>();
/** Unsupported routes are session-scoped; network failures can be checked again. */
export function resumablePublishing(server: string, token: string | null): Promise<boolean> {
  server = server.replace(/\/+$/, '');
  let value = capabilities.get(server);
  if (!value) {
    value = (async () => {
      try {
        const r = await serverFetch(`${server}/v1/assets/check`, { method: 'POST', headers: {
          'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}),
        }, body: '{"sha256":[]}' }, undefined, fetchWithDeadline);
        if (r.status === 200) return true;
        if ([404, 405, 501].includes(r.status)) return false;
        throw await failure(r);
      } catch (error) { throw error instanceof PublishError ? error : new PublishError(0, '', String(error)); }
    })();
    capabilities.set(server, value);
    void value.catch(() => { capabilities.delete(server); });
  }
  return value;
}
// Share the upload budget across tabs and foreground publishing too.
let uploading = 0;
const uploadWaiters: (() => void)[] = [];
async function uploadSlot<T>(work: () => Promise<T>): Promise<T> {
  if (uploading < 2) uploading++;
  else await new Promise<void>(resolve => uploadWaiters.push(resolve));
  try { return await work(); }
  finally { const next = uploadWaiters.shift(); if (next) next(); else uploading--; }
}
export interface PublishOptions {
  server: string; mount: string; token: string | null; bundle: PublishBundle; job: PublishJob;
  save: (job: PublishJob) => void; gate: () => Promise<void>;
  progress: (done: number, total: number) => void;
  wait?: (ms: number) => Promise<void>; checkpoint?: () => Promise<void>;
}
/** One immutable manifest and keys per job. Credentials never enter draft storage. */
export async function uploadImages(o: PublishOptions): Promise<AtlasPublishOut> {
  if (oversightSession.readOnly) throw new ServerReadOnlyError('Viewing as someone else is read-only.');
  const { bundle, token, server, mount } = o;
  let job = { ...o.job };
  const wait = o.wait ?? waitFor;
  const save = (patch: Partial<PublishJob>) => { job = { ...job, ...patch }; o.save(job); };
  async function request(path: string, init: RequestInit = {}, key?: 'start_key' | 'commit_key') {
    const body = init.body;
    for (let attempt = 0; ; attempt++) {
      await o.gate();
      await o.checkpoint?.();
      try {
        const fetchRequest = (id?: string) => serverFetch(`${server}${path}`, { ...init, body, headers: {
          ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(init.headers as Record<string, string>),
          ...(id ? { 'Idempotency-Key': id } : {}),
        } }, bundle.atlas.project_id, fetchWithDeadline);
        const send = (id?: string) => init.method === 'PUT' ? uploadSlot(async () => { await o.gate(); return fetchRequest(id); }) : fetchRequest(id);
        const partKey = (init.headers as Record<string, string> | undefined)?.['Idempotency-Key'];
        const r = key || partKey ? await idempotentWrite(send, { key: key ? job[key] : partKey, keyChanged: key ? async id => { save({ [key]: id }); await o.checkpoint?.(); } : undefined, wait }) : await send();
        if (r.ok || r.status === 304 || r.status === 308) return r;
        throw await failure(r);
      } catch (error) {
        if (error instanceof ServerReadOnlyError) throw error;
        if (error instanceof DOMException && error.name === 'AbortError') throw error;
        const e = error instanceof PublishError ? error : new PublishError(0, '', String(error));
        if (!e.transient || attempt >= 2) throw e;
        await wait(Math.max(retryDelay(attempt), e.retryAfter));
      }
    }
  }
  if (job.decision === 'ask') throw new PublishError(409, 'replace_confirm', 'A decision is needed.');
  o.progress(job.uploaded, bundle.atlas.files.length);
  await o.gate();
  if (!await resumablePublishing(server, token)) {
    for (let attempt = 0; ; attempt++) {
      await o.gate();
      await o.checkpoint?.();
      try { return await uploadSlot(async () => {
        await o.gate();
        return publishAtlas(server, token, mount, bundle.atlas.project_id, bundle.zip, job.zip_key, job.replace_current, async key => { save({ zip_key: key }); await o.checkpoint?.(); }, job.start_key);
      }); }
      catch (error) {
        if (error instanceof ServerReadOnlyError) throw error;
        const raw = error as { status?: number; code?: string; retryAfter?: number; raw?: string };
        const e = new PublishError(raw.status ?? 0, raw.code ?? '', raw.raw ?? String(error), raw.retryAfter);
        if (e.code === 'replace_confirm') { save({ decision: 'ask' }); throw e; }
        if (e.status && e.status < 500 && e.status !== 429 || attempt >= 2) throw e;
        await wait(Math.max(retryDelay(attempt), e.retryAfter ?? 0));
      }
    }
  }
  type Status = { missing: string[]; received: number; total: number; state: string };
  let missing: string[] = [];
  const restarting = !!job.upload_id;
  if (job.upload_id && (!job.expires_at || Date.parse(job.expires_at) > Date.now())) {
    const cacheKey = `${server}\n${token}\n${job.upload_id}`;
    const cached = statuses.get(cacheKey);
    try {
      const r = await request(`/v1/atlas/uploads/${encodeURIComponent(job.upload_id)}`, { headers: cached?.etag ? { 'If-None-Match': cached.etag } : {} });
      const status: Status = r.status === 304 ? cached!.status : await responseJson(r);
      if (statuses.size >= 32) statuses.delete(statuses.keys().next().value!);
      statuses.set(cacheKey, { etag: r.headers.get('ETag') ?? cached?.etag ?? null, status });
      if (status.state === 'expired') save({ upload_id: undefined });
      else { missing = status.missing; save({ uploaded: status.received }); }
    } catch (error) {
      if (error instanceof PublishError && [404, 410].includes(error.status)) save({ upload_id: undefined });
      else throw error;
    }
  } else if (job.upload_id) save({ upload_id: undefined });
  if (!job.upload_id) {
    // Expiration is a new logical creation, even when the manifest is unchanged.
    if (restarting || job.expires_at) save({ start_key: crypto.randomUUID(), commit_key: crypto.randomUUID(), expires_at: undefined });
    try {
      const r = await request(`/v1/mounts/${encodeURIComponent(mount)}/atlas/uploads`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ atlas: bundle.atlas, replace_current: job.replace_current }) }, 'start_key');
      const created = await responseJson(r);
      missing = created.missing;
      const absent = new Set(missing);
      save({ upload_id: created.upload_id, expires_at: created.expires_at, uploaded: created.received ?? bundle.atlas.files.filter(file => !absent.has(file.sha256)).length });
    } catch (error) {
      if (error instanceof PublishError && error.code === 'replace_confirm') save({ decision: 'ask' });
      throw error;
    }
  }
  const files = new Map(bundle.files.map(f => [f.path, f.bytes]));
  const hashes = new Map(bundle.atlas.files.map(f => [f.sha256, f]));
  const occurrences = new Map<string, number>();
  for (const file of bundle.atlas.files) occurrences.set(file.sha256, (occurrences.get(file.sha256) ?? 0) + 1);
  const base = `/v1/atlas/uploads/${encodeURIComponent(job.upload_id!)}`;
  async function upload(hashesToSend: string[]) {
    const queue = [...new Set(hashesToSend)];
    let next = 0, stopped = false;
    const results = await Promise.allSettled(Array.from({ length: 2 }, async () => {
      while (!stopped && next < queue.length) {
        const hash = queue[next++], file = hashes.get(hash), bytes = file && files.get(file.path);
        try {
          if (!file || !bytes) throw new PublishError(422, 'unknown_hash', `Server requested an unlisted file: ${hash}`);
          let offset = 0;
          do {
            await o.gate();
            const end = Math.min(bytes.length, offset + PART_BYTES);
            const chunked = bytes.length > PART_BYTES;
            // Stable UUID per exact part, also after a crash. Never reuse a key for a different body.
            const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`${job.start_key}:${hash}:${offset}`)));
            digest[6] = (digest[6] & 15) | 64; digest[8] = (digest[8] & 63) | 128;
            const hex = Array.from(digest.slice(0, 16), b => b.toString(16).padStart(2, '0')).join('');
            const key = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
            const r = await request(`${base}/blobs/${hash}`, { method: 'PUT', headers: {
              'Content-Type': file.path.endsWith('.webp') ? 'image/webp' : 'application/json', 'Idempotency-Key': key,
              ...(chunked ? { 'Content-Range': `bytes ${offset}-${end - 1}/${bytes.length}` } : {}),
            }, body: bytes.subarray(offset, end) as BodyInit });
            if (r.status !== 308) break;
            const range = /^bytes[= ]0-(\d+)$/.exec(r.headers.get('Range') ?? '');
            const received = range ? Number(range[1]) + 1 : NaN;
            if (!chunked || !Number.isSafeInteger(received) || received <= offset || received >= bytes.length) throw new PublishError(308, 'invalid_range', 'The server returned an invalid upload Range.');
            offset = received;
          } while (offset < bytes.length);
          save({ uploaded: Math.min(bundle.atlas.files.length, job.uploaded + (occurrences.get(hash) ?? 1)) });
          o.progress(job.uploaded, bundle.atlas.files.length);
        } catch (error) { stopped = true; throw error; }
      }
    }));
    for (const result of results) if (result.status === 'rejected') throw result.reason;
  }
  o.progress(job.uploaded, bundle.atlas.files.length);
  await upload(missing);
  let previousIncomplete = '';
  for (;;) {
    try { return await responseJson(await request(`${base}/commit`, { method: 'POST' }, 'commit_key')); }
    catch (error) {
      if (error instanceof PublishError && error.code === 'replace_confirm') save({ decision: 'ask' });
      if (!(error instanceof PublishError) || error.code !== 'incomplete' || !error.missing.length) throw error;
      const signature = [...error.missing].sort().join(',');
      if (signature === previousIncomplete) throw error;
      previousIncomplete = signature;
      const missingSet = new Set(error.missing);
      save({ uploaded: bundle.atlas.files.filter(f => !missingSet.has(f.sha256)).length });
      await upload(error.missing);
      // ADR-0010 stores 4xx replies. A repaired commit must use a new key.
      save({ commit_key: crypto.randomUUID() });
    }
  }
}
const statuses = new Map<string, { etag: string | null; status: { missing: string[]; received: number; total: number; state: string } }>();

export function clearPublishStatusCache() { statuses.clear(); }
