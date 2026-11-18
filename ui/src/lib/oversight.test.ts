import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { OversightSession, oversightSession, serverFetch } from './oversight';
import type { PublishBundle, PublishJob } from './publish-engine';
const server = 'https://database.example.test';
beforeEach(() => { vi.stubGlobal('confirm', () => false); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); oversightSession.clear(); oversightSession.set(null); });
it('asks once per project in a session, never persists or adopts shell onBehalf as consent', async () => {
  const session = new OversightSession(); const ask = vi.fn(() => true);
  session.set({ viewAs: null, onBehalf: true });
  expect(session.allowed('a')).toBe(false);
  expect(await session.consent('a', 'Robin Example', ask)).toBe(true);
  await session.consent('a', 'Robin Example', ask); expect(ask).toHaveBeenCalledTimes(1);
  await session.consent('b', 'Robin Example', ask); expect(ask).toHaveBeenCalledTimes(2);
  session.clear(); expect(session.allowed('a')).toBe(false);
});
it('cancel never grants consent, concurrent writes share one prompt, and sign-out cancels a pending grant', async () => {
  const session = new OversightSession();
  expect(await session.consent('a', 'Robin', () => false)).toBe(false);
  let answer!: (yes: boolean) => void;
  const ask = vi.fn(() => new Promise<boolean>(resolve => answer = resolve));
  const a = session.consent('a', 'Robin', ask), b = session.consent('a', 'Robin', ask);
  session.clear(); answer(true); expect(await a).toBe(false); expect(await b).toBe(false); expect(ask).toHaveBeenCalledTimes(1);
});
for (const path of ['/v1/mounts/mount/sync', '/v1/atlas/projects/project/link', '/v1/mounts/mount/atlas', '/v1/mounts/mount/spots']) {
  it(`retries ${path} only after confirmation and scopes the header to the project and token`, async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => new Headers(init?.headers).has('X-UPbDB-Oversight') ? Response.json({ ok: true }) : Response.json({ code: 'oversight_confirm', owner: 'Robin Example' }, { status: 403 }));
    const ask = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
    const init = { method: 'POST', headers: { Authorization: 'Bearer invented-token' }, body: 'unchanged' };
    await serverFetch(server + path, init, 'project-a');
    expect(ask).toHaveBeenCalledWith("This mount is Robin Example's. Sync on their behalf?");
    expect(fetch).toHaveBeenCalledTimes(2); expect(fetch.mock.calls.every(([, value]) => value?.body === 'unchanged')).toBe(true);
    await serverFetch(server + path, init, 'project-a'); expect(ask).toHaveBeenCalledTimes(1);
    await serverFetch(server + path, init, 'project-b'); expect(ask).toHaveBeenCalledTimes(2);
    await serverFetch(server + path, { ...init, headers: { Authorization: 'Bearer other-token' } }, 'project-a'); expect(ask).toHaveBeenCalledTimes(3);
  });
}
it('older shells keep ordinary collaboration free of confirmation', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(Response.json({ ok: true }));
  await serverFetch(server + '/v1/mounts/m/sync', { method: 'POST' }, 'project');
  expect(new Headers(fetch.mock.calls[0][1]?.headers).has('X-UPbDB-Oversight')).toBe(false);
});
it('view-as blocks all server writes before fetch and sends as only on reads', async () => {
  oversightSession.set({ viewAs: { username: 'robin', name: 'Robin Example' }, onBehalf: false });
  const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(Response.json([]));
  for (const path of ['/v1/mounts', '/v1/mounts/m/sync', '/v1/atlas/projects/p/link', '/v1/mounts/m/atlas', '/v1/mounts/m/spots']) await expect(serverFetch(server + path, { method: 'POST' }, 'project')).rejects.toThrow('read-only');
  expect(fetch).not.toHaveBeenCalled();
  await serverFetch(server + '/v1/mounts?limit=50'); expect(new URL(String(fetch.mock.calls[0][0])).searchParams.get('as')).toBe('robin');
  await serverFetch(server + '/v1/atlas/projects/revisions', { method: 'POST' }); expect(new URL(String(fetch.mock.calls[1][0])).searchParams.get('as')).toBe('robin');
  await serverFetch(server + '/v1/assets/check', { method: 'POST', body: '{"sha256":[]}' }); expect(new URL(String(fetch.mock.calls[2][0])).searchParams.get('as')).toBe('robin');
});
it('a view-as change while a prompt is pending prevents the write', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(Response.json({ code: 'oversight_confirm', owner: 'Robin' }, { status: 403 }));
  vi.stubGlobal('confirm', () => { oversightSession.set({ viewAs: { username: 'robin', name: 'Robin' }, onBehalf: false }); return true; });
  await serverFetch(server + '/v1/mounts/m/sync', { method: 'POST' }, 'project');
  expect(fetch).toHaveBeenCalledTimes(1); vi.unstubAllGlobals();
});
it('cancel leaves the write refused and adds no header to the next attempt', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => Response.json({ code: 'oversight_confirm', owner: 'Robin' }, { status: 403 }));
  const ask = vi.spyOn(globalThis, 'confirm').mockReturnValue(false);
  const init = { method: 'POST', headers: { Authorization: 'Bearer invented-token' } };
  expect((await serverFetch(server + '/v1/mounts/m/sync', init, 'project')).status).toBe(403);
  await serverFetch(server + '/v1/mounts/m/sync', init, 'project'); expect(ask).toHaveBeenCalledTimes(2);
  expect(fetch).toHaveBeenCalledTimes(2); expect(fetch.mock.calls.every(([, init]) => !new Headers(init?.headers).has('X-UPbDB-Oversight'))).toBe(true);
});
it('link, sync and publish use the same project consent without changing ownership fields', async () => {
  const { SyncApi } = await import('./sync-api');
  const { publishAtlas } = await import('./upbdb');
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => new Headers(init?.headers).get('X-UPbDB-Oversight') === 'on-behalf' ? Response.json({ revision: 4 }) : Response.json({ code: 'oversight_confirm', owner: 'Robin Example' }, { status: 403 }));
  const ask = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
  const api = new SyncApi(server, 'invented-token');
  await api.link('project', 'mount');
  const empty = { upsert: [], delete: [] };
  await api.sync('mount', { project_uuid: 'project', base_revision: 3, mode: 'snapshot', spots: empty, labels: empty, grains: { ...empty, retire: [] } });
  await publishAtlas(server, 'invented-token', 'mount', 'project', new Uint8Array([0]));
  expect(ask).toHaveBeenCalledTimes(1); expect(fetch).toHaveBeenCalledTimes(4);
  expect(JSON.parse(String(fetch.mock.calls[1][1]?.body))).toEqual({ mount_id: 'mount', make_current: false });
  for (const [, init] of fetch.mock.calls.slice(1)) expect(new Headers(init?.headers).get('X-UPbDB-Oversight')).toBe('on-behalf');
});

