import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TimsApi, localSuggestion, nextFreeCapsule, orderedBatches, uniqueGrains, sampleConflict, type BatchDetail, type CapsuleOut, type FractionInput, type FractionOut, type TimsPages, type TimsTransport } from './tims';
import { TimsOutbox } from './tims-outbox';
import { SyncApiError } from './sync-api';
import { elementsOf, storedElements, KINDS } from './sync';
import { elementHash } from './sync-hash';
import type { Project } from './core/types';

const sample = { id: 'sample-1', name: 'SAMPLE1' };
const set: CapsuleOut = { id: 'set-1', lab_id: 'lab-1', name: 'Krogh', code: 'K', positions: 15, capsule_labels: Array.from({ length: 15 }, (_, i) => `K${i + 1}`) };
const batch: BatchDetail = { id: 'batch-1', capsule_set_id: set.id, started_on: '2026-01-02', operator_name: 'Invented operator', notes: '', capsule_map: { '1': 'elsewhere' } };
const input = (grain = 'grain-1', name = 'z1'): FractionInput => ({ name, mineral: 'zircon', batch_id: null, capsule_number: null, notes: '', sources: [{ atlas_grain_id: grain }] });
const fraction = (id = 'fraction-1', grain = 'grain-1', name = 'z1'): FractionOut => ({ ...input(grain, name), id, sample, status: 'picked', sources: [{ id: 'source-1', atlas_grain_id: grain }] });
function memory() { const map = new Map<string, string>(); return { map, getItem: (key: string) => map.get(key) ?? null, setItem: (key: string, value: string) => { map.set(key, value); } }; }
const instances: TimsOutbox[] = [];
function setup(storage = memory(), key = 'account/server/mount') {
  const writes: { path: string; body: any; key: string | undefined; method: string }[] = [];
  let next = 0;
  const api: TimsTransport = {
    list: vi.fn(async () => []) as TimsTransport['list'], detail: vi.fn() as TimsTransport['detail'], suggest: vi.fn(async () => ({ suggestion: 'z1', sample })),
    write: vi.fn(async (path, method, body: any, identity) => {
      writes.push({ path, method, body: structuredClone(body), key: identity.key });
      if (path === '/v1/tims/batches') return { ...batch, ...body, id: `real-batch-${++next}`, capsule_map: {} };
      if (path.endsWith('/sources')) return { ...body, id: `real-source-${++next}` };
      return { ...fraction(), ...body, id: path === '/v1/fractions' ? `real-fraction-${++next}` : decodeURIComponent(path.split('/')[3]), sources: body.sources?.map((s: object, i: number) => ({ ...s, id: `real-source-${i}` })) ?? fraction().sources };
    }) as TimsTransport['write'],
  };
  const store = new TimsOutbox(key, 'mount-1', storage, api); instances.push(store);
  return { store, api, writes, storage };
}
function refusal(status: number, code: string, detail: string) { const e = new SyncApiError(status, [detail]); e.code = code; return e; }
beforeEach(() => vi.useFakeTimers());
afterEach(() => { for (const s of instances.splice(0)) s.dispose(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('durable ordered TIMS outbox', () => {
  it('manual mode persists and retries only on explicit flush, and opt-in restores scheduled sends', async () => {
    const { store, api, writes, storage } = setup();
    store.setAutomatic(false);
    await store.createFraction(input(), sample);
    expect(JSON.parse(storage.map.get(store.key)!).outbox).toHaveLength(1);
    store.wake();
    await vi.advanceTimersByTimeAsync(300_000);
    expect(writes).toHaveLength(0);
    expect(vi.getTimerCount()).toBe(0);
    vi.mocked(api.write).mockRejectedValueOnce(new Error('Synthetic offline'));
    await store.flush();
    expect(store.data.outbox).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
    store.setAutomatic(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(writes).toHaveLength(1);
    expect(store.data.outbox).toHaveLength(0);
  });

  it('writes batches before fractions, rewrites temporary references and remembers real ids', async () => {
    const { store, writes } = setup();
    const b = await store.createBatch({ capsule_set_id: set.id, started_on: batch.started_on });
    const f = await store.createFraction({ ...input(), batch_id: b, capsule_number: 1 }, sample);
    const originalKeys = store.data.outbox.map(e => e.key);
    await store.remember(set.id, b);
    await store.flush();
    expect(writes.map(w => w.path)).toEqual(['/v1/tims/batches', '/v1/fractions']);
    expect(writes[1].body.batch_id).toBe('real-batch-1');
    expect(writes.map(w => w.key)).toEqual(originalKeys);
    expect(store.resolve(f)).toBe('real-fraction-2');
    expect(store.data.lastBatch[set.id]).toBe('real-batch-1');
    expect(store.data.outbox).toHaveLength(0);
  });
  it('sends an independent grain after a waiting pluck, but preserves source order', async () => {
    const { store, api, writes } = setup();
    const first = await store.createFraction(input('grain-A'), sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'grain_not_registered', 'Not registered'));
    await store.flush();
    await store.createFraction(input('grain-B', 'z2'), sample);
    await store.createFraction(input('grain-A', 'z3'), sample);
    await store.flush();
    expect(writes.map(w => w.body.name)).toEqual(['z2']);
    expect(store.data.outbox.map(e => e.target)).toContain(first);
    expect(store.data.outbox).toHaveLength(2);
  });
  it('blocks a source addition after its failed fraction creation', async () => {
    const { store, api } = setup();
    const id = await store.createFraction(input(), sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'validation_failed', 'Refused'));
    await store.flush();
    await store.addSource(id, { atlas_grain_id: 'grain-B' }); await store.flush();
    expect(api.write).toHaveBeenCalledTimes(1); expect(store.data.outbox[1].op).toBe('source.add');
  });
  it.each(['waiting', 'failed'] as const)('blocks fractions using a %s batch', async state => {
    const { store, writes } = setup();
    const b = await store.createBatch({ capsule_set_id: set.id, started_on: batch.started_on });
    store.data.outbox[0].state = state;
    await store.createFraction({ ...input(), batch_id: b, capsule_number: 1 }, sample);
    await store.createFraction(input('grain-B', 'z2'), sample); await store.flush();
    expect(writes.map(w => w.body.name)).toEqual(['z2']); expect(store.data.outbox).toHaveLength(2);
  });
  it('keeps later writes to the same existing batch behind a refusal', async () => {
    const { store, api, writes } = setup(); store.data.batches = [batch];
    await store.createFraction({ ...input(), batch_id: batch.id, capsule_number: 2 }, sample);
    await store.createFraction({ ...input('grain-B', 'z2'), batch_id: batch.id, capsule_number: 3 }, sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'validation_failed', 'Refused'));
    await store.flush(); expect(api.write).toHaveBeenCalledTimes(1); expect(writes).toHaveLength(0);
  });
  it('coalesces unsent fraction edits and source changes, and drops an unsent retirement', async () => {
    const { store, writes } = setup();
    const id = await store.createFraction(input(), sample);
    await store.patchFraction(id, { name: 'z8', notes: 'Edited locally' });
    await store.addSource(id, { atlas_grain_id: 'grain-2' });
    const addedSource = store.fractions[0].sources[1];
    await store.retireSource(id, addedSource.id);
    expect(store.data.outbox).toHaveLength(1);
    expect(store.fractions[0].name).toBe('z8');
    expect(store.fractions[0].sources).toHaveLength(1);
    await store.retireFraction(id);
    expect(store.data.outbox).toHaveLength(0); expect(store.fractions).toHaveLength(0); expect(writes).toHaveLength(0);
  });
  it('drops dependent unsent entries with an unsent batch', async () => {
    const { store } = setup();
    const b = await store.createBatch({ capsule_set_id: set.id, started_on: batch.started_on });
    await store.createFraction({ ...input(), batch_id: b, capsule_number: 1 }, sample);
    await store.discard(store.data.outbox[0].id);
    expect(store.data.outbox).toHaveLength(0);
  });
  it('coalesces unsent patches and drops unsent source additions on existing fractions', async () => {
    const { store, writes } = setup(); store.data.fractions = [fraction()];
    await store.patchFraction('fraction-1', { name: 'z2' }); await store.patchFraction('fraction-1', { notes: 'Note' });
    await store.addSource('fraction-1', { atlas_grain_id: 'grain-2' });
    await store.retireSource('fraction-1', store.fractions[0].sources[1].id);
    expect(store.data.outbox).toHaveLength(1);
    await store.flush();
    expect(writes[0].method).toBe('PATCH'); expect(writes[0].body).toEqual({ name: 'z2', notes: 'Note' });
  });
  it('keeps the key and exact body across network retries and a restart', async () => {
    const { store, api, storage } = setup();
    await store.createFraction(input(), sample);
    const original = structuredClone(store.data.outbox[0]);
    vi.mocked(api.write).mockRejectedValueOnce(new TypeError('Connection lost'));
    await store.flush(); expect(store.data.outbox[0].key).toBe(original.key); expect(store.offline).toBe(true);
    store.dispose();
    const restarted = setup(storage); await restarted.store.load();
    await restarted.store.flush();
    expect(restarted.writes[0].key).toBe(original.key); expect(restarted.writes[0].body).toEqual(original.body);
  });
  it('never coalesces an attempted write whose reply may be lost', async () => {
    const { store, api } = setup(); await store.createFraction(input(), sample);
    vi.mocked(api.write).mockRejectedValue(new TypeError('Reply lost')); await store.flush();
    const original = structuredClone(store.data.outbox[0]);
    await store.patchFraction(original.target, { name: 'z2' });
    expect(store.data.outbox).toHaveLength(2); expect(store.data.outbox[0].body).toEqual(original.body);
    vi.mocked(api.write).mockImplementation(async (_path, _method, body) => ({ ...fraction(), ...(body as object) } as never));
    await store.flush();
    expect(vi.mocked(api.write).mock.calls.at(-1)?.[0]).toBe('/v1/fractions/fraction-1');
  });
  it.each(['grain_not_registered', 'spot_not_registered'])('waits for a successful mount sync after %s, keeping its key', async code => {
    const { store, api } = setup(); await store.createFraction(input(), sample); const key = store.data.outbox[0].key;
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, code, 'Not registered'));
    await store.flush(); expect(store.data.outbox[0].state).toBe('waiting'); expect(store.badges.get('grain-1')?.state).toBe('waiting');
    store.wake(); await store.flush(); expect(api.write).toHaveBeenCalledTimes(1);
    store.mountSynced(); await store.flush(); expect(api.write).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.write).mock.calls[1][3].key).toBe(key);
  });
  it('rolls back a failed edit and allows corrected editing with a new key', async () => {
    const { store, api } = setup(); store.data.fractions = [fraction()];
    await store.patchFraction('fraction-1', { name: 'z9' }); expect(store.fractions[0].name).toBe('z9');
    const key = store.data.outbox[0].key;
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'validation_failed', 'That fraction name is already used'));
    await store.flush(); expect(store.fractions[0].name).toBe('z1');
    expect(store.data.outbox[0].error).toBe('That fraction name is already used'); expect(store.badges.get('grain-1')?.state).toBe('failed');
    await store.editFailed(store.data.outbox[0].id, { name: 'z10' }); expect(store.data.outbox[0].key).not.toBe(key);
    await store.flush(); expect(store.data.outbox).toHaveLength(0);
  });
  it('rolls back refused creation while showing the failed badge and permission detail', async () => {
    const { store, api } = setup(); await store.createFraction(input(), sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(403, 'forbidden', 'Forbidden'));
    await store.flush(); expect(store.fractions).toHaveLength(0); expect(store.badges.get('grain-1')?.state).toBe('failed');
    expect(store.data.outbox[0].error).toBe('Only lab members can pluck grains for this sample');
    await store.discard(store.data.outbox[0].id); expect(store.badges.size).toBe(0);
  });
  it('blocks following writes to the same batch on a refusal and resumes after discard', async () => {
    const { store, api, writes } = setup();
    await store.createFraction({ ...input(), batch_id: batch.id, capsule_number: 2 }, sample);
    await store.createFraction({ ...input('grain-2', 'z2'), batch_id: batch.id, capsule_number: 3 }, sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(409, 'conflict', 'Capsule occupied'));
    await store.flush(); expect(api.write).toHaveBeenCalledTimes(1);
    await store.discard(store.data.outbox[0].id); await store.flush(); expect(writes).toHaveLength(1); expect(writes[0].body.name).toBe('z2');
  });
  it('persists replacement keys from idempotentWrite before their use', async () => {
    const { store, api, storage } = setup(); await store.createFraction(input(), sample);
    vi.mocked(api.write).mockImplementationOnce(async (_path, _method, _body, identity) => {
      await identity.keyChanged?.('replacement-key');
      expect(JSON.parse(storage.map.get(store.key)!).outbox[0].key).toBe('replacement-key');
      throw new TypeError('Lost reply');
    });
    await store.flush(); expect(store.data.outbox[0].key).toBe('replacement-key');
  });
  it('keeps stable identities when several unsent sources are removed', async () => {
    const { store } = setup();
    const id = await store.createFraction({ ...input(), sources: ['g1', 'g2', 'g3'].map(atlas_grain_id => ({ atlas_grain_id })) }, sample);
    const [first, , third] = store.fractions[0].sources;
    await store.retireSource(id, first.id); await store.retireSource(id, third.id);
    expect(store.fractions[0].sources.map(s => s.atlas_grain_id)).toEqual(['g2']);
  });
  it('rewrites sources and retirements queued after an attempted fraction creation', async () => {
    const { store, api, writes } = setup();
    const id = await store.createFraction(input(), sample), source = store.fractions[0].sources[0].id;
    vi.mocked(api.write).mockRejectedValueOnce(new TypeError('Lost reply')); await store.flush();
    await store.retireSource(id, source);
    await store.addSource(id, { atlas_grain_id: 'grain-2' });
    await store.patchFraction(id, { notes: 'Later edit' });
    await store.flush();
    expect(writes.map(w => w.path)).toEqual(['/v1/fractions', '/v1/fractions/real-fraction-1/sources/real-source-0/retire', '/v1/fractions/real-fraction-1/sources', '/v1/fractions/real-fraction-1']);
    expect(store.resolve(source)).toBe('real-source-0'); expect(store.resolve(id)).toBe('real-fraction-1');
  });
  it('rolls back a refused batch and its dependent fraction, shows the failure on its grain, and retries in order', async () => {
    const { store, api } = setup();
    const b = await store.createBatch({ capsule_set_id: set.id, started_on: batch.started_on });
    await store.createFraction({ ...input(), batch_id: b, capsule_number: 1 }, sample);
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'validation_failed', 'Invalid batch date'));
    await store.flush(); expect(store.fractions).toHaveLength(0); expect(store.batches).toHaveLength(0);
    expect(store.badges.get('grain-1')?.state).toBe('failed');
    await store.editFailed(store.data.outbox[0].id, { started_on: '2026-01-03' });
    await store.flush(); expect(store.data.outbox).toHaveLength(0);
  });
  it('rolls back a refused source addition, retaining its failed badge for editing', async () => {
    const { store, api } = setup(); store.data.fractions = [fraction()];
    await store.addSource('fraction-1', { atlas_grain_id: 'grain-2', is_fragment: true, fragment_note: 'Tip' });
    vi.mocked(api.write).mockRejectedValueOnce(refusal(422, 'validation_failed', 'Wrong sample'));
    await store.flush(); expect(store.fractions[0].sources).toHaveLength(1); expect(store.badges.get('grain-2')?.state).toBe('failed');
  });
  it('queues new batches immediately and resolves the unique recipe before the first send', async () => {
    const { store, api, writes } = setup(); store.data.sets = [set];
    vi.mocked(api.list).mockResolvedValueOnce([{ id: 'recipe-1' }]);
    const b = await store.newBatch({ capsule_set_id: set.id, started_on: batch.started_on }, 'zircon');
    expect(api.list).not.toHaveBeenCalled(); expect(store.batches[0].id).toBe(b);
    await store.flush(); expect(writes[0].body.recipe_id).toBe('recipe-1');
    expect(vi.mocked(api.list).mock.calls[0][0]).toContain('lab=lab-1&mineral=zircon');
  });
  it('loads a selected batch and its fraction names in two bounded requests, never by row', async () => {
    const { store, api } = setup();
    vi.mocked(api.detail).mockResolvedValueOnce({ value: batch, etag: 'batch-etag' });
    vi.mocked(api.list).mockResolvedValueOnce([{ ...fraction('elsewhere', 'other-grain', 'z55'), batch_id: batch.id, capsule_number: 1 }]);
    await store.loadBatch(batch.id);
    expect(api.detail).toHaveBeenCalledTimes(1); expect(api.list).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.list).mock.calls[0][0]).toBe('/v1/fractions?batch=batch-1&limit=500');
    expect(store.fractionName('elsewhere')).toBe('z55'); expect(store.data.fractions).toHaveLength(0);
  });
  it('uses the batch detail fraction summary without another list or per-capsule requests', async () => {
    const { store, api } = setup();
    vi.mocked(api.detail).mockResolvedValueOnce({ value: { ...batch, capsule_set: set, fractions: [fraction('elsewhere', 'other-grain', 'z55')] }, etag: 'batch-etag' });
    await store.loadBatch(batch.id);
    expect(api.detail).toHaveBeenCalledTimes(1); expect(api.list).not.toHaveBeenCalled();
    expect(store.fractionName('elsewhere')).toBe('z55'); expect(store.data.sets).toEqual([set]);
  });
  it('keeps hidden/sign-out mirrors and never sends while detached', async () => {
    const storage = memory(), first = setup(storage, 'account-a/server/mount');
    await first.store.createFraction(input(), sample); first.store.dispose(); await first.store.flush();
    expect(first.writes).toHaveLength(0);
    const second = setup(storage, 'account-b/server/mount'); await second.store.load(); expect(second.store.fractions).toHaveLength(0);
    const restored = setup(storage, first.store.key); await restored.store.load(); expect(restored.store.data.outbox).toHaveLength(1);
  });
  it('does not send when durable storage fails', async () => {
    const { store, writes, storage } = setup(); await store.createFraction(input(), sample);
    storage.setItem = () => { throw new Error('Disk full'); };
    await expect(store.flush()).rejects.toThrow('Disk full'); expect(writes).toHaveLength(0);
  });
});

