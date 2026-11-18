import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { browserAccount, accountLabs, defaultScope, mirrorServer, serverMirrorId, batchRevisions, recentRevisionIds, recentRevisions, cacheKey, hideMountCache, observeServerToken, localMatch, mountPage, mountPath, readCache, tileBadges, writeCache, CACHE_BYTES } from './mount-browser';
import type { MountOut } from './upbdb';
import type { Draft } from './drafts';
import { pixelSignature } from './sync-pixels';
import { DEFAULT_LAYOUT, loadLayout, saveLayout, resetLayoutStorage } from './layout-prefs';

function storage() {
  const map = new Map<string, string>();
  return { get length() { return map.size; }, key: (i: number) => [...map.keys()][i] ?? null, getItem: (k: string) => map.get(k) ?? null, setItem: (k: string, v: string) => { map.set(k, v); }, removeItem: (k: string) => { map.delete(k); } };
}
const server = 'https://database.example.test';
const mount: MountOut = { id: '20000000-0000-4000-8000-000000000001', label: 'Invented mount', mount_type: 'grain_tape', can_edit: true, grain_count: 4, spot_count: 6, creator: { id: 'owner', name: 'Robin Example' }, role: 'write', atlas: { project_id: '10000000-0000-4000-8000-000000000001', thumbnail: null } };
const draft: Draft = { uuid: mount.atlas!.project_id, name: 'Invented project', server, mount: mount.id, base_revision: 3, base: null, updated_at: '2026-01-02T00:00:00Z', changes: { grains: 1, spots: 0, labels: 0 }, source_keys: ['grainatlas.project.Invented project'] };
const recent = [{ id: draft.uuid, name: draft.name, areas: 1, spots: 6, when: draft.updated_at }];
beforeEach(() => hideMountCache());
afterEach(() => vi.restoreAllMocks());

