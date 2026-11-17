import assert from 'node:assert/strict';
import test from 'node:test';
import { setImmediate as settle } from 'node:timers/promises';
import { loadPublishedPixels } from './published-pixels.ts';
import { neutralTone } from './bundle-tone.ts';

function setup(t, count = 20) {
  const oldCanvas = globalThis.OffscreenCanvas, oldBitmap = globalThis.createImageBitmap;
  globalThis.OffscreenCanvas = class {
    getContext() { return { clearRect() {}, drawImage() {}, getImageData: () => ({ data: new Uint8Array([42, 42, 42, 255]) }) }; }
  };
  globalThis.createImageBitmap = async () => ({ width: 1, height: 1, close() {} });
  t.after(() => { globalThis.OffscreenCanvas = oldCanvas; globalThis.createImageBitmap = oldBitmap; });
  const areas = ['a', 'b'].map(id => ({ id, path: id, w: count, h: 1, params: { tone: neutralTone() } }));
  const project = { uuid: 'example', areas };
  const assembled = [], grains = [];
  const core = {
    async loadBundleArea(id, w, h, pixels) {
      assembled.push(id);
      assert.equal(pixels.length, w * h);
      assert.ok(pixels.every(p => p === 42));
      return { w, h, sha256: id };
    },
    async setGrains(id, value) { grains.push([id, value]); },
    async setPackageDigest() {},
  };
  const app = { remote: { server: 'https://pixels.test', projectId: 'example', token: null, access: null, atlas: {
    tile_size: 1, areas, files: [], package: { content_sha256: 'example', content_files: 1 },
  } }, core: Promise.resolve(core), fullProject: () => project, areaId: 'b', readOnly: false,
    params: { a: {}, b: {} }, grains: { a: [{ id: 'original-a' }], b: [] }, tone: {},
    stitched: { a: { marker: 'original' }, b: { marker: 'original' } }, bundle: null, pixelsVersion: 0,
    remoteTiles: {}, busy: null, run() { throw new Error('Background loading must not use the modal task runner'); } };
  return { app, assembled, grains };
}