describe('suggestions, occupancy and mount isolation', () => {
  it('uses the sample, retired names, suffixes, mineral and pending fractions', async () => {
    const { store, api } = setup();
    store.data.fractions = [fraction('f1', 'g1', 'z3a'), { ...fraction('f2', 'g2', 'z8'), retired_at: '2026-01-01' }, { ...fraction('f3', 'g3', 'z99'), sample: { id: 'other', name: 'Other sample' } }];
    expect(localSuggestion(store.fractions, sample, 'zircon')).toBe('z9');
    for (const [mineral, letter] of Object.entries({ baddeleyite: 'b', titanite: 't', rutile: 'r', apatite: 'a', monazite: 'm', xenotime: 'x', other: 'o' })) expect(localSuggestion([], sample, mineral as never)).toBe(`${letter}1`);
    await store.createFraction(input('g4', 'z9'), sample);
    vi.mocked(api.suggest).mockRejectedValueOnce(new TypeError('Offline'));
    expect(await store.suggestion('g4', sample, 'zircon')).toEqual({ name: 'z10', sample, checked: false });
    expect((await store.suggestion('g4', sample, 'zircon')).name).toBe('z10');
  });
  it('reserves cached and pending capsules and frees moved or retired positions', async () => {
    const { store } = setup(); store.data.sets = [set]; store.data.batches = [batch];
    await store.createFraction({ ...input(), batch_id: batch.id, capsule_number: 2 }, sample);
    expect(store.nextCapsule(batch.id)).toBe(3);
    expect(nextFreeCapsule(2, { '1': 'a', '2': 'b' })).toBeNull();
    store.data.fractions = [{ ...fraction('f1', 'g2', 'z2'), batch_id: batch.id, capsule_number: 3 }]; store.data.batches[0].capsule_map['3'] = 'f1';
    await store.patchFraction('f1', { capsule_number: 4 }); expect(store.nextCapsule(batch.id)).toBe(3);
    expect(store.occupancy(batch.id, 'f1')['4']).toBeUndefined();
  });
  it('shows capsule and fragment badges without per-grain requests', async () => {
    const { store, api } = setup(); store.data.sets = [set]; store.data.batches = [batch];
    await store.createFraction({ ...input(), batch_id: batch.id, capsule_number: 3, sources: [{ atlas_grain_id: 'grain-1', is_fragment: true, fragment_note: 'Tip' }] }, sample);
    expect(store.badges.get('grain-1')?.text).toBe('K3 · z1†'); expect(store.badges.get('grain-1')?.state).toBe('pending');
    expect(api.list).not.toHaveBeenCalled(); expect(api.detail).not.toHaveBeenCalled();
  });
  it('leaves project data, registered names, sync elements and every sync hash identical after plucking', async () => {
    const project = { uuid: 'project-1', areas: [{ id: 'a', sample: sample.name }], upbdb: { mount_id: 'mount-1' }, grainatlas_ui: { grains: { a: [{ id: 'grain-1', number: 1, label: 'a/1', centroid_um: [1, 2], polygon_um: [[0, 0], [2, 0], [1, 3]], area_um2: 3 }] }, spots: [{ id: 'spot-1', name: 'Invented_1', area_id: 'a', grain_id: 'grain-1', kind: 'spot', x_um: 1, y_um: 2, diameter_um: 1 }], labels: [] } } as unknown as Project;
    const original = JSON.stringify(project), before = storedElements(elementsOf(project));
    const hashes = await Promise.all(KINDS.map(k => elementHash(before[k])));
    const { store } = setup(); await store.createFraction({ ...input(), sources: [{ atlas_grain_id: 'grain-1', atlas_spot_id: 'spot-1' }] }, sample); await store.flush();
    expect(JSON.stringify(project)).toBe(original); const after = storedElements(elementsOf(project)); expect(after).toEqual(before);
    expect(await Promise.all(KINDS.map(k => elementHash(after[k])))).toEqual(hashes);
  });
});