describe('tile matching and badges', () => {
  it('matches a local UUID or linked draft without using a label', () => {
    expect(localMatch(mount, recent, [], server)?.recent).toEqual(recent[0]);
    expect(localMatch({ ...mount, atlas: null }, [], [draft], server)?.draft).toEqual(draft);
    expect(localMatch({ ...mount, atlas: null }, [{ ...recent[0], id: 'different' }], [], server)).toBeNull();
    expect(localMatch({ ...mount, atlas: null }, [], [draft], 'https://other.example.test')).toBeNull();
    expect(localMatch({ ...mount, id: mount.id.toUpperCase() }, [], [draft], server)).not.toBeNull();
  });
  it('compares the batched revision with the local base and shows explicit roles', () => {
    expect(tileBadges(mount, { draft }, 'viewer', 4)).toEqual(['On this computer', 'Local changes', 'Newer on server', 'Shared · Write']);
    expect(tileBadges(mount, { draft: { ...draft, changes: null, synced: true } }, 'owner', 3)).toEqual(['On this computer']);
    expect(tileBadges({ ...mount, role: 'read' }, null, 'viewer')).toEqual(['Not on this computer', 'Shared · Read']);
    expect(tileBadges({ ...mount, role: 'admin' }, null, 'viewer')).toContain('Shared · Admin');
  });
  it('a stale pixels_changed flag in an otherwise synced draft is not local changes', () => {
    const project = { uuid: draft.uuid, package: { name: 'Invented project' }, settings: {}, upbdb: { mount_id: mount.id },
      areas: [{ id: 'a', mosaic: { sha256: 'invented-sha', w: 10, h: 8, um_per_px: 2 }, params: { tone: { low_pct: 0, gamma: 0.8 } } }],
      grainatlas_ui: { tone: {}, spots: [], grains: {}, labels: [] } } as unknown as Parameters<typeof pixelSignature>[0];
    const key = pixelSignature(project);
    const synced = { ...draft, changes: { grains: 0, spots: 0, labels: 0 }, pending: null, synced: true, pixels_base: key, pixels_changed: true } as Draft;
    // A newer draft records the key it compared: it matches, so the old flag is stale.
    expect(tileBadges(mount, { draft: { ...synced, pixels_key: key } }, 'owner', 3)).toEqual(['On this computer']);
    // A legacy draft without a key: the stored project copy matches the published images.
    const store = storage();
    store.setItem(draft.source_keys![0], JSON.stringify(project));
    expect(tileBadges(mount, { draft: synced }, 'owner', 3, store)).toEqual(['On this computer']);
    // A real image change still counts.
    const changed = pixelSignature({ ...project, areas: [{ ...project.areas[0], mosaic: { ...project.areas[0].mosaic!, sha256: 'other-sha' } }] });
    expect(tileBadges(mount, { draft: { ...synced, pixels_key: changed } }, 'owner', 3)).toEqual(['On this computer', 'Local changes']);
  });
});
describe('private, bounded offline cache', () => {
  it('keys by normalized server or dataset and account and keeps mirrors and unsynced work at sign-out', async () => {
    const s = storage();
    const dataset = await serverMirrorId(server + '/');
    expect(dataset).toBe(await serverMirrorId('https://DATABASE.example.test:443/?ignored=1#fragment'));
    expect(mirrorServer(server + '/')).toBe(server);
    const a = cacheKey(dataset, 'a'), b = cacheKey(dataset, 'b');
    expect(a).toBe(`mirror/${dataset}/a/start-screen.json`);
    expect(a).not.toBe(b); expect(a).not.toBe(cacheKey(await serverMirrorId(server + '/alternate'), 'a'));
    await writeCache(s, a, { pages: { first: { rows: [mount], etag: 'tag', at: draft.updated_at, next: 'cursor', total: 9000 } } });
    expect((await readCache(s, b)).pages).toEqual({});
    s.setItem('grainatlas.project.Local', 'unsynced edits'); s.setItem('grainatlas.draft.Local', 'pending request');
    hideMountCache();
    expect((await readCache(s, a)).pages.first.rows).toEqual([mount]);
    expect(s.getItem('grainatlas.project.Local')).toBe('unsynced edits');
    expect(s.getItem('grainatlas.draft.Local')).toBe('pending request');
  });
  it('caps thousands of rows at 500 and evicts thumbnails before exceeding 20 MB', async () => {
    const s = storage(), key = cacheKey(server, 'a');
    await writeCache(s, key, { pages: { first: { rows: Array.from({ length: 4000 }, (_, i) => ({ ...mount, id: String(i), thumbnail_data: 'x'.repeat(45000) })), etag: null, at: draft.updated_at, next: null, total: 4000 } } });
    const rows = (await readCache(s, key)).pages.first.rows;
    expect(rows).toHaveLength(500); expect(rows.every(r => !r.thumbnail_data)).toBe(true);
    expect(new TextEncoder().encode(s.getItem(key)!).length).toBeLessThanOrEqual(CACHE_BYTES);
  });
  it('survives damaged storage and unavailable quota', async () => {
    const s = storage(); s.setItem('broken', '{'); expect(await readCache(s, 'broken')).toEqual({ pages: {} });
    await expect(writeCache({ ...s, setItem() { throw new Error('quota'); } }, 'key', { pages: {} })).resolves.toBeUndefined();
  });
});
describe('layout storage', () => {
  it('round trips tile size and split alongside existing geometry, and resets everything', () => {
    const s = storage(); saveLayout(s, { ...DEFAULT_LAYOUT, panel: { x: 42, y: 52 }, recentWidth: 320, tileSize: 'L' });
    expect(loadLayout(s)).toMatchObject({ recentWidth: 320, tileSize: 'L', panel: { x: 42, y: 52 } });
    expect(resetLayoutStorage(s)).toEqual(DEFAULT_LAYOUT); expect(loadLayout(s)).toEqual(DEFAULT_LAYOUT);
    s.setItem('grainatlas.layout', JSON.stringify({ recentWidth: 900, tileSize: 'invalid' })); expect(loadLayout(s)).toMatchObject({ recentWidth: 360, tileSize: 'M' });
  });
});
describe('bounded reads', () => {
  it('requests one 50-row page out of 4000, follows the opaque cursor, and revalidates with ETag', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
      const u = new URL(String(input));
      expect(u.searchParams.get('limit')).toBe('50');
      if (new Headers(init?.headers).get('If-None-Match')) return new Response(null, { status: 304 });
      const from = u.searchParams.has('after') ? 50 : 0;
      return Response.json(Array.from({ length: 50 }, (_, i) => ({ ...mount, id: String(from + i) })), { headers: { ETag: 'page-tag', 'X-Next-Cursor': 'opaque-next', 'X-Total-Count': '4000' } });
    });
    const started = performance.now();
    const first = await mountPage(server, 'token', mountPath({ mineral: 'zircon', sort: '-last_changed' }));
    expect(first.rows).toHaveLength(50); expect(first.total).toBe(4000); expect(fetch).toHaveBeenCalledTimes(1);
    await mountPage(server, 'token', mountPath({}, first.next));
    const current = await mountPage(server, 'token', mountPath({ mineral: 'zircon', sort: '-last_changed' }), first);
    expect(current.rows).toEqual(first.rows); expect(fetch).toHaveBeenCalledTimes(3);
    console.info(`Paged mock: ${fetch.mock.calls.length} requests, 100 rows transferred out of 4000, ${Math.round(performance.now() - started)} ms (legacy list: 1 request, 4000 rows).`);
  });
  it('checks all 4000 UUIDs in batches of at most 500 without dropping projects', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => Response.json([]));
    const ids = Array.from({ length: 4000 }, (_, i) => `10000000-0000-4000-8000-${String(i).padStart(12, '0')}`);
    const start = performance.now();
    const result = await recentRevisions(server, 'token', 'account', ids, undefined, () => true);
    expect(fetch).toHaveBeenCalledTimes(8);
    const sent = fetch.mock.calls.flatMap(([, init]) => {
      const projects = JSON.parse(String(init?.body)).projects;
      expect(projects).toHaveLength(500); return projects;
    });
    expect(sent).toEqual(ids);
    await recentRevisions(server, 'token', 'account', ids, result, () => true);
    expect(fetch).toHaveBeenCalledTimes(8);
    console.info(JSON.stringify({ revisionProjects: 4000, before: { requests: 1, checked: 500, dropped: 3500 }, after: { requests: 8, checked: sent.length, revisitRequests: 0, ms: performance.now() - start } }));
    await expect(batchRevisions(server, 'token', ids)).rejects.toThrow('at most 500');
  });
  it('sends all combined filters to the server and preserves cursor encoding', () => {
    const q = new URL(server + mountPath({ mine: 'true', lab: 'invented-lab', shared: 'true', q: 'mount', mineral: 'zircon', sample: 'Example', creator: 'robin', date_field: 'session', date_from: '2026-01-01', date_to: '2026-02-01', sort: '-session' }, 'opaque+/=' )).searchParams;
    expect(q.get('after')).toBe('opaque+/='); expect(q.get('date_field')).toBe('session'); expect(q.get('creator')).toBe('robin');
  });
});

