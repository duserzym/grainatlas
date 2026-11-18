import assert from 'node:assert/strict';
import test from 'node:test';
import { fileRequest, publishedFile } from './published-files.ts';

const remote = () => ({ server: 'https://images.test', projectId: 'example', token: 'example-token', access: 'signed value',
  version: 7, atlas: { files: [{ path: 'a/0/0_0.webp', sha256: 'f'.repeat(64) }] } });

test('versioned requests preserve access and bearer auth and use HTTP caching', () => {
  const r = remote(), [url, init] = fileRequest(r, 'a/0/0_0.webp');
  assert.equal(new URL(url).searchParams.get('version'), '7');
  assert.equal(new URL(url).searchParams.get('access'), 'signed value');
  assert.equal(init.cache, 'default');
  assert.equal(init.headers.Authorization, 'Bearer example-token');
  delete r.version;
  const [oldUrl, oldInit] = fileRequest(r, 'a/0/0_0.webp');
  assert.equal(new URL(oldUrl).searchParams.has('version'), false);
  assert.equal(oldInit.cache, 'no-cache');
});

test('pyramid and full-resolution consumers share requests; stopping one leaves other readers usable', async t => {
  const r = remote();
  let release, requests = 0, requestSignal;
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    requests++;
    requestSignal = init.signal;
    await new Promise(resolve => { release = resolve; });
    return new Response(new Uint8Array([1, 2, 3]));
  });
  const controller = new AbortController();
  const drawing = publishedFile(r, 'a/0/0_0.webp');
  const loading = publishedFile(r, 'a/0/0_0.webp', controller.signal);
  // Cache failure (there is no IndexedDB in Node) still reaches the network.
  while (!release) await new Promise(resolve => setImmediate(resolve));
  controller.abort();
  await assert.rejects(loading, { name: 'AbortError' });
  assert.equal(requestSignal.aborted, false);
  release();
  assert.deepEqual(new Uint8Array(await drawing), new Uint8Array([1, 2, 3]));
  assert.equal(requests, 1);
});

test('stopping the last reader aborts its network request; missing hashes still load', async t => {
  const r = remote(), controller = new AbortController();
  let began;
  const started = new Promise(resolve => { began = resolve; });
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    began();
    return new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason), { once: true }));
  });
  const pending = publishedFile(r, 'unlisted.webp', controller.signal);
  await started;
  controller.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  t.mock.method(globalThis, 'fetch', async () => new Response(new Uint8Array([5])));
  assert.deepEqual(new Uint8Array(await publishedFile(r, 'unlisted.webp')), new Uint8Array([5]));
});

test('sign-out aborts readers and clears the image cache', async t => {
  const { clearPublishedFiles } = await import('./published-files.ts');
  const { imageCache } = await import('./tile-cache.ts');
  const clear = t.mock.method(imageCache, 'clear', async () => true);
  let began;
  const started = new Promise(resolve => { began = resolve; });
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    began();
    return new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason), { once: true }));
  });
  const loading = publishedFile(remote(), 'sign-out.webp');
  const rejected = assert.rejects(loading, { name: 'AbortError' });
  await started;
  await clearPublishedFiles(); await rejected;
  assert.equal(clear.mock.callCount(), 1);
});