it('oversight keeps the exact sync body and durable retry-key renewal', async () => {
  const { SyncApi } = await import('./sync-api');
  const ask = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
  const keyChanged = vi.fn();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => {
    const headers = new Headers(init?.headers);
    if (!headers.has('X-UPbDB-Oversight')) return Response.json({ code: 'oversight_confirm', owner: 'Robin' }, { status: 403 });
    if (headers.get('Idempotency-Key') === 'saved-key') return Response.json({ code: 'idempotency_key_reused' }, { status: 422 });
    return Response.json({ revision: 4 });
  });
  const empty = { upsert: [], delete: [] };
  await new SyncApi(server, 'invented-token').sync('mount', {
    project_uuid: 'project', base_revision: 3, mode: 'delta', spots: empty, labels: empty, grains: { ...empty, retire: [] },
  }, 'saved-key', keyChanged, 'saved-wire-id');
  expect(ask).toHaveBeenCalledTimes(1);
  expect(fetch).toHaveBeenCalledTimes(3);
  const requests = fetch.mock.calls.map(([, init]) => init!);
  expect(new Set(requests.map(init => init.body)).size).toBe(1);
  expect(JSON.parse(String(requests[0].body)).request_id).toBe('saved-wire-id');
  expect(new Headers(requests[1].headers).get('Idempotency-Key')).toBe('saved-key');
  expect(keyChanged).toHaveBeenCalledExactlyOnceWith(new Headers(requests[2].headers).get('Idempotency-Key'));
});

it('sync ETags remain separate for the signed-in user and view-as identity', async () => {
  const { SyncApi } = await import('./sync-api');
  const { clearSyncCache } = await import('./sync-cache');
  clearSyncCache();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url, init) => {
    const user = new URL(String(url)).searchParams.get('as') ?? 'self';
    if (new Headers(init?.headers).get('If-None-Match') === `"${user}"`) return new Response(null, { status: 304 });
    return Response.json({ name: user }, { headers: { ETag: `"${user}"` } });
  });
  const api = new SyncApi(server, 'invented-token');
  expect((await api.lookup('project'))?.name).toBe('self');
  oversightSession.set({ viewAs: { username: 'robin', name: 'Robin' }, onBehalf: false });
  expect((await api.lookup('project'))?.name).toBe('robin');
  expect(new Headers(fetch.mock.calls[1][1]?.headers).has('If-None-Match')).toBe(false);
  expect((await api.lookup('project'))?.name).toBe('robin');
  oversightSession.set(null);
  expect((await api.lookup('project'))?.name).toBe('self');
  expect(new Headers(fetch.mock.calls[3][1]?.headers).get('If-None-Match')).toBe('"self"');
  expect(fetch).toHaveBeenCalledTimes(4);
  clearSyncCache();
});

it('oversight preserves the upstream fail-fast deadline for stalled database reads', async () => {
  const { SyncApi, SyncNetworkError } = await import('./sync-api');
  vi.useFakeTimers();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(() => new Promise(() => {}));
  const result = new SyncApi(server, 'invented-token').lookup('stalled-project');
  const refused = expect(result).rejects.toBeInstanceOf(SyncNetworkError);
  await vi.advanceTimersByTimeAsync(8000);
  await refused;
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(fetch.mock.calls[0][1]?.signal?.aborted).toBe(true);
});