it('reuses session identity, revalidates lab metadata, supports offline sign-in, and refuses revoked credentials', async () => {
  const { browserAccount } = await import('./mount-browser');
  const s = storage();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
    if (new Headers(init?.headers).get('If-None-Match')) return new Response(null, { status: 304 });
    if (String(input).includes('/v1/server')) return Response.json({});
    return Response.json(String(input).includes('/accounts/me') ? { id: 'invented-user', username: 'casey', display_name: 'Casey Example', can_manage_people: false } : [{ id: 'lab', handle: 'invented-lab' }], { headers: { ETag: 'identity-tag' } });
  });
  const me = await browserAccount(server, 'invented-token', s); expect(me.labs[0].handle).toBe('invented-lab');
  expect(await browserAccount(server, 'invented-token', s)).toEqual(me); expect(fetch).toHaveBeenCalledTimes(4);
  hideMountCache(); fetch.mockRejectedValue(new Error('offline')); expect(await browserAccount(server, 'invented-token', s)).toEqual(me);
  hideMountCache(); fetch.mockResolvedValue(Response.json({ detail: 'Revoked' }, { status: 401 })); await expect(browserAccount(server, 'invented-token', s)).rejects.toThrow('401');
  expect(s.length).toBeGreaterThan(0);
});
it('a sign-out during an identity request never repopulates the cache', async () => {
  const { browserAccount } = await import('./mount-browser');
  const s = storage(); let release!: () => void;
  vi.spyOn(globalThis, 'fetch').mockImplementation(async input => {
    if (String(input).includes('/accounts/me')) { await new Promise<void>(resolve => { release = resolve; }); return Response.json({ id: 'invented-user', groups: [] }); }
    return Response.json([]);
  });
  const pending = browserAccount(server, 'invented-token', s);
  await vi.waitFor(() => expect(release).toBeTypeOf('function'));
  hideMountCache(); release(); await expect(pending).rejects.toThrow('account changed'); expect(s.length).toBe(0);
});
it('compares transfer counts and time against the old unbounded list on a 4000-row mock', async () => {
  const { readResponse } = await import('./mount-browser');
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async input => {
    const q = new URL(String(input)).searchParams;
    return Response.json(Array.from({ length: Number(q.get('limit') ?? 4000) }, (_, i) => ({ ...mount, id: String(i) })));
  });
  const before = performance.now(); const all = await (await readResponse(server + '/v1/mounts')).json(); const beforeMs = performance.now() - before;
  expect(fetch).toHaveBeenCalledTimes(1); fetch.mockClear();
  const after = performance.now(); const page = await mountPage(server, 'token', mountPath({})); const afterMs = performance.now() - after;
  expect(fetch).toHaveBeenCalledTimes(1); expect(all).toHaveLength(4000); expect(page.rows).toHaveLength(50);
  console.info(JSON.stringify({ legacy: { requests: 1, rows: all.length, ms: beforeMs }, paged: { requests: 1, rows: page.rows.length, ms: afterMs } }));
});

