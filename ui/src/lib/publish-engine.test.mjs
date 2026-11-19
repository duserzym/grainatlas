import assert from 'node:assert/strict';
import test from 'node:test';
import { PART_BYTES, PublishError, resumablePublishing, uploadImages } from './publish-engine.ts';
import { imagesMayRun } from './image-idle.ts';
import { BackgroundSync } from './background-sync.ts';
import { publishAtlas } from './upbdb.ts';
import { writeTimeout } from './http.ts';

let sequence = 0;
function setup(t, count = 4) {
  const server = `https://images-${sequence++}.test`;
  const files = Array.from({ length: count }, (_, i) => ({ path: `tiles/${i}.webp`, bytes: new Uint8Array([i % 256]) }));
  const atlas = { project_id: 'synthetic', files: files.map((f, i) => ({ path: f.path, size: f.bytes.length, sha256: String(i).padStart(64, '0') })) };
  const bundle = { atlas, files, zip: new Uint8Array(count * 100), atlasSha256: 'manifest' };
  let job = { server, mount: 'mount', account: 'synthetic-account', atlas_sha256: 'manifest', start_key: 'start', commit_key: 'commit', zip_key: 'zip',
    uploaded: 0, exported: '2026-01-01T00:00:00Z', pixels_key: 'pixels', replace_current: false };
  const calls = [], have = new Set(), waits = [], progress = [];
  let handler;
  const fetch = t.mock.method(globalThis, 'fetch', async (url, init = {}) => {
    const path = new URL(url).pathname;
    const headers = new Headers(init.headers);
    const call = { path, method: init.method ?? 'GET', init, headers };
    calls.push(call);
    const override = await handler?.(call);
    if (override) return override;
    if (path === '/v1/assets/check') return Response.json({ have: [] });
    if (path.endsWith('/atlas/uploads')) return Response.json({ upload_id: 'upload', missing: atlas.files.map(f => f.sha256).filter(h => !have.has(h)), expires_at: '2099-01-01T00:00:00Z' }, { status: 201 });
    if (call.method === 'GET') return Response.json({ state: 'open', missing: atlas.files.map(f => f.sha256).filter(h => !have.has(h)), received: have.size, total: count }, { headers: { ETag: '"status-1"' } });
    if (call.method === 'PUT') { have.add(path.split('/').at(-1)); return Response.json({ stored: true }, { status: 201 }); }
    return Response.json({ version: 2 });
  });
  const run = () => uploadImages({ server, mount: 'mount', token: 'synthetic-token', bundle, job,
    save: value => { job = structuredClone(value); }, gate: async () => {}, wait: async ms => { waits.push(ms); }, progress: (done, total) => progress.push([done, total]) });
  return { server, files, atlas, bundle, calls, have, waits, progress, fetch, run, get job() { return job; }, set job(v) { job = v; }, handler(fn) { handler = fn; } };
}

test('capability is once per server with an empty read-like body and no idempotency header', async t => {
  const s = setup(t);
  assert.equal(await resumablePublishing(s.server, 'one'), true);
  assert.equal(await resumablePublishing(s.server, 'two'), true);
  assert.equal(s.calls.length, 1);
  assert.deepEqual(JSON.parse(s.calls[0].init.body), { sha256: [] });
  assert.equal(s.calls[0].headers.has('Idempotency-Key'), false);
});
for (const status of [404, 405, 501]) test(`capability ${status} falls back to one zip write and caches the result`, async t => {
  const s = setup(t);
  s.handler(c => {
    if (c.path === '/v1/assets/check') return new Response(null, { status });
  });
  await s.run();
  assert.deepEqual(s.calls.map(c => c.path), ['/v1/assets/check', '/v1/mounts/mount/atlas']);
  assert.equal(s.calls[1].headers.get('Idempotency-Key'), 'zip');
  assert.equal(await resumablePublishing(s.server, null), false);
});

test('a network failure at capability check stops publishing without caching unsupported routes', async t => {
  const s = setup(t);
  s.handler(() => { throw new TypeError('Synthetic unreachable server'); });
  await assert.rejects(s.run(), e => e instanceof PublishError && e.status === 0 && e.transient);
  assert.deepEqual(s.calls.map(c => c.path), ['/v1/assets/check']);
  s.handler(() => undefined);
  await s.run();
  assert.equal(s.calls.filter(c => c.path === '/v1/assets/check').length, 2);
  assert.equal(s.calls.some(c => c.path.endsWith('/atlas')), false);
});