function heldNetwork(t, at = 16) {
  let release, began, active = 0, max = 0;
  const paths = [], started = new Promise(resolve => { began = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  t.mock.method(globalThis, 'fetch', async (url, { signal }) => {
    paths.push(new URL(url).pathname);
    max = Math.max(max, ++active);
    if (paths.length === at) began();
    let stop;
    try {
      await Promise.race([gate, new Promise((_, reject) => {
        stop = () => reject(signal.reason);
        signal.addEventListener('abort', stop, { once: true });
      })]);
      signal.throwIfAborted();
      return new Response(new Uint8Array([42]));
    } finally { signal.removeEventListener('abort', stop); active--; }
  });
  return { paths, started, release: () => release(), maximum: () => max };
}

test('background loading prioritises current area, bounds fetches, and publishes all pixels and current grains together', async t => {
  const { app, assembled, grains } = setup(t);
  const network = heldNetwork(t);
  const original = app.stitched;
  const loading = loadPublishedPixels(app);
  assert.equal(loadPublishedPixels(app), loading, 'one load per tab');
  await network.started;
  assert.equal(network.paths.length, 16);
  assert.ok(network.paths.every(p => p.includes('/b/0/')));
  assert.equal(app.busy, null);
  assert.equal(app.bundle, null);
  assert.equal(app.stitched, original);
  assert.deepEqual(app.tone, {});
  app.areaId = 'a';
  app.grains.a = [{ id: 'edited-on-canvas' }];
  const core = await app.core, loadArea = core.loadBundleArea;
  core.loadBundleArea = async (...args) => {
    const mosaic = await loadArea(...args);
    if (args[0] === 'a') app.grains.b = [{ id: 'edited-after-b-assembled' }];
    return mosaic;
  };
  network.release();
  assert.equal(await loading, true);
  assert.deepEqual(assembled, ['b', 'a']);
  assert.deepEqual(grains, [['b', [{ id: 'edited-after-b-assembled' }]], ['a', [{ id: 'edited-on-canvas' }]]]);
  assert.deepEqual(Object.keys(app.bundle).sort(), ['a', 'b']);
  assert.equal(app.remoteTiles, null);
  assert.equal(app.pixelsLoad, null);
  assert.equal(app.cancelPixelsLoad, null);
  assert.equal(network.maximum(), 16);
  assert.equal(network.paths.length, 40);
});

test('stop is non-modal and retry finishes without exposing partial bundle state', async t => {
  const { app } = setup(t, 2), network = heldNetwork(t, 4);
  const original = app.stitched;
  const loading = loadPublishedPixels(app);
  await network.started;
  app.cancelPixelsLoad();
  assert.equal(await loading, false);
  assert.equal(app.bundle, null);
  assert.equal(app.stitched, original);
  assert.equal(app.pixelsLoad.error, 'Stopped.');
  assert.equal(app.busy, null);
  network.release();
  assert.equal(await loadPublishedPixels(app), true);
  assert.equal(app.pixelsLoad, null);
});

test('a tile error or size mismatch never exposes partially assembled pixels', async t => {
  for (const mismatch of [false, true]) {
    const { app } = setup(t, 1), original = app.stitched;
    t.mock.method(globalThis, 'fetch', async () => new Response(new Uint8Array([42]), { status: mismatch ? 200 : 503 }));
    if (mismatch) globalThis.createImageBitmap = async () => ({ width: 2, height: 1, close() {} });
    assert.equal(await loadPublishedPixels(app), false);
    assert.match(app.pixelsLoad.error, mismatch ? /dimensions/ : /503/);
    assert.equal(app.bundle, null);
    assert.equal(app.stitched, original);
    assert.deepEqual(app.tone, {});
  }
});

test('a project change or read-only transition stops loading before core writes', async t => {
  for (const readOnly of [false, true]) {
    const { app, assembled } = setup(t, 1), network = heldNetwork(t, 2);
    const original = app.stitched;
    const loading = loadPublishedPixels(app);
    await network.started;
    if (readOnly) app.readOnly = true;
    else app.remote = { ...app.remote, atlas: { ...app.remote.atlas } };
    network.release();
    assert.equal(await loading, false);
    assert.deepEqual(assembled, []);
    assert.equal(app.bundle, null);
    assert.equal(app.stitched, original);
  }
});

function heldDecodes() {
  const jobs = [];
  let automatic = false, live = 0, maximum = 0;
  globalThis.createImageBitmap = async blob => {
    const value = new Uint8Array(await blob.arrayBuffer())[0];
    maximum = Math.max(maximum, ++live);
    const bmp = { width: 1, height: 1, value, closed: 0, close() { this.closed++; live--; } };
    return new Promise((resolve, reject) => {
      const job = { bmp, resolve: () => resolve(bmp), reject: error => { live--; reject(error); } };
      jobs.push(job);
      if (automatic) job.resolve();
    });
  };
  return { jobs, maximum: () => maximum, release() { automatic = true; for (const job of jobs) job.resolve(); } };
}

test('fetching runs 64 tiles ahead of a stalled decoder, with eight ordered decodes and monotonic progress', async t => {
  const { app } = setup(t, 40), decodes = heldDecodes();
  const paths = [], progress = [], copied = [], assembled = [];
  let load;
  Object.defineProperty(app, 'pixelsLoad', {
    get: () => load,
    set(value) {
      load = value && new Proxy(value, { set(target, key, value) {
        if (key === 'done') progress.push(value);
        target[key] = value;
        return true;
      } });
    },
  });
  t.mock.method(globalThis, 'fetch', async url => {
    const path = new URL(url).pathname;
    paths.push(path);
    const value = Number(path.match(/\/(\d+)_0.webp$/)[1]) + (path.includes('/a/0/') ? 40 : 0);
    return new Response(new Uint8Array([value]));
  });
  globalThis.OffscreenCanvas = class {
    getContext() {
      let bmp;
      return { clearRect() {}, drawImage(value) { bmp = value; copied.push(bmp.value); },
        getImageData: () => ({ data: new Uint8Array([bmp.value, 99, 100, 255]) }) };
    }
  };
  const core = await app.core;
  core.loadBundleArea = async (id, w, h, pixels) => {
    assembled.push(id);
    assert.deepEqual([...pixels], Array.from({ length: 40 }, (_, i) => i + (id === 'a' ? 40 : 0)));
    return { w, h, sha256: id };
  };
  core.setPackageDigest = async () => assert.equal(app.pixelsLoad.done, app.pixelsLoad.total);
  const loading = loadPublishedPixels(app);
  await settle();
  assert.equal(paths.length, 64, 'network refills without waiting for a decode');
  assert.equal(app.pixelsLoad.done, 64);
  assert.equal(decodes.jobs.length, 8);
  assert.deepEqual(assembled, []);
  for (const job of decodes.jobs.slice(1).reverse()) job.resolve();
  await settle();
  assert.equal(paths.length, 64, 'compressed look-ahead remains bounded');
  assert.equal(decodes.jobs.length, 8, 'decoded look-ahead remains bounded');
  assert.deepEqual(copied, [], 'out-of-order decodes wait for the first tile');
  decodes.release();
  assert.equal(await loading, true);
  assert.deepEqual(assembled, ['b', 'a']);
  assert.deepEqual(copied, Array.from({ length: 80 }, (_, i) => i));
  assert.deepEqual(progress, Array.from({ length: 80 }, (_, i) => i + 1));
  assert.equal(decodes.maximum(), 8);
  assert.equal(decodes.jobs.length, 80);
  assert.ok(decodes.jobs.every(job => job.bmp.closed === 1));
});

test('all queued and late bitmaps close on stop, decode failure, dimension mismatch, or copy failure', async t => {
  for (const mode of ['stop', 'decode', 'dimensions', 'copy', 'project']) await t.test(mode, async t => {
    const { app, assembled } = setup(t, 40), decodes = heldDecodes(), original = app.stitched;
    let fetched = 0;
    t.mock.method(globalThis, 'fetch', async () => { fetched++; return new Response(new Uint8Array([42])); });
    if (mode === 'copy') globalThis.OffscreenCanvas = class {
      getContext() { return { clearRect() {}, drawImage() { throw new Error('Copy failed'); } }; }
    };
    const loading = loadPublishedPixels(app);
    await settle();
    assert.equal(decodes.jobs.length, 8);
    // Leave one decode pending until after the failure, with six ready bitmaps.
    for (const job of decodes.jobs.slice(1, 7)) job.resolve();
    await settle();
    const first = decodes.jobs[0];
    if (mode === 'stop') app.cancelPixelsLoad();
    if (mode === 'project') app.readOnly = true;
    if (mode === 'dimensions') first.bmp.width = 2;
    if (mode === 'decode') first.reject(new Error('Decode failed'));
    else first.resolve();
    await settle();
    for (const job of decodes.jobs.slice(1, 7)) assert.equal(job.bmp.closed, 1, 'ready bitmaps close without waiting for late decodes');
    decodes.jobs[7].resolve();
    assert.equal(await loading, false);
    assert.equal(fetched, 64, 'failure does not refill the network queue');
    assert.equal(decodes.jobs.length, 8);
    for (const job of decodes.jobs) assert.equal(job.bmp.closed, mode === 'decode' && job === first ? 0 : 1);
    if (mode !== 'project') assert.match(app.pixelsLoad.error, { stop: /Stopped/, decode: /Decode failed/, dimensions: /dimensions/, copy: /Copy failed/ }[mode]);
    assert.deepEqual(assembled, []);
    assert.equal(app.stitched, original);
    assert.equal(app.bundle, null);
  });
});

test('row copying preserves red-channel bytes across full and partial edge tiles', async t => {
  const { app } = setup(t, 1);
  app.remote.atlas.tile_size = 2;
  for (const area of app.remote.atlas.areas) { area.w = 3; area.h = 3; }
  t.mock.method(globalThis, 'fetch', async url => {
    const [, x, y] = new URL(url).pathname.match(/\/(\d+)_(\d+).webp$/);
    return new Response(new Uint8Array([Number(x), Number(y)]));
  });
  globalThis.createImageBitmap = async blob => {
    const [x, y] = new Uint8Array(await blob.arrayBuffer());
    return { x, y, width: x ? 1 : 2, height: y ? 1 : 2, close() {} };
  };
  globalThis.OffscreenCanvas = class {
    getContext() {
      let bmp;
      return { clearRect() {}, drawImage(value) { bmp = value; }, getImageData(_x, _y, w, h) {
        const data = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
          data.set([(bmp.y * 2 + y) * 3 + bmp.x * 2 + x, 99, 100, 255], (y * w + x) * 4);
        }
        return { data };
      } };
    }
  };
  const core = await app.core;
  core.loadBundleArea = async (id, w, h, pixels) => {
    assert.deepEqual([...pixels], [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    return { w, h, sha256: id };
  };
  assert.equal(await loadPublishedPixels(app), true);
});