it('shows the same account immediately on a new sign-in before revalidation, never another account or server', async () => {
  const s = storage();
  let id = 'a';
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async input => {
    const url = String(input);
    if (url.includes('/accounts/me')) return Response.json({ id, username: id, can_manage_people: false });
    if (url.includes('/v1/server')) return Response.json({ dataset_id: url.startsWith(server + '/') ? 'dataset-a' : 'dataset-b' });
    return Response.json([{ id: 'lab', handle: 'lab' }]);
  });
  const a = await browserAccount(server, 'token-a', s);
  const key = cacheKey(a.mirrorId, a.id);
  const cache = await readCache(s, key);
  await writeCache(s, key, { ...cache, pages: { first: { rows: [mount], at: draft.updated_at, etag: 'tag', next: null, total: 1 } } });
  hideMountCache();
  const shown = vi.fn();
  await expect(browserAccount(server, '', s, shown)).rejects.toThrow('Sign in');
  expect(shown).not.toHaveBeenCalled();
  const reads = vi.spyOn(s, 'getItem');
  id = 'b'; await browserAccount(server, 'token-b', s, shown);
  expect(reads.mock.calls.some(([path]) => path === key)).toBe(false);
  reads.mockClear();
  expect(shown).not.toHaveBeenCalled();
  id = 'a'; await browserAccount('https://other.example.test', 'other-token', s, shown);
  expect(shown).not.toHaveBeenCalled();
  expect(reads.mock.calls.some(([path]) => path === key)).toBe(false);
  // A fresh token is resolved to /me, then the account folder is shown while /labs is pending.
  let release!: () => void;
  fetch.mockImplementation(async input => {
    if (String(input).includes('/accounts/me')) return Response.json({ id: 'a', username: 'a', can_manage_people: false });
    if (String(input).includes('/v1/server')) return Response.json({ dataset_id: 'dataset-a' });
    await new Promise<void>(resolve => { release = resolve; }); return Response.json([]);
  });
  const signIn = browserAccount(server, 'new-token-a', s, shown);
  await vi.waitFor(() => expect(release).toBeTypeOf('function'));
  expect(shown).toHaveBeenCalledWith(expect.objectContaining({ id: 'a' }), expect.objectContaining({ pages: { first: expect.objectContaining({ rows: [mount] }) } }));
  release(); await signIn;
  hideMountCache(); fetch.mockRejectedValue(new Error('offline'));
  expect((await browserAccount(server, 'new-token-a', s)).id).toBe('a');
  await expect(browserAccount(server, 'unknown-token', s)).rejects.toThrow('offline');
});

