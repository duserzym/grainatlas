import assert from 'node:assert/strict';
import test from 'node:test';
import { BackgroundSync } from './background-sync.ts';
import { idempotentWrite } from './idempotency.ts';
import { SyncApi, SyncApiError, requestIdFor } from './sync-api.ts';
import { plainSyncStatus, draftSyncStatus } from './sync-status.ts';
import { clearSyncCache } from './sync-cache.ts';
import { publishAtlas } from './upbdb.ts';

const body = { project_uuid: 'synthetic', mode: 'delta', base_revision: 1,
  spots: { upsert: [], delete: [] }, grains: { upsert: [], delete: [], retire: [] }, labels: { upsert: [], delete: [] } };
const draft = { uuid: 'synthetic', name: 'Synthetic', mount: 'mount', base_revision: 1, base: { spots: [], grains: [], labels: [] },
  changes: { spots: 0, grains: 0, labels: 0 }, pending: null, pixels_base: null, checked_at: new Date().toISOString(), server_revision: 1 };

test('automatic timer coalesces edits, cancels and wakes immediately', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let runs = 0;
  const timer = new BackgroundSync(() => runs++);
  timer.edit(); t.mock.timers.tick(2999); timer.edit();
  t.mock.timers.tick(2999); assert.equal(runs, 0);
  t.mock.timers.tick(1); assert.equal(runs, 1);
  timer.ensure(); timer.ensure(); t.mock.timers.tick(3000); assert.equal(runs, 2);
  timer.edit(); timer.cancel(); t.mock.timers.tick(3000); assert.equal(runs, 2);
  timer.wake(); t.mock.timers.tick(0); assert.equal(runs, 3);
});
test('backoff doubles from 5 seconds to 5 minutes, resets on success and bounds jitter', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let runs = 0;
  const timer = new BackgroundSync(() => runs++, () => .5);
  for (const delay of [5000, 10000, 20000, 40000, 80000, 160000, 300000, 300000]) {
    const before = runs; timer.retry(); timer.retry();
    t.mock.timers.tick(delay - 1); assert.equal(runs, before);
    t.mock.timers.tick(1); assert.equal(runs, before + 1);
  }
  timer.reset(); timer.retry(); t.mock.timers.tick(5000); assert.equal(runs, 9);
  for (const [random, delay] of [[0, 4000], [1, 6000]]) {
    const jitter = new BackgroundSync(() => runs++, () => random);
    jitter.retry(); const before = runs;
    t.mock.timers.tick(delay - 1); assert.equal(runs, before);
    t.mock.timers.tick(1); assert.equal(runs, before + 1);
  }
});
test('one status table covers sessions and Open, with attention and freshness precedence', () => {
  for (const [patch, flags, expected] of [
    [{}, {}, 'Up to date'], [{}, { busy: true }, 'Syncing…'],
    [{ changes: { spots: 1, grains: 0, labels: 0 } }, {}, 'Saved'],
    [{}, { reachable: false }, 'Offline, will sync later'],
    [{}, { error: true }, 'Needs your attention'], [{}, { attention: true, busy: true }, 'Needs your attention'],
    [{ held_back: [{}] }, {}, 'Needs your attention'], [{ link_conflict: true }, {}, 'Needs your attention'],
    [{ checked_at: undefined }, {}, 'Saved'], [{ checked_at: '2000-01-01T00:00:00Z' }, {}, 'Saved'],
    [{ server_revision: 2 }, {}, 'Saved'], [{ pending_request: { id: 'pending' } }, {}, 'Saved'],
  ]) {
    const d = { ...draft, ...patch };
    assert.equal(plainSyncStatus(d, draftSyncStatus(d), flags), expected);
  }
});
test('409 waits Retry-After (default 2 s), repeats exact key and body, replay is success', async t => {
  const keys = [], waits = [];
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    keys.push([init.headers['Idempotency-Key'], init.body]);
    if (keys.length < 3) return Response.json({ code: 'idempotency_in_progress' }, { status: 409, headers: keys.length === 1 ? { 'Retry-After': '3' } : {} });
    return Response.json({ ok: true }, { headers: { 'Idempotent-Replayed': 'true' } });
  });
  const reply = await idempotentWrite(key => fetch('https://sync.test/write', { headers: { 'Idempotency-Key': key }, body: 'same' }), { key: 'stable', wait: async ms => { waits.push(ms); } });
  assert.equal(reply.ok, true); assert.deepEqual(waits, [3000, 2000]);
  assert.deepEqual(keys, Array(3).fill(['stable', 'same']));
});
test('422 key-reused renews once, logs and persists before retry; other 409 is untouched', async t => {
  const keys = [], saved = [];
  const warn = t.mock.method(console, 'warn', () => {});
  const response = await idempotentWrite(async key => {
    keys.push(key);
    if (keys.length === 2) assert.deepEqual(saved, [key]);
    return Response.json({ code: 'idempotency_key_reused' }, { status: 422 });
  }, { key: 'old', keyChanged: async key => { saved.push(key); } });
  assert.equal(response.status, 422); assert.equal(keys.length, 2); assert.notEqual(keys[0], keys[1]); assert.equal(warn.mock.callCount(), 1);
  let calls = 0;
  await idempotentWrite(async () => { calls++; return Response.json({ code: 'replace_confirm' }, { status: 409 }); });
  assert.equal(calls, 1);
});
test('in-progress writes stop after 30 waits and return the last reply with the same key', async () => {
  const keys = [], waits = [];
  let last;
  const response = await idempotentWrite(async key => {
    keys.push(key);
    return last = Response.json({ code: 'idempotency_in_progress' }, { status: 409 });
  }, { key: 'still-running', wait: async ms => { waits.push(ms); } });
  assert.equal(response, last);
  assert.equal(response.status, 409);
  assert.deepEqual(await response.json(), { code: 'idempotency_in_progress' });
  assert.deepEqual(waits, Array(30).fill(2000));
  assert.deepEqual(keys, Array(31).fill('still-running'));
});
test('SyncApi keeps key and exact wire body across retries and a JSON restart', async t => {
  const calls = [];
  t.mock.method(console, 'warn', () => {});
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    calls.push(init);
    return calls.length === 1 ? Response.json({ detail: 'Synthetic outage' }, { status: 503 }) : Response.json({ revision: 2 });
  });
  const pending = { ...requestIdFor(null, 'same'), body };
  await assert.rejects(new SyncApi('https://sync.test', null).sync('mount', body, pending.id), e => e instanceof SyncApiError && e.status === 503);
  const restored = JSON.parse(JSON.stringify(pending));
  await new SyncApi('https://sync.test', null).sync('mount', restored.body, restored.id);
  assert.equal(calls[0].headers['Idempotency-Key'], restored.id);
  assert.equal(calls[1].headers['Idempotency-Key'], restored.id);
  assert.equal(calls[0].body, calls[1].body);
  assert.equal(JSON.parse(calls[1].body).request_id, restored.id);
});
test('project link and multipart publish keep idempotency headers and encoded bytes', async t => {
  let calls = [];
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    calls.push(init);
    return Response.json({ revision: 1, version: 1 });
  });
  await new SyncApi('https://sync.test', null).link('synthetic', 'mount');
  assert.match(calls[0].headers['Idempotency-Key'], /^[0-9a-f-]{36}$/);
  calls = [];
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    calls.push(init);
    return calls.length === 1 ? Response.json({ code: 'idempotency_in_progress' }, { status: 409, headers: { 'Retry-After': '0' } }) : Response.json({ version: 1 });
  });
  await publishAtlas('https://sync.test', null, 'mount', 'synthetic', new Uint8Array([1, 2, 3]), 'publish-key');
  assert.equal(calls[0].headers['Idempotency-Key'], 'publish-key');
  assert.match(calls[0].headers['Content-Type'], /multipart\/form-data; boundary=/);
  assert.ok(calls[0].body instanceof Blob);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].body, calls[1].body);
  assert.deepEqual(calls[0].headers, calls[1].headers);
});
test('ETags revalidate detail snapshots; credentials and sign-out clear the cache', async t => {
  clearSyncCache(); let calls = 0;
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    calls++;
    if (calls === 2) { assert.equal(init.headers['If-None-Match'], '"r1"'); return new Response(null, { status: 304 }); }
    assert.equal(init.headers['If-None-Match'], undefined);
    return Response.json({ revision: 1 }, { headers: { ETag: '"r1"' } });
  });
  assert.deepEqual(await new SyncApi('https://cache.test', 'account-one').lookup('synthetic'), { revision: 1 });
  assert.deepEqual(await new SyncApi('https://cache.test', 'account-one').lookup('synthetic'), { revision: 1 });
  await new SyncApi('https://cache.test', 'account-two').lookup('synthetic');
  clearSyncCache(); await new SyncApi('https://cache.test', 'account-two').lookup('synthetic');
});
test('4,000 pending drafts derive status without any server calls', t => {
  const fetch = t.mock.method(globalThis, 'fetch', () => { throw new Error('No per-row requests'); });
  const start = performance.now();
  const drafts = Array.from({ length: 4000 }, (_, n) => ({ ...draft, uuid: `synthetic-${n}`, changes: { spots: 1, grains: 0, labels: 0 } }));
  const statuses = drafts.map(d => plainSyncStatus(d, draftSyncStatus(d)));
  assert.ok(statuses.every(s => s === 'Saved')); assert.equal(fetch.mock.callCount(), 0);
  t.diagnostic(`4,000 rows: 0 requests; ${(performance.now() - start).toFixed(1)} ms`);
});
