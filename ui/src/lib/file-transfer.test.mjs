import assert from 'node:assert/strict';
import test from 'node:test';
import { downloadFile } from './file-transfer.ts';

test('a stalled stream aborts, retries a file three times with backoff and leaves no total size cap', async t => {
  let calls = 0, cancelled = 0;
  const waits = [];
  t.mock.method(globalThis, 'fetch', async () => {
    calls++;
    return new Response(new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1])); }, cancel() { cancelled++; } }));
  });
  await assert.rejects(downloadFile('https://files.test/stalled', {}, new AbortController().signal, { stallMs: 5, wait: async ms => { waits.push(ms); } }), /stalled/);
  assert.equal(calls, 4); assert.equal(cancelled, 4);
  assert.deepEqual(waits, [1000, 2000, 4000]);
});
test('a slow continuous transfer lives longer than its stall deadline and returns every chunk', async t => {
  t.mock.method(globalThis, 'fetch', async () => new Response(new ReadableStream({
    async start(c) {
      for (let i = 0; i < 5; i++) { c.enqueue(new Uint8Array([i])); await new Promise(resolve => setTimeout(resolve, 5)); }
      c.close();
    },
  })));
  const bytes = await downloadFile('https://files.test/slow', {}, new AbortController().signal, { stallMs: 20 });
  assert.deepEqual(new Uint8Array(bytes), new Uint8Array([0, 1, 2, 3, 4]));
});
test('connect timeout retries and caller cancellation never retries', async t => {
  let calls = 0;
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    calls++;
    return new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason), { once: true }));
  });
  await assert.rejects(downloadFile('https://files.test/connect', {}, new AbortController().signal, { connectMs: 5, wait: async () => {} }), /database did not answer/);
  assert.equal(calls, 4);
  calls = 0;
  const controller = new AbortController();
  const pending = downloadFile('https://files.test/cancel', {}, controller.signal);
  controller.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(calls, 1);
});
test('HTTP errors include the status and reply excerpt; 404 is not retried', async t => {
  let calls = 0;
  t.mock.method(globalThis, 'fetch', async () => { calls++; return new Response('Synthetic missing tile', { status: 404 }); });
  await assert.rejects(downloadFile('https://files.test/missing', {}, new AbortController().signal), /404.*Synthetic missing tile/);
  assert.equal(calls, 1);
});

test('an error response body also stops on stalls and caller cancellation after headers', async t => {
  let cancelled = 0;
  t.mock.method(globalThis, 'fetch', async () => new Response(new ReadableStream({
    start(c) { c.enqueue(new TextEncoder().encode('Synthetic partial error')); },
    cancel() { cancelled++; },
  }), { status: 503 }));
  await assert.rejects(downloadFile('https://files.test/error-stall', {}, new AbortController().signal, { stallMs: 5, wait: async () => {} }), /stalled/);
  assert.equal(cancelled, 4);
  const controller = new AbortController();
  const pending = downloadFile('https://files.test/error-cancel', {}, controller.signal);
  await new Promise(resolve => setImmediate(resolve));
  controller.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(cancelled, 5);
});