it('migrates the URL-hash mirror when dataset_id appears and reads it through a standby', async () => {
  const s = storage(); let dataset: string | undefined;
  vi.spyOn(globalThis, 'fetch').mockImplementation(async input => Response.json(
    String(input).includes('/accounts/me') ? { id: 'a', can_manage_people: false } : String(input).includes('/v1/server') ? { dataset_id: dataset } : []));
  const first = await browserAccount(server, 'token', s);
  expect(first.mirrorId).toBe(await serverMirrorId(server));
  const old = await readCache(s, cacheKey(first.mirrorId, 'a'));
  await writeCache(s, cacheKey(first.mirrorId, 'a'), { ...old, pages: { first: { rows: [mount], etag: null, at: draft.updated_at, next: null, total: 1 } } });
  hideMountCache(); dataset = 'dataset-a';
  const second = await browserAccount(server, 'token', s);
  expect(second.mirrorId).toBe(dataset);
  const shown = vi.fn(); await browserAccount('https://standby.example.test', 'standby-token', s, shown);
  expect(shown).toHaveBeenCalledWith(expect.objectContaining({ id: 'a', mirrorId: dataset }), expect.objectContaining({ pages: { first: expect.objectContaining({ rows: [mount] }) } }));
  hideMountCache(); dataset = 'different-dataset';
  const changed = vi.fn(); await browserAccount(server, 'token', s, changed);
  expect(changed.mock.calls.filter(([me]) => me.mirrorId === dataset)).toHaveLength(0);
  expect((await readCache(s, cacheKey(dataset, 'a'))).pages).toEqual({});
});

for (const superuser of [false, true]) for (const roles of [false, true]) {
  it(`uses /labs membership for ${superuser ? 'superuser' : 'ordinary account'}, my_role ${roles ? 'present' : 'absent'}`, async () => {
    const labs = [
      { id: 'a', handle: 'alpha', ...(roles ? { my_role: 'owner' as const } : {}) },
      { id: 'b', handle: 'beta', ...(roles ? { my_role: 'member' as const } : {}) },
      { id: 'c', handle: 'gamma', ...(roles ? { my_role: null } : {}) },
    ];
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async input => Response.json(
      String(input).includes('/accounts/me') ? { id: 'a', can_manage_people: superuser, groups: [{ lab_id: 'c' }] } : String(input).includes('/v1/server') ? {} : labs,
      { headers: { 'X-Next-Cursor': 'next-labs', ETag: 'labs-tag' } }));
    const start = performance.now();
    const me = await browserAccount(server, 'token', storage());
    expect(me.labs.map(l => l.handle)).toEqual(superuser ? roles ? ['alpha', 'beta'] : [] : ['alpha', 'beta', 'gamma']);
    expect(me.labsNext).toBe('next-labs');
    expect(defaultScope(me)).toEqual({ sort: '-last_changed', ...(superuser ? roles ? { lab: 'alpha' } : { mine: 'true' } : {}) });
    expect(accountLabs(me, labs)).toEqual(me.labs);
    expect(fetch).toHaveBeenCalledTimes(3);
    console.info(`Identity and lab chips: 3 requests, ${Math.round(performance.now() - start)} ms (before: 2 requests; added dataset discovery, no row requests).`);
  });
}

it('keeps lab discovery paged on a 4000-lab server and measures the extra dataset lookup', async () => {
  const { readResponse } = await import('./mount-browser');
  const labs = Array.from({ length: 4000 }, (_, i) => ({ id: String(i), handle: `lab-${i}`, my_role: i % 2 ? null : 'member' }));
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async input => {
    const u = new URL(String(input));
    if (u.pathname === '/v1/accounts/me') return Response.json({ id: 'a', can_manage_people: true, groups: [{ lab_id: '0' }] });
    if (u.pathname === '/v1/server') return Response.json({ dataset_id: 'dataset' });
    expect(u.pathname).toBe('/v1/labs'); expect(u.searchParams.get('limit')).toBe('100');
    return Response.json(labs.slice(0, 100), { headers: { 'X-Next-Cursor': 'opaque-next', ETag: 'lab-page-tag' } });
  });
  const before = performance.now();
  // Previous chip discovery used /me followed by the first lab page and team-lab filtering.
  const me = await (await readResponse(`${server}/v1/accounts/me`)).json();
  const oldLabs = await (await readResponse(`${server}/v1/labs?limit=100`)).json();
  expect(oldLabs.filter((lab: { id: string }) => me.groups.some((g: { lab_id: string }) => g.lab_id === lab.id))).toHaveLength(1);
  const beforeMs = performance.now() - before;
  expect(fetch).toHaveBeenCalledTimes(2); fetch.mockClear();
  const after = performance.now();
  const account = await browserAccount(server, 'token', storage());
  const afterMs = performance.now() - after;
  expect(account.labs).toHaveLength(50); expect(account.labsNext).toBe('opaque-next');
  expect(fetch).toHaveBeenCalledTimes(3);
  console.info(JSON.stringify({ chipDiscoveryBefore: { requests: 2, transferredLabs: 100, ms: beforeMs }, chipDiscoveryAfter: { requests: 3, transferredLabs: 100, ms: afterMs }, availableLabs: 4000 }));
});

