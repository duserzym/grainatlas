import { afterEach, expect, it, vi } from 'vitest';
import { hideMountCache, recentRevisions } from './mount-browser';
import { startSyncPolling } from './sync-scheduling';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); hideMountCache(); });

it('measures manual versus automatic scheduling with 1,001 mocked project revisions', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  const ids = Array.from({ length: 1001 }, (_, i) => `synthetic-project-${i}`);
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_input, init) => {
    const projects = JSON.parse(String(init?.body)).projects as string[];
    expect(projects.length).toBeLessThanOrEqual(500);
    const etag = `batch-${projects[0]}`;
    const previous = new Headers(init?.headers).get('If-None-Match');
    if (previous) { expect(previous).toBe(etag); return new Response(null, { status: 304 }); }
    return Response.json(projects.map(project_uuid => ({ project_uuid, mount_id: 'synthetic-mount', revision: 1 })), { headers: { ETag: etag } });
  });
  async function measure(automatic: boolean) {
    hideMountCache(); fetch.mockClear();
    const start = performance.now();
    let cached = await recentRevisions('https://scale.example.test', 'synthetic-token', 'synthetic-account', ids, undefined, () => true);
    const firstLoadRequests = fetch.mock.calls.length;
    const jobs: Promise<unknown>[] = [];
    const stop = startSyncPolling(automatic, 120_000, () => {
      jobs.push(recentRevisions('https://scale.example.test', 'synthetic-token', 'synthetic-account', ids, cached, () => true).then(next => { cached = next; }));
    });
    await vi.advanceTimersByTimeAsync(300_000);
    await Promise.all(jobs);
    stop();
    return { firstLoadRequests, periodicRequests: fetch.mock.calls.length - firstLoadRequests, wallMs: Number((performance.now() - start).toFixed(2)) };
  }
  const before = await measure(true), manual = await measure(false);
  expect(before.firstLoadRequests).toBe(3);
  expect(before.periodicRequests).toBe(6);
  expect(manual.firstLoadRequests).toBe(3);
  expect(manual.periodicRequests).toBe(0);
  console.info(JSON.stringify({ manualSyncScale: 1001, simulatedMinutes: 5, before, manual }));
});


it('explicit Refresh bypasses revision freshness while retaining conditional revalidation', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_input, init) =>
    new Headers(init?.headers).has('If-None-Match') ? new Response(null, { status: 304 })
      : Response.json([{ project_uuid: 'synthetic-project', mount_id: 'synthetic-mount', revision: 1 }], { headers: { ETag: 'synthetic-revision-tag' } }));
  const first = await recentRevisions('https://refresh.example.test', 'synthetic-token', 'account', ['synthetic-project'], undefined, () => true);
  await recentRevisions('https://refresh.example.test', 'synthetic-token', 'account', ['synthetic-project'], first, () => true);
  expect(fetch).toHaveBeenCalledTimes(1);
  const refreshed = await recentRevisions('https://refresh.example.test', 'synthetic-token', 'account', ['synthetic-project'], first, () => true, true);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(new Headers(fetch.mock.calls[1][1]?.headers).get('If-None-Match')).toBe('synthetic-revision-tag');
  expect(refreshed.batches[0].rows).toEqual(first.batches[0].rows);
});