it('4,000 mocked candidates keep one request with oversight on the upstream transport', async () => {
  const { SyncApi } = await import('./sync-api');
  const { fetchWithTimeout } = await import('./http');
  const rows = Array.from({ length: 4000 }, (_, i) => ({ project_uuid: `invented-${i}`, mount_label: `Synthetic ${i}` }));
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => Response.json(rows));
  const url = server + '/v1/atlas/projects?package=Synthetic';
  const before = performance.now();
  expect(await (await fetchWithTimeout(url)).json()).toEqual(rows);
  const baseline = performance.now() - before;
  expect(fetch).toHaveBeenCalledTimes(1);
  const after = performance.now();
  expect(await new SyncApi(server, 'invented-token').candidates('Synthetic')).toEqual(rows);
  const combined = performance.now() - after;
  expect(fetch).toHaveBeenCalledTimes(2);
  console.info(`4,000 candidates: upstream transport 1 request / ${baseline.toFixed(2)} ms; combined 1 request / ${combined.toFixed(2)} ms (mock processing).`);
});

it('resumable publishing shares project consent and keeps deduplication, keys and resume status', async () => {
  const { uploadImages } = await import('./publish-engine');
  const base = 'https://resumable-oversight.example.test';
  const hash = 'a'.repeat(64);
  const bundle = { atlas: { project_id: 'project', files: [{ path: 'a.webp', sha256: hash }, { path: 'b.webp', sha256: hash }] },
    files: [{ path: 'a.webp', bytes: new Uint8Array([0]) }, { path: 'b.webp', bytes: new Uint8Array([0]) }], atlasSha256: 'manifest', zip: new Uint8Array([0]) } as PublishBundle;
  let job: PublishJob = { server: base, mount: 'mount', account: 'account', atlas_sha256: 'manifest', start_key: 'start', commit_key: 'commit', zip_key: 'zip',
    exported: '2026-01-01T00:00:00Z', pixels_key: 'pixels', uploaded: 0, replace_current: false };
  const ask = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url, init) => {
    const path = new URL(String(url)).pathname;
    if (path === '/v1/assets/check') return Response.json({ have: [] });
    if (!init?.method) return Response.json({ state: 'open', missing: [], received: 2, total: 2 }, { headers: { ETag: '"upload"' } });
    if (!new Headers(init.headers).has('X-UPbDB-Oversight')) return Response.json({ code: 'oversight_confirm', owner: 'Robin' }, { status: 403 });
    if (path.endsWith('/atlas/uploads')) return Response.json({ upload_id: 'upload', expires_at: '2099-01-01T00:00:00Z', missing: [hash] }, { status: 201 });
    return Response.json(path.endsWith('/commit') ? { version: 2 } : { stored: true });
  });
  const run = () => uploadImages({ server: base, mount: 'mount', token: 'invented-token', bundle, job,
    save: value => { job = value; }, gate: async () => {}, progress: () => {} });
  expect(await run()).toEqual({ version: 2 });
  expect(fetch).toHaveBeenCalledTimes(5); // Four upstream requests plus one consent retry.
  const starts = fetch.mock.calls.filter(([url]) => String(url).endsWith('/atlas/uploads'));
  expect(starts).toHaveLength(2);
  expect(starts[0][1]?.body).toBe(starts[1][1]?.body);
  for (const [, init] of starts) expect(new Headers(init?.headers).get('Idempotency-Key')).toBe('start');
  const parts = fetch.mock.calls.filter(([, init]) => init?.method === 'PUT');
  expect(parts).toHaveLength(1);
  expect(job.uploaded).toBe(2);
  await run();
  expect(fetch).toHaveBeenCalledTimes(7); // Resume status and commit; no new blobs or manifest.
  expect(ask).toHaveBeenCalledTimes(1);
  for (const [url, init] of fetch.mock.calls.slice(2)) if (init?.method) {
    expect(new Headers(init.headers).get('X-UPbDB-Oversight')).toBe('on-behalf');
    if (String(url).endsWith('/commit')) expect(new Headers(init.headers).get('Idempotency-Key')).toBe('commit');
  }
});

it('a view-as change stops resumable writes between requests without a network retry', async () => {
  const { uploadImages } = await import('./publish-engine');
  const base = 'https://view-as-upload.example.test';
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url) => {
    if (String(url).endsWith('/v1/assets/check')) return Response.json({ have: [] });
    oversightSession.set({ viewAs: { username: 'robin', name: 'Robin' }, onBehalf: false });
    return Response.json({ upload_id: 'upload', expires_at: '2099-01-01T00:00:00Z', missing: [] });
  });
  const wait = vi.fn();
  await expect(uploadImages({ server: base, mount: 'mount', token: 'invented-token',
    bundle: { atlas: { project_id: 'project', files: [] }, files: [], zip: new Uint8Array(), atlasSha256: 'manifest' } as unknown as PublishBundle,
    job: { start_key: 'start', commit_key: 'commit' } as PublishJob,
    save: () => {}, gate: async () => {}, progress: () => {}, wait,
  })).rejects.toThrow('read-only');
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(wait).not.toHaveBeenCalled();
});