it('a new credential carries over the same account URL mirror when it discovers dataset_id', async () => {
  const s = storage(); let dataset: string | undefined;
  vi.spyOn(globalThis, 'fetch').mockImplementation(async input => Response.json(String(input).includes('/accounts/me') ? { id: 'a' } : String(input).includes('/v1/server') ? { dataset_id: dataset } : []));
  const first = await browserAccount(server, 'old-token', s);
  const previous = await readCache(s, cacheKey(first.mirrorId, 'a'));
  await writeCache(s, cacheKey(first.mirrorId, 'a'), { ...previous, pages: { first: { rows: [mount], at: draft.updated_at, etag: null, next: null, total: 1 } } });
  hideMountCache(); dataset = 'dataset';
  const shown = vi.fn(); await browserAccount(server, 'new-token', s, shown);
  expect(shown).toHaveBeenCalledWith(expect.objectContaining({ mirrorId: 'dataset', id: 'a' }), expect.objectContaining({ pages: { first: expect.objectContaining({ rows: [mount] }) } }));
  expect((await readCache(s, cacheKey('dataset', 'a'))).pages.first.rows).toEqual([mount]);
});


it('coalesces concurrent identity reads and reuses them across screen, project and sync consumers', async () => {
  const s = storage();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async input => {
    const path = new URL(String(input)).pathname;
    return Response.json(path === '/v1/accounts/me' ? { id: 'a', can_manage_people: true } : path === '/v1/server' ? { dataset_id: 'dataset' } : []);
  });
  // Clearing only between baseline consumers reproduces the former uncached reads.
  const before = performance.now();
  for (let i = 0; i < 5; i++) { hideMountCache(); await browserAccount(server, 'token', s); }
  const beforeMs = performance.now() - before;
  expect(fetch).toHaveBeenCalledTimes(15);
  fetch.mockClear(); hideMountCache();
  const start = performance.now();
  const accounts = await Promise.all(Array.from({ length: 4 }, () => browserAccount(server, 'token', s)));
  expect(accounts.every(a => a.id === 'a' && a.can_manage_people && a.mirrorId === 'dataset')).toBe(true);
  await browserAccount(server + '/', 'token', s);
  const paths = fetch.mock.calls.map(([url]) => new URL(String(url)).pathname);
  expect(paths.filter(p => p === '/v1/accounts/me')).toHaveLength(1);
  expect(paths.filter(p => p === '/v1/server')).toHaveLength(1);
  expect(paths.filter(p => p === '/v1/labs')).toHaveLength(5);
  console.info(JSON.stringify({ identityConsumers: 5, uncached: { identityRequests: 10, totalRequests: 15, ms: beforeMs }, cached: { identityRequests: 2, totalRequests: fetch.mock.calls.length, ms: performance.now() - start } }));
});

it('isolates servers and credentials and refreshes on sign-out, including signing in with the same token', async () => {
  const s = storage();
  const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => Response.json(
    String(input).includes('/accounts/me') ? { id: new Headers(init?.headers).get('Authorization') } : String(input).includes('/v1/server') ? { dataset_id: String(input) } : []));
  const token = 'session-token';
  observeServerToken(server, token);
  const a = await browserAccount(server, token, s);
  const other = 'https://separate.example.test';
  await browserAccount(other, token, s);
  await browserAccount(server, token, s);
  expect(fetch.mock.calls.filter(([u]) => String(u).includes('/accounts/me'))).toHaveLength(2);
  observeServerToken(other, token); observeServerToken(other, 'replacement');
  await browserAccount(other, 'replacement', s);
  expect(await browserAccount(server, token, s)).toEqual(a);
  expect(fetch.mock.calls.filter(([u]) => String(u).includes('/accounts/me'))).toHaveLength(3);
  hideMountCache();
  await browserAccount(server, token, s);
  expect(fetch.mock.calls.filter(([u]) => String(u).includes('/accounts/me'))).toHaveLength(4);
});