test('capability, creation, PUT, commit, status and zip all use body-scaled request deadlines', async t => {
  const s = setup(t, 1);
  s.files[0].bytes = new Uint8Array(128_000);
  s.atlas.files[0].size = 128_000;
  const deadlines = [];
  const timeout = AbortSignal.timeout.bind(AbortSignal);
  t.mock.method(AbortSignal, 'timeout', ms => { deadlines.push(ms); return timeout(ms); });
  await s.run();
  s.job = { ...s.job, upload_id: 'upload' };
  await s.run();
  await publishAtlas(s.server, null, 'mount', 'synthetic', new Uint8Array(128_000), 'zip');
  assert.equal(deadlines.length, s.calls.length);
  s.calls.forEach((c, i) => {
    assert.equal(deadlines[i], writeTimeout(c.init), c.path);
    assert.ok(c.init.signal instanceof AbortSignal, c.path);
  });
  assert.equal(deadlines[s.calls.findIndex(c => c.method === 'PUT')], 62_000);
  assert.equal(deadlines[s.calls.findIndex(c => c.method === 'GET')], 8000);
  assert.ok(deadlines.at(-1) > 62_000, 'multipart encoding counts toward the zip write deadline');
});

for (const stage of ['creation', 'status', 'commit']) test(`a stalled ${stage} reply is a retryable failure, not caller cancellation`, async t => {
  const s = setup(t, 1);
  if (stage === 'status') s.job = { ...s.job, upload_id: 'upload' };
  const timeout = AbortSignal.timeout.bind(AbortSignal);
  t.mock.method(AbortSignal, 'timeout', () => timeout(5));
  s.handler(c => {
    if (!(stage === 'creation' && c.path.endsWith('/atlas/uploads') || stage === 'status' && c.method === 'GET' || stage === 'commit' && c.path.endsWith('/commit'))) return;
    return new Response(new ReadableStream({ start(controller) {
      c.init.signal.addEventListener('abort', () => controller.error(new DOMException('Reply aborted', 'AbortError')), { once: true });
    } }));
  });
  // Native timeout signals are unref'ed; keep the test alive while the reply stalls.
  await Promise.all([
    assert.rejects(s.run(), e => e instanceof PublishError && e.status === 0 && e.transient),
    new Promise(resolve => setTimeout(resolve, 30)),
  ]);
});