describe('TIMS requests', () => {
  it('persists ETags per page and reuses cached bodies for 304, including after restart', async () => {
    vi.useRealTimers(); let count = 0;
    const fetcher = vi.fn(async (_url, init: RequestInit) => {
      count++; if (count === 1) return Response.json([fraction()], { headers: { ETag: '"fractions-v1"' } });
      expect(new Headers(init.headers).get('If-None-Match')).toBe('"fractions-v1"'); return new Response(null, { status: 304 });
    }); vi.stubGlobal('fetch', fetcher);
    const api = new TimsApi('https://db.example.test', 'token'), pages: TimsPages = {};
    const path = '/v1/fractions?mount=mount-1&limit=500';
    const first = await api.list(path, pages); const restarted = JSON.parse(JSON.stringify(pages));
    expect(await new TimsApi('https://db.example.test', 'token').list(path, restarted)).toEqual(first); expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it('loads 3000 fractions progressively in six bounded pages with no row requests, then revalidates cheaply', async () => {
    vi.useRealTimers(); const rows = Array.from({ length: 3000 }, (_, i) => fraction(`f-${i}`, `g-${i}`, `z${i + 1}`));
    const urls: URL[] = []; let revalidate = false;
    vi.stubGlobal('fetch', vi.fn(async (url: string, init: RequestInit) => {
      const u = new URL(url); urls.push(u); expect(u.pathname).toBe('/v1/fractions'); expect(u.searchParams.get('mount')).toBe('mount-1'); expect(u.searchParams.get('limit')).toBe('500');
      const start = Number(u.searchParams.get('after') ?? 0), next = start + 500;
      if (revalidate) { expect(new Headers(init.headers).get('If-None-Match')).toBe(`"page-${start}"`); return new Response(null, { status: 304 }); }
      return Response.json(rows.slice(start, next), { headers: { ETag: `"page-${start}"`, ...(next < rows.length ? { 'X-Next-Cursor': String(next) } : {}) } });
    }));
    const api = new TimsApi('https://db.example.test', 'token'), pages: TimsPages = {}, progress: number[] = [];
    const started = performance.now();
    expect(await api.list('/v1/fractions?mount=mount-1&limit=500', pages, r => progress.push(r.length))).toHaveLength(3000);
    const coldMs = performance.now() - started; expect(urls).toHaveLength(6); expect(progress).toEqual([500, 1000, 1500, 2000, 2500, 3000]);
    revalidate = true; const warm = performance.now();
    expect(await api.list('/v1/fractions?mount=mount-1&limit=500', pages)).toHaveLength(3000); expect(urls).toHaveLength(12);
    console.info(JSON.stringify({ timsFractions: 3000, before: { requests: 0, ms: 0 }, after: { coldRequests: 6, coldMs: Math.round(coldMs), revalidateRequests: 6, revalidateMs: Math.round(performance.now() - warm) }, perRowRequests: 0 }));
  });
  it('shares authentication and preserves a persisted write key for PATCH as well as POST', async () => {
    vi.useRealTimers(); const fetcher = vi.fn(async (_url, init: RequestInit) => {
      expect(new Headers(init.headers).get('Authorization')).toBe('Bearer token'); expect(new Headers(init.headers).get('Idempotency-Key')).toBe('persisted-key'); return Response.json(fraction());
    }); vi.stubGlobal('fetch', fetcher);
    await new TimsApi('https://db.example.test', 'token').write('/v1/fractions/f1', 'PATCH', { name: 'z1' }, { key: 'persisted-key' }); expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('keeps server 422 detail verbatim and the status/code', async () => {
    vi.useRealTimers(); vi.stubGlobal('fetch', vi.fn(async () => Response.json({ detail: 'Capsule K3 is already taken', code: 'validation_failed' }, { status: 422 })));
    await expect(new TimsApi('https://db.example.test', 'token').write('/v1/fractions', 'POST', input(), { key: 'key' })).rejects.toMatchObject({ status: 422, code: 'validation_failed', message: 'Capsule K3 is already taken' });
  });
});

describe('shared pluck dialog rules', () => {
  it('orders active batches newest first', () => {
    expect(orderedBatches([batch, { ...batch, id: 'newer', started_on: '2026-02-01' }, { ...batch, id: 'retired', retired_at: 'today' }], set.id).map(b => b.id)).toEqual(['newer', batch.id]);
  });
  it('deduplicates two spots on one grain, keeping the first', () => {
    expect(uniqueGrains([{ grain: 'A', spot: 'first' }, { grain: 'A', spot: 'second' }])).toEqual([{ grain: 'A', spot: 'first' }]);
  });
  it('names conflicting samples and allows one sample', () => {
    expect(sampleConflict([{ sample }, { sample: { id: 'sample-2', name: 'SAMPLE2' } }])).toContain('SAMPLE1, SAMPLE2');
    expect(sampleConflict([{ sample }, { sample }])).toBe('');
  });
  it('formats fragment badges with a dagger', () => {
    const { store } = setup(); store.data.sets = [set]; store.data.batches = [batch];
    store.data.fractions = [{ ...fraction(), batch_id: batch.id, capsule_number: 3, sources: [{ id: 'source', atlas_grain_id: 'A', is_fragment: true, fragment_note: 'Tip' }] }];
    expect(store.badges.get('A')?.text).toBe('K3 · z1†');
  });
});