it('skips identity reads without a token or configured server and retains failed reads for this sign-in', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('offline'));
  const s = storage();
  await expect(browserAccount(server, '', s)).rejects.toThrow('Sign in');
  await expect(browserAccount('', 'token', s)).rejects.toThrow('server first');
  expect(fetch).not.toHaveBeenCalled();
  for (let i = 0; i < 3; i++) await expect(browserAccount(server, 'token', s)).rejects.toThrow('offline');
  expect(fetch).toHaveBeenCalledTimes(2);
  hideMountCache();
  fetch.mockResolvedValue(Response.json({ detail: 'Revoked' }, { status: 401 }));
  for (let i = 0; i < 3; i++) await expect(browserAccount(server, 'token', s)).rejects.toThrow('401');
  expect(fetch).toHaveBeenCalledTimes(4);
});

describe('start-screen revision freshness', () => {
  const visible = () => true;
  const row = { project_uuid: draft.uuid, mount_id: mount.id, revision: 4 };
  it('selects only recent projects linked to this server, including linked UUID aliases', () => {
    expect(recentRevisionIds([], [draft], server)).toEqual([]);
    expect(recentRevisionIds(recent, [{ ...draft, mount: null }], server)).toEqual([]);
    expect(recentRevisionIds(recent, [{ ...draft, server: 'https://other.example.test' }], server)).toEqual([]);
    expect(recentRevisionIds(recent, [draft, { ...draft, server: server + '/', linked_uuid: 'other-project' }], server)).toEqual([draft.uuid, 'other-project']);
  });
  it('never requests without a visible screen, token, account, server or linked recent UUID', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch');
    await recentRevisions(server, '', 'a', [draft.uuid], undefined, visible);
    await recentRevisions('', 'token', 'a', [draft.uuid], undefined, visible);
    await recentRevisions(server, 'token', '', [draft.uuid], undefined, visible);
    await recentRevisions(server, 'token', 'a', [], undefined, visible);
    await recentRevisions(server, 'token', 'a', [draft.uuid], undefined, () => false);
    expect(fetch).not.toHaveBeenCalled();
  });
  it('reuses fresh answers, persists across documents, then sends the last ETag and renews freshness on 304', async () => {
    let now = 1_000_000;
    vi.spyOn(Date, 'now').mockImplementation(() => now);
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_input, init) => {
      const tag = new Headers(init?.headers).get('If-None-Match');
      return tag ? new Response(null, { status: 304 }) : Response.json([row], { headers: { ETag: 'revision-tag' } });
    });
    const before = performance.now();
    for (let i = 0; i < 5; i++) await batchRevisions(server, 'token', [draft.uuid]);
    const beforeMs = performance.now() - before;
    expect(fetch).toHaveBeenCalledTimes(5); fetch.mockClear();
    const start = performance.now();
    const first = await recentRevisions(server, 'token', 'a', [draft.uuid], undefined, visible);
    now += 59_999;
    expect(await recentRevisions(server + '/', 'token', 'a', [draft.uuid], undefined, visible)).toEqual(first);
    expect(fetch).toHaveBeenCalledTimes(1);
    // The next document reads its account/server mirror with no in-memory entry.
    hideMountCache();
    const stored = JSON.parse(JSON.stringify(first));
    expect(await recentRevisions(server, 'token', 'a', [draft.uuid], stored, visible)).toEqual(first);
    expect(fetch).toHaveBeenCalledTimes(1);
    now++;
    const revalidated = await recentRevisions(server, 'token', 'a', [draft.uuid], stored, visible);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(new Headers(fetch.mock.calls[1][1]?.headers).get('If-None-Match')).toBe('revision-tag');
    expect(revalidated.batches[0].rows).toEqual([row]);
    expect(revalidated.batches[0].at).toBe(now);
    now += 59_999;
    await recentRevisions(server, 'token', 'a', [draft.uuid], revalidated, visible);
    expect(fetch).toHaveBeenCalledTimes(2);
    console.info(JSON.stringify({ revisionVisits: 5, before: { requests: 5, ms: beforeMs }, after: { requests: fetch.mock.calls.length, ms: performance.now() - start } }));
  });
  it('coalesces concurrent consumers even when the first screen instance is destroyed', async () => {
    let active = true, release!: () => void;
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => {
      await new Promise<void>(resolve => { release = resolve; });
      return Response.json([row], { headers: { ETag: 'tag' } });
    });
    const first = recentRevisions(server, 'token', 'a', [draft.uuid], undefined, () => active).catch(e => e);
    active = false;
    const second = recentRevisions(server, 'token', 'a', [draft.uuid], undefined, visible);
    await vi.waitFor(() => expect(release).toBeTypeOf('function'));
    expect(fetch).toHaveBeenCalledTimes(1);
    release();
    expect(await first).toBeInstanceOf(Error);
    expect((await second).batches[0].rows).toEqual([row]);
  });
  it('scopes ETags and freshness to account, normalized server, view-as and exact batch body', async () => {
    const { oversightSession } = await import('./oversight');
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => Response.json([row], { headers: { ETag: 'tag' } }));
    const first = await recentRevisions(server, 'token', 'a', [draft.uuid], undefined, visible);
    await recentRevisions(server, 'token', 'b', [draft.uuid], first, visible);
    await recentRevisions('https://other.example.test', 'token', 'a', [draft.uuid], first, visible);
    await recentRevisions(server, 'token', 'a', ['other-project'], first, visible);
    oversightSession.set({ viewAs: { username: 'robin', name: 'Robin Example' }, onBehalf: false });
    try {
      await recentRevisions(server, 'token', 'a', [draft.uuid], first, visible);
      expect(fetch).toHaveBeenCalledTimes(5);
      expect(fetch.mock.calls.every(([, init]) => !new Headers(init?.headers).has('If-None-Match'))).toBe(true);
      expect(String(fetch.mock.calls[4][0])).toContain('as=robin');
    } finally { oversightSession.set(null); }
  });
  it('retains separate ETags per batch when thousands of UUIDs revalidate', async () => {
    let now = 1_000_000;
    vi.spyOn(Date, 'now').mockImplementation(() => now);
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_input, init) => {
      const ids = JSON.parse(String(init?.body)).projects;
      const tag = `batch-${ids[0]}`;
      const previous = new Headers(init?.headers).get('If-None-Match');
      expect(ids.length).toBeLessThanOrEqual(500);
      if (previous) { expect(previous).toBe(tag); return new Response(null, { status: 304 }); }
      return Response.json([{ ...row, project_uuid: ids[0] }], { headers: { ETag: tag } });
    });
    const ids = Array.from({ length: 1001 }, (_, i) => String(i).padStart(4, '0'));
    const first = await recentRevisions(server, 'token', 'a', ids, undefined, visible);
    expect(fetch).toHaveBeenCalledTimes(3);
    now += 60_000;
    const result = await recentRevisions(server, 'token', 'a', ids, first, visible);
    expect(fetch).toHaveBeenCalledTimes(6);
    expect(result.batches.map(b => b.rows)).toEqual(first.batches.map(b => b.rows));
  });
  it('does not send later batches after leaving the screen or signing out during a stalled lookup', async () => {
    for (const signOut of [false, true]) {
      hideMountCache();
      let active = true, release!: () => void;
      const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => {
        await new Promise<void>(resolve => { release = resolve; }); return Response.json([row]);
      });
      const result = recentRevisions(server, 'token', 'a', Array.from({ length: 1001 }, (_, i) => String(i)), undefined, () => active);
      const rejected = expect(result).rejects.toThrow(signOut ? 'account changed' : 'session changed');
      await vi.waitFor(() => expect(release).toBeTypeOf('function'));
      if (signOut) hideMountCache(); else active = false;
      release(); await rejected;
      expect(fetch).toHaveBeenCalledTimes(1); fetch.mockRestore();
    }
  });
  it('surfaces status and reply excerpts and permits retry after a failed lookup', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(Response.json({ detail: 'Invented failure' }, { status: 503 }))
      .mockImplementation(async () => Response.json([row]));
    await expect(recentRevisions(server, 'token', 'a', [draft.uuid], undefined, visible)).rejects.toThrow('Database 503: {"detail":"Invented failure"}');
    expect((await recentRevisions(server, 'token', 'a', [draft.uuid], undefined, visible)).batches[0].rows).toEqual([row]);
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});