test('fresh upload puts only missing hashes, at most two at once, then commits', async t => {
  const s = setup(t, 30);
  s.have.add(s.atlas.files[0].sha256);
  let active = 0, maximum = 0;
  s.handler(async c => {
    if (c.method !== 'PUT') return;
    maximum = Math.max(maximum, ++active);
    await new Promise(resolve => setImmediate(resolve));
    active--;
  });
  assert.equal((await s.run()).version, 2);
  assert.equal(maximum, 2);
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 29);
  assert.deepEqual(s.progress.at(-1), [30, 30]);
  assert.equal(s.calls.at(-1).headers.get('Idempotency-Key'), 'commit');
  assert.equal(JSON.parse(s.calls[1].init.body).replace_current, false);
});
test('a JSON-restored job resumes with present blobs, revalidates status with ETag and replays commit', async t => {
  const s = setup(t);
  s.job = JSON.parse(JSON.stringify({ ...s.job, upload_id: 'resume' }));
  s.have.add(s.atlas.files[0].sha256);
  await s.run();
  assert.equal(s.calls.some(c => c.path.endsWith('/atlas/uploads')), false);
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 3);
  s.calls.length = 0;
  s.handler(c => c.method === 'GET' ? new Response(null, { status: 304 }) : undefined);
  await s.run();
  const get = s.calls.find(c => c.method === 'GET');
  assert.equal(get.headers.get('If-None-Match'), '"status-1"');
  assert.equal(s.calls.at(-1).headers.get('Idempotency-Key'), 'commit');
});
for (const expiry of ['date', 'state', 404, 410]) test(`expired upload (${expiry}) starts again with a fresh key and retained server blobs`, async t => {
  const s = setup(t);
  s.job = { ...s.job, upload_id: 'old', expires_at: expiry === 'date' ? '2000-01-01T00:00:00Z' : '2099-01-01T00:00:00Z' };
  s.have.add(s.atlas.files[0].sha256);
  s.handler(c => c.method === 'GET' ? typeof expiry === 'number' ? Response.json({}, { status: expiry }) : Response.json({ state: 'expired' }) : undefined);
  await s.run();
  assert.notEqual(s.job.start_key, 'start');
  assert.notEqual(s.job.commit_key, 'commit');
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 3);
});
test('incomplete commit uploads the listed hashes and uses a new commit identity (4xx are cached)', async t => {
  const s = setup(t);
  for (const f of s.atlas.files) s.have.add(f.sha256);
  let commits = 0;
  s.handler(c => c.path.endsWith('/commit') && ++commits === 1 ? Response.json({ code: 'incomplete', missing: [s.atlas.files[1].sha256] }, { status: 409 }) : undefined);
  await s.run();
  assert.equal(commits, 2);
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 1);
  const keys = s.calls.filter(c => c.path.endsWith('/commit')).map(c => c.headers.get('Idempotency-Key'));
  assert.notEqual(keys[0], keys[1]);
});
test('replace_confirm persists an attention decision and never retries as replace automatically', async t => {
  const s = setup(t);
  s.handler(c => c.path.endsWith('/atlas/uploads') ? Response.json({ code: 'replace_confirm' }, { status: 409 }) : undefined);
  await assert.rejects(s.run(), e => e instanceof PublishError && e.code === 'replace_confirm');
  assert.equal(s.job.decision, 'ask');
  const before = s.calls.length;
  await assert.rejects(s.run(), e => e.code === 'replace_confirm');
  assert.equal(s.calls.length, before);
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 0);
});
test('chunked file follows 308 Range including partial retained bytes after restart', async t => {
  const s = setup(t, 1);
  s.files[0].bytes = new Uint8Array(PART_BYTES * 2 + 50);
  s.atlas.files[0].size = s.files[0].bytes.length;
  const ranges = [];
  s.handler(c => {
    if (c.method !== 'PUT') return;
    const range = c.headers.get('Content-Range'); ranges.push(range);
    if (ranges.length === 1) return new Response(null, { status: 308, headers: { Range: `bytes=0-${PART_BYTES + 19}` } });
    if (ranges.length === 2) return new Response(null, { status: 308, headers: { Range: `bytes=0-${PART_BYTES * 2 + 19}` } });
  });
  await s.run();
  assert.deepEqual(ranges, [`bytes 0-${PART_BYTES - 1}/${PART_BYTES * 2 + 50}`, `bytes ${PART_BYTES + 20}-${PART_BYTES * 2 + 19}/${PART_BYTES * 2 + 50}`, `bytes ${PART_BYTES * 2 + 20}-${PART_BYTES * 2 + 49}/${PART_BYTES * 2 + 50}`]);
});
test('network and 503 retries retain body and key, honour Retry-After, including a PUT', async t => {
  const s = setup(t, 1);
  const tried = new Map();
  s.handler(c => {
    if (c.path === '/v1/assets/check') return;
    const previous = tried.get(c.path);
    if (!previous) { tried.set(c.path, c); return Response.json({ detail: 'Synthetic outage' }, { status: 503, headers: { 'Retry-After': '12' } }); }
    assert.equal(c.headers.get('Idempotency-Key'), previous.headers.get('Idempotency-Key'));
    assert.equal(c.init.body, previous.init.body);
  });
  await s.run();
  assert.deepEqual(s.waits, [12000, 12000, 12000]);
});
test('zip request encoding and identity survive a JSON restart', async t => {
  const bodies = [], headers = [];
  t.mock.method(globalThis, 'fetch', async (_url, init) => { bodies.push(await init.body.arrayBuffer()); headers.push(init.headers); return Response.json({ version: 1 }); });
  for (let i = 0; i < 2; i++) await publishAtlas('https://zip.test', null, 'mount', 'synthetic', new Uint8Array([1, 2]), 'stable');
  assert.deepEqual(bodies[0], bodies[1]); assert.deepEqual(headers[0], headers[1]);
});
test('image idle gates stop edits, attention, hidden windows, element work and Area loading', t => {
  const gates = { elementsIdle: true, attention: false, visible: true, loading: false };
  assert.equal(imagesMayRun(100, 10099, gates), false);
  assert.equal(imagesMayRun(100, 10100, gates), true);
  for (const [key, value] of Object.entries({ elementsIdle: false, attention: true, visible: false, loading: true })) assert.equal(imagesMayRun(100, 10100, { ...gates, [key]: value }), false);
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let runs = 0;
  const timer = new BackgroundSync(() => runs++, () => .5, 10000);
  timer.edit(); t.mock.timers.tick(9999); assert.equal(runs, 0);
  timer.edit(); t.mock.timers.tick(9999); assert.equal(runs, 0);
  t.mock.timers.tick(1); assert.equal(runs, 1);
});
test('4,000-file changed manifest uses one creation and only the two absent blobs (no per-row checks)', async t => {
  const s = setup(t, 4000);
  for (const f of s.atlas.files.slice(2)) s.have.add(f.sha256);
  const before = performance.now();
  await publishAtlas(s.server, null, 'mount', 'synthetic', s.bundle.zip, 'baseline');
  const oldMs = performance.now() - before;
  s.calls.length = 0;
  const start = performance.now(); await s.run(); const newMs = performance.now() - start;
  assert.equal(s.calls.length, 5);
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 2);
  t.diagnostic(`4,000 synthetic files: zip baseline 1 request / ${s.bundle.zip.length} bundle bytes / ${oldMs.toFixed(1)} ms; deduplicated 5 requests (includes capability), 2 blob bytes / ${newMs.toFixed(1)} ms (mock timings).`);
});

