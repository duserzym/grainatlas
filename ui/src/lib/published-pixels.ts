import { bundlePixels, isNeutral, overlayFor, type BundleSources } from './bundle-tone.ts';
import { publishedFile } from './published-files.ts';
import type { Remote } from './remote';
import type { AppState } from './store.svelte';

// One background load per tab, independent of the modal task runner.
const loading = new WeakMap<AppState, { remote: Remote; promise: Promise<boolean>; cancel: () => void }>();
export function loadPublishedPixels(app: AppState): Promise<boolean> {
  if (bundlePixels(app)) return Promise.resolve(true);
  if (!app.remote || app.readOnly) return Promise.resolve(false);
  const remote = app.remote;
  const pending = loading.get(app);
  if (pending) {
    if (pending.remote.atlas === remote.atlas) return pending.promise;
    pending.cancel();
    return pending.promise.then(() => loadPublishedPixels(app));
  }
  const project = app.fullProject()!;
  const controller = new AbortController();
  const sameProject = () => app.remote?.atlas === remote.atlas && app.remote?.server === remote.server && app.remote?.projectId === remote.projectId && !app.readOnly;
  function check() {
    controller.signal.throwIfAborted();
    if (!sameProject()) throw new Error('Published image loading stopped.');
  }
  const cancel = () => controller.abort();
  app.cancelPixelsLoad = cancel;
  const tile = remote.atlas.tile_size;
  const areas = [...remote.atlas.areas].sort((a, b) => Number(b.id === app.areaId) - Number(a.id === app.areaId));
  const tiles = areas.flatMap(area => Array.from({ length: Math.ceil(area.w / tile) * Math.ceil(area.h / tile) }, (_, i) => {
    const x = i % Math.ceil(area.w / tile), y = Math.floor(i / Math.ceil(area.w / tile));
    return { area, x, y, path: `${area.path}/0/${x}_${y}.webp` };
  }));
  app.pixelsLoad = { done: 0, total: tiles.length, error: null };
  const promise = (async () => {
    // Windows span area boundaries, but only the current area has an assembled
    // image. Fetch completion refills the network pool independently of decoding.
    // Include in-flight requests and decodes in the 64-tile look-ahead limit.
    const ahead = new Map<number, Promise<{ bytes: ArrayBuffer } | { error: unknown }>>();
    const decoded = new Map<number, Promise<{ bmp: ImageBitmap } | { error: unknown }>>();
    let index = 0, nextFetch = 0, fetching = 0, nextDecode = 0;
    let failure: unknown;
    function fetchAhead() {
      while (!controller.signal.aborted && fetching < 16 && nextFetch < Math.min(tiles.length, index + 64)) startFetch(nextFetch++);
    }
    function startFetch(index: number) {
      fetching++;
      ahead.set(index, (async () => {
        try {
          check();
          const bytes = await publishedFile(remote, tiles[index].path, controller.signal);
          check();
          if (app.pixelsLoad) app.pixelsLoad.done++;
          return { bytes };
        } catch (error) {
          failure ??= error;
          controller.abort();
          return { error };
        } finally {
          fetching--;
          fetchAhead();
        }
      })());
    }
    function decodeAhead() {
      // At most eight bitmaps are decoding or waiting for the ordered copy.
      while (!controller.signal.aborted && nextDecode < Math.min(tiles.length, index + 8)) {
        const at = nextDecode++;
        decoded.set(at, (async () => {
          let bmp: ImageBitmap | undefined;
          try {
            const result = await ahead.get(at)!;
            ahead.delete(at);
            if ('error' in result) throw failure ?? result.error;
            check();
            bmp = await createImageBitmap(new Blob([result.bytes], { type: 'image/webp' }), { colorSpaceConversion: 'none', premultiplyAlpha: 'none' });
            check();
            return { bmp };
          } catch (error) {
            bmp?.close();
            failure ??= error;
            controller.abort();
            return { error };
          }
        })());
      }
    }
    try {
      const core = await app.core;
      check();
      fetchAhead();
      decodeAhead();
      const sources: BundleSources = {};
      const mosaics: typeof app.stitched = {};
      const canvas = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(tile, tile) : document.createElement('canvas');
      canvas.width = canvas.height = tile;
      const ctx = canvas.getContext('2d', { willReadFrequently: true }) as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;
      for (const area of areas) {
        check();
        const pixels = new Uint8Array(area.w * area.h);
        while (index < tiles.length && tiles[index].area === area) {
          const { x, y } = tiles[index];
          const result = await decoded.get(index)!;
          decoded.delete(index);
          if ('error' in result) throw failure ?? result.error;
          const { bmp } = result;
          try {
            check();
            const w = Math.min(tile, area.w - x * tile), h = Math.min(tile, area.h - y * tile);
            if (bmp.width !== w || bmp.height !== h) throw new Error('Published tile dimensions do not match atlas.json.');
            ctx.clearRect(0, 0, tile, tile);
            ctx.drawImage(bmp, 0, 0);
            const rgba = ctx.getImageData(0, 0, w, h).data;
            for (let yy = 0; yy < h; yy++) {
              let dest = (y * tile + yy) * area.w + x * tile;
              const end = (yy + 1) * w * 4;
              for (let src = yy * w * 4; src < end; src += 4) pixels[dest++] = rgba[src];
            }
          } finally { bmp.close(); }
          index++;
          fetchAhead();
          decodeAhead();
        }
        check();
        const mosaic = await core.loadBundleArea(area.id, area.w, area.h, pixels);
        check();
        const raw = project.areas.find(a => a.id === area.id);
        if (!raw) throw new Error(`Published area ${area.id} is missing from the project.`);
        const rawDisplay = (project.grainatlas_ui as { tone?: Record<string, import('./core/types').ToneParams> })?.tone?.[area.id] ?? raw.params.tone;
        sources[area.id] = { rawCurve: structuredClone(raw.params.tone), rawDisplay: structuredClone(rawDisplay), sha256: mosaic.sha256 };
        mosaics[area.id] = { ...app.stitched[area.id], mosaic };
      }
      // Canvas edits made during the download must reach every area, including
      // areas assembled early. Do this after all pixels arrive, before re-toning.
      for (const area of areas) {
        check();
        await core.setGrains(area.id, JSON.parse(JSON.stringify(app.grains[area.id] ?? [])));
        check();
        const overlay = overlayFor(project, area.id, sources[area.id].sha256);
        if (!isNeutral(overlay.curve)) mosaics[area.id].mosaic = await core.retone(area.id, overlay.curve);
      }
      check();
      await core.setPackageDigest(remote.atlas.package.content_sha256, remote.atlas.package.content_files);
      check();
      for (const [id, source] of Object.entries(sources)) {
        const overlay = overlayFor(project, id, source.sha256);
        app.params[id] = { ...app.params[id], tone: overlay.curve };
        app.tone[id] = overlay.display;
      }
      // The only readiness switch: never expose a partially loaded pixel bundle.
      app.stitched = mosaics;
      app.bundle = sources;
      app.remoteTiles = null;
      // remoteTiles is not reactive. Refresh the remote reference so an already
      // mounted Canvas also switches its tile provider to the completed core.
      app.remote = { ...app.remote! };
      app.liveKeys = {};
      app.pixelsVersion++;
      app.pixelsLoad = null;
      return true;
    } catch (error) {
      if (sameProject() && app.pixelsLoad) {
        const reason = failure ?? error;
        app.pixelsLoad.error = reason instanceof Error && reason.name !== 'AbortError' ? reason.message : 'Stopped.';
      }
      return false;
    } finally {
      controller.abort();
      await Promise.all([
        ...ahead.values(),
        ...Array.from(decoded.values(), async pending => {
          const result = await pending;
          if ('bmp' in result) result.bmp.close();
        }),
      ]);
    }
  })().finally(() => {
    loading.delete(app);
    if (app.cancelPixelsLoad === cancel) app.cancelPixelsLoad = null;
  });
  loading.set(app, { remote, promise, cancel });
  return promise;
}
