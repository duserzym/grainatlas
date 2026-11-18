import assert from 'node:assert/strict';
import test from 'node:test';
import { fetchWithTimeout, HttpNetworkError, databaseOfflineMessage } from './http.ts';
import { SyncApi, SyncNetworkError } from './sync-api.ts';

test('response timeout aborts a server that never sends headers', async t => {
  let signal;
  t.mock.method(globalThis, 'fetch', (_url, init) => {
    signal = init.signal;
    return new Promise(() => {});
  });
  await assert.rejects(fetchWithTimeout('https://offline.test', {}, { responseTimeoutMs: 10 }),
    e => e instanceof HttpNetworkError && e.message === databaseOfflineMessage);
  assert.equal(signal.aborted, true);
});

test('caller cancellation combines init and option signals before headers arrive', async t => {
  for (const afterHeaders of [false]) {
    const initAbort = new AbortController(), optionAbort = new AbortController();
    let fetchSignal;
    t.mock.method(globalThis, 'fetch', (_url, init) => {
      fetchSignal = init.signal;
      return afterHeaders ? Promise.resolve(new Response(new ReadableStream())) : new Promise(() => {});
    });
    const response = fetchWithTimeout('https://offline.test', { signal: initAbort.signal }, { signal: optionAbort.signal });
    const read = afterHeaders ? (await response).text() : response;
    const reason = new Error('cancelled');
    (afterHeaders ? optionAbort : initAbort).abort(reason);
    await assert.rejects(read, e => e === reason);
    assert.equal(fetchSignal.aborted, true);
  }
});

test('an already cancelled caller never starts a request', async t => {
  const fetch = t.mock.method(globalThis, 'fetch', () => { throw new Error('Unexpected request'); });
  const controller = new AbortController(); controller.abort();
  await assert.rejects(fetchWithTimeout('https://offline.test', {}, { signal: controller.signal }), { name: 'AbortError' });
  assert.equal(fetch.mock.callCount(), 0);
});

test('a large progressing body has no total size or duration cap', async t => {
  let chunks = 0;
  t.mock.method(globalThis, 'fetch', async () => new Response(new ReadableStream({
    async pull(c) {
      await new Promise(resolve => setTimeout(resolve, 5));
      if (chunks++ === 20) c.close();
      else c.enqueue(new Uint8Array(256 * 1024));
    },
  }), { headers: { ETag: 'large' } }));
  const start = performance.now();
  const r = await fetchWithTimeout('https://large.test', {}, { responseTimeoutMs: 20, stallTimeoutMs: 50 });
  assert.equal((await r.arrayBuffer()).byteLength, 20 * 256 * 1024);
  assert.equal(r.headers.get('ETag'), 'large');
  assert.ok(performance.now() - start > 50);
});

test('SyncApi maps lost headers and lost response bodies to SyncNetworkError', async t => {
  t.mock.method(console, 'warn', () => {});
  t.mock.method(globalThis, 'fetch', async () => { throw new TypeError('Network lost'); });
  await assert.rejects(new SyncApi('https://offline.test', null).lookup('invented'), e => e instanceof SyncNetworkError);
  t.mock.method(globalThis, 'fetch', async () => new Response(new ReadableStream({
    start(c) { c.error(new TypeError('Body lost')); },
  })));
  await assert.rejects(new SyncApi('https://offline.test', null).lookup('invented'), e => e instanceof SyncNetworkError);
});

test('4,000 rows still use one request with the timeout wrapper', async t => {
  const rows = Array.from({ length: 4000 }, (_, n) => ({ id: `invented-${n}`, name: `Synthetic ${n}` }));
  const fetch = t.mock.method(globalThis, 'fetch', async () => Response.json(rows));
  const before = performance.now();
  assert.deepEqual(await (await globalThis.fetch('https://large.test')).json(), rows);
  const baseline = performance.now() - before;
  assert.equal(fetch.mock.callCount(), 1);
  const after = performance.now();
  assert.deepEqual(await (await fetchWithTimeout('https://large.test')).json(), rows);
  const wrapped = performance.now() - after;
  assert.equal(fetch.mock.callCount(), 2);
  t.diagnostic(`4,000 rows before: 1 request, ${baseline.toFixed(1)} ms; after: 1 request, ${wrapped.toFixed(1)} ms`);
});

test('writes get a response window that grows with the body', async () => {
  const { writeTimeout } = await import('./http.ts');
  assert.equal(writeTimeout({}), 8000);
  assert.equal(writeTimeout({ method: 'GET' }), 8000);
  assert.equal(writeTimeout({ method: 'POST', body: '{}' }), 61000);
  assert.equal(writeTimeout({ method: 'POST', body: new Blob([new Uint8Array(64_000_000)]) }), 1_060_000);
});