test('a replacement race at commit also persists a decision without confirming it', async t => {
  const s = setup(t);
  s.handler(c => c.path.endsWith('/commit') ? Response.json({ code: 'replace_confirm' }, { status: 409 }) : undefined);
  await assert.rejects(s.run(), e => e.code === 'replace_confirm');
  assert.equal(s.job.decision, 'ask'); assert.equal(s.job.replace_current, false);
});
test('duplicate content hashes are uploaded once while progress counts every manifest path', async t => {
  const s = setup(t, 3);
  s.atlas.files[1].sha256 = s.atlas.files[0].sha256;
  s.files[1].bytes = s.files[0].bytes;
  await s.run();
  assert.equal(s.calls.filter(c => c.method === 'PUT').length, 2);
  assert.deepEqual(s.progress.at(-1), [3, 3]);
});
test('a lost connection on creation retries the identical request and key', async t => {
  const s = setup(t);
  let first;
  s.handler(c => {
    if (!c.path.endsWith('/atlas/uploads')) return;
    if (!first) { first = c; throw new TypeError('Synthetic disconnected relay'); }
    assert.equal(c.init.body, first.init.body);
    assert.equal(c.headers.get('Idempotency-Key'), first.headers.get('Idempotency-Key'));
  });
  await s.run(); assert.equal(s.waits.length, 1);
});

test('two concurrent tab jobs share the two-upload budget', async t => {
  const first = setup(t, 6), second = setup(t, 6);
  let active = 0, maximum = 0;
  second.handler(async c => {
    if (c.method !== 'PUT') return;
    maximum = Math.max(maximum, ++active);
    await new Promise(resolve => setImmediate(resolve)); active--;
  });
  await Promise.all([first.run(), second.run()]);
  assert.equal(maximum, 2);
});

test('recipe-only project publications reuse 4000 existing image assets and return the saved recipe', async t => {
  const s=setup(t,4001);
  for(const f of s.atlas.files.slice(0,4000))s.have.add(f.sha256);
  const projectFile=s.files[4000], entry=s.atlas.files[4000];
  projectFile.path=entry.path='project.json';
  let stored;
  s.handler(c=>{
    if(c.method==='PUT' && c.path.endsWith(entry.sha256))stored=JSON.parse(new TextDecoder().decode(c.init.body));
  });
  const measurements=[];
  for(const image_edits of [undefined,{version:1,areas:{a:{rotate_deg:90,flip_h:true,flip_v:false,crop:{kind:'rect',points_um:[[0,0],[100,100]]},levels:{black:18,white:236},masks:[{points_um:[[1,2],[3,4]],radius_um:5}]}}}]) {
    s.have.delete(entry.sha256);projectFile.bytes=new TextEncoder().encode(JSON.stringify({schema_version:1,uuid:'synthetic',image_edits}));entry.size=projectFile.bytes.length;
    const start=s.calls.length,time=performance.now();await s.run();
    const calls=s.calls.slice(start),ms=performance.now()-time;
    assert.equal(calls.filter(c=>c.method==='PUT').length,1);assert.deepEqual(stored.image_edits,image_edits);
    assert.equal(calls.some(c=>c.path.includes('/elements')),false);
    measurements.push({recipe:!!image_edits,requests:calls.length,uploadedBytes:projectFile.bytes.length,ms:Number(ms.toFixed(2))});
    s.job={...s.job,upload_id:undefined,start_key:'recipe-start',commit_key:'recipe-commit'};
  }
  t.diagnostic(`4000 reused image assets, before/after recipe: ${JSON.stringify(measurements)} (mock timings; capability cached after first publication).`);
});
