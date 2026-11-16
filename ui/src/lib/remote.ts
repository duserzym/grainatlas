import { bundlePixels, isNeutral, overlayFor, type BundleSources } from './bundle-tone';
import { canWrite } from './presentation';
// Open from server: a published atlas bundle read from `<server>/atlas/projects/<id>/`
// (WORKFLOW.md 3.1). Nothing is stitched: the canvas draws the bundle's WebP tiles, and the
// grains, spots, labels and scale bars come from its project.json. Read-only until "Edit a
// copy" makes it a local project (annotations, tone and print sheets editable). This is how UPbDB's mount page embeds grainatlas and how a spot link from
// anywhere reaches the images (`?server=&project=&spot=`).

import { isBridgeCommand } from '@lab/bridge';
import { hostedSync, ATLAS_EVENTS } from './hosted-sync';
import { draftElements, listDrafts, storedDraftProject } from './drafts';
import { groupedGrains, uiSpots } from './sync';
import type { RawImage } from './core/core';
import type { LevelInfo, MosaicJson, Project, StitchSummary } from './core/types';
import { atlasUrl, normalizeServer } from './upbdb';
import { current, type AppState } from './store.svelte';
import { savedProject, sameId } from './deeplink';
import { authToken } from './atlas';

export interface AtlasArea {
  id: string;
  uuid: string;
  name: string;
  sample: string;
  path: string;
  w: number;
  h: number;
  um_per_px: number;
  levels: { z: number; w: number; h: number; cols: number; rows: number; scale: number }[];
  mosaic_sha256?: string | null;
}

export interface AtlasJson {
  format: string;
  version: number;
  project_id: string;
  name: string;
  mount_id: string | null;
  opened_from: string;
  tile_size: number;
  tile_format: string;
  tile_encoding: string;
  areas: AtlasArea[];
  package: { name: string; content_sha256: string; content_files: number; nas_path: string | null; path: string };
  grainatlas_version: string;
  exported: string;
  files: { path: string; size: number; sha256: string }[];
}

export interface Remote {
  server: string;
  projectId: string;
  atlas: AtlasJson;
  readOnly: boolean;
  /** UPbDB serves bundles to a bearer token or a signed read-only `access` token (12 h). */
  token: string | null;
  access: string | null;
  /** The mount the server says the project is published on (`?mount=`), when given. */
  mount: string | null;
}

/** URL and headers for one file of the remote bundle. */
function fileRequest(r: Pick<Remote, 'server' | 'projectId' | 'token' | 'access'>, path: string): [string, RequestInit] {
  const url = atlasUrl(r.server, r.projectId, path) + (r.access ? `?access=${encodeURIComponent(r.access)}` : '');
  return [url, { cache: 'no-cache', headers: r.token ? { Authorization: `Bearer ${r.token}` } : {} }];
}

/** The pyramid tiles of a published bundle, decoded to grey like the core's tiles. */
export class RemoteTiles {
  private canvas: OffscreenCanvas | HTMLCanvasElement | null = null;
  constructor(private r: Remote) {}

  async pyramidTile(areaId: string, level: number, col: number, row: number): Promise<RawImage> {
    const area = this.r.atlas.areas.find((a) => a.id === areaId);
    if (!area) throw new Error(`no area ${areaId}`);
    const res = await fetch(...fileRequest(this.r, `${area.path}/${level}/${col}_${row}.webp`));
    if (!res.ok) throw new Error(`tile ${level}/${col}_${row}: ${res.status}`);
    const bmp = await createImageBitmap(await res.blob());
    const { width: w, height: h } = bmp;
    if (!this.canvas) this.canvas = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : document.createElement('canvas');
    this.canvas.width = w;
    this.canvas.height = h;
    const ctx = this.canvas.getContext('2d', { willReadFrequently: true }) as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;
    ctx.drawImage(bmp, 0, 0);
    bmp.close();
    const rgba = ctx.getImageData(0, 0, w, h).data;
    const data = new Uint8Array(w * h);
    for (let i = 0, j = 0; i < data.length; i++, j += 4) data[i] = rgba[j];
    return { w, h, data };
  }
}

async function getJson<T>([url, init]: [string, RequestInit], what: string): Promise<T> {
  let r: Response;
  try {
    r = await fetch(url, init);
  } catch (e) {
    throw new Error(`${what}: cannot reach ${url.split('?')[0]} (${e instanceof Error ? e.message : e})`);
  }
  const hint = r.status === 401 ? ' (sign in, paste a token in Settings, or use a link with ?access=)' : r.status === 404 ? ' (not published, or not visible to you)' : '';
  if (!r.ok) throw new Error(`${what}: ${r.status} ${r.statusText}${hint} at ${url.split('?')[0]}`);
  return (await r.json()) as T;
}

function mosaicOf(a: AtlasArea, tile: number): MosaicJson {
  const levels: LevelInfo[] = a.levels.map((l) => ({ level: l.z, w: l.w, h: l.h, cols: l.cols, rows: l.rows, scale: l.scale }));
  return { w: a.w, h: a.h, um_per_px: a.um_per_px, origin_um: [0, 0], tile_size: tile, format: 'webp', levels, sha256: a.mosaic_sha256 ?? '' };
}

/** Loads a published bundle and shows it on the Canvas, read-only; selects `spot` when given. */
export async function openFromServer(serverIn: string, projectId: string, spot: string | null = null, access: string | null = null, mount: string | null = null, given?: AppState) {
  // Callers that pass a tab chose it (and may reload the project there). Without one
  // (Settings, links), the project's own tab, the empty active tab or a new tab.
  let app = given ?? current();
  if (!given) {
    const { targetTab } = await import('./actions');
    app = targetTab(projectId);
  }
  if (!given && sameId(app.project?.uuid, projectId)) {
    if (spot) {
      const { applyShellFocus } = await import('./shellcmd');
      await applyShellFocus(app, { type: 'focus-spot', project: projectId, spot, flash: false });
    }
    return true;
  }
  await app.sync.settled();
  const server = normalizeServer(serverIn);
  if (!server) {
    app.error = `Not a server address: ${serverIn}`;
    return false;
  }
  if (!isBridgeCommand({ type: 'open-project', project: projectId, server })) { app.error = 'Enter a valid project id and server address.'; return false; }
  hostedSync.status({ type: ATLAS_EVENTS.status, project: projectId, mount: null, name: projectId, mode: 'view', state: 'checking', revision: null, atlasVersion: null, changes: null, syncedAt: null }, true);
  const token = await authToken(app, server);
  const at = { server, projectId, token, access };
  const r = await app.run(`Opening ${projectId} from ${new URL(server).host}`, async () => {
    const atlas = await getJson<AtlasJson>(fileRequest(at, 'atlas.json'), 'atlas.json');
    if (atlas.format !== 'grainatlas-atlas') throw new Error('atlas.json: not a grainatlas atlas bundle');
    if (atlas.version > 1) throw new Error(`atlas.json: version ${atlas.version} is newer than this grainatlas reads (1)`);
    const project = await getJson<Project>(fileRequest(at, 'project.json'), 'project.json');
    // Published bundles are immutable; ones from before the UPbDB rename have no `upbdb`
    // link. The mount this viewer was opened for stands in until the sync check confirms it.
    project.upbdb ??= { mount_id: mount ?? null };
    // check() overlays current identified database elements (or a pending draft)
    // on this bundle; the bundle still supplies pixels and display metadata.
    return { atlas, project };
  });
  if (!r) return false;
  const remote: Remote = { server, projectId, atlas: r.atlas, readOnly: true, token, access, mount };
  app.clearHistory(); app.viewMode = false; app.draftHere = null;
  app.bundle = null;
  app.remote = remote;
  app.remoteTiles = new RemoteTiles(remote);
  app.pkg = null;
  app.params = {};
  app.grains = {};
  app.spots = [];
  app.labels = [];
  app.selection = [];
  app.overlays = {};
  app.project = r.project;
  app.adopt(r.project);
  // The tiles carry the display tone already.
  app.tone = {};
  app.stitched = Object.fromEntries(r.atlas.areas.map((a) => [a.id, { mosaic: mosaicOf(a, r.atlas.tile_size), placement: null, photometric: null, timings: [] } as unknown as StitchSummary]));
  app.preview = {};
  app.pixelsVersion++;
  app.areaId = r.atlas.areas[0]?.id ?? null;
  app.sync.openedView();
  await app.sync.check();
  app.screen = 'canvas';
  if (spot) {
    const s = app.spots.find((x) => sameId(x.id, spot));
    if (s) {
      app.areaId = s.area_id;
      app.selection = [s.id];
      app.focusSpot = s.id;
    } else app.notice = `Spot ${spot} is not in ${r.atlas.name}.`;
  }
  return true;
}

/**
 * Makes the opened bundle a local project: same project id (a re-publish becomes the next
 * version), kept in this browser like any project, editable. Area and Layout load the published
 * images into the core; only stitching and shading correction need the raw package.
 */
export async function editCopy(app: AppState = current()) {
  if (!canWrite()) return;
  const remote = app.remote;
  const p = app.project;
  if (!remote || !p) return;
  const draft = listDrafts(remote.server).find(d => d.uuid === p.uuid);
  const sameUuid = draft ? storedDraftProject(draft) : null;
  const packageCopy = savedProject(p.package.name);
  const existing = draft?.pending === null ? null : sameUuid ?? (draft ? packageCopy : null);
  if (packageCopy && !sameId(packageCopy.uuid, p.uuid) && !confirm(`This computer already has a project for ${p.package.name} (${packageCopy.uuid}). Replace it with this project from the server?`)) return;
  if (app.bundle) {
    app.remote = { ...remote, readOnly: false };
    await app.sync.editBase();
    return;
  }
  const core = await app.core;
  const ok = await app.run('Making a local copy', async () => {
    await core.blank(p.package.name);
    const { grainatlas_ui: _ui, ...record } = p as Project & { grainatlas_ui?: unknown };
    await core.loadProject(JSON.stringify(record));
    return true;
  });
  if (!ok) return;
  // Keep the published JSON numbers verbatim; the core's f32 serialization can spell them differently.
  app.project = p;
  app.adopt(p);
  app.remote = { ...remote, readOnly: false };
  if (existing && sameId(existing.uuid, p.uuid)) { app.project = existing; app.adopt(existing); }
  app.tone = {}; // Server tiles already carry the raw display tone.
  const pending = draft ? draftElements(draft) : null;
  if (pending) {
    app.spots = uiSpots(pending.spots, pending.grains, app.spots);
    app.grains = groupedGrains(pending.grains, app.grains); app.labels = pending.labels;
    app.draftHere = draft!.base_revision ?? 'unknown';
  }
  await app.sync.editBase();
  if (pending) app.draftHere = app.sync.draft?.base_revision ?? 'unknown';
  app.persist();
  app.notice = `Editing ${remote.atlas.name}. Spots, grains and labels can be changed. Tone and print sheets are in Area and Layout. Re-stitching needs the raw package.`;
}


// One load per tab, even when routing changes while fetches are in flight.
const loading = new WeakMap<AppState, Promise<boolean>>();
export function loadPublishedPixels(app: AppState): Promise<boolean> {
  if (bundlePixels(app)) return Promise.resolve(true);
  if (!app.remote || app.readOnly) return Promise.resolve(false);
  const pending = loading.get(app);
  if (pending) return pending;
  const remote = app.remote;
  const project = app.fullProject()!;
  const controller = new AbortController();
  app.cancelTask = () => controller.abort();
  const promise = (async () => {
    const core = await app.core;
    const result = await app.run('Loading published images', async () => {
      const sources: BundleSources = {};
      const mosaics: typeof app.stitched = {};
      const total = remote.atlas.areas.reduce((n, a) => n + Math.ceil(a.w / remote.atlas.tile_size) * Math.ceil(a.h / remote.atlas.tile_size), 0);
      let done = 0;
      // Areas are sequential: only one assembled byte image resides in JS at a time.
      for (const area of remote.atlas.areas) {
        if (app.remote?.projectId !== remote.projectId || app.readOnly) throw new Error('Published image loading stopped.');
        const pixels = new Uint8Array(area.w * area.h);
        const tile = remote.atlas.tile_size;
        const cols = Math.ceil(area.w / tile), rows = Math.ceil(area.h / tile);
        let next = 0;
        const jobs = await Promise.allSettled(Array.from({ length: Math.min(8, cols * rows) }, async () => {
          try {
            const canvas = new OffscreenCanvas(tile, tile);
            const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
            for (;;) {
              const index = next++;
              if (index >= cols * rows) return;
              const x = index % cols, y = Math.floor(index / cols);
              controller.signal.throwIfAborted();
              const [url, init] = fileRequest(remote, `${area.path}/0/${x}_${y}.webp`);
              const response = await fetch(url, { ...init, signal: controller.signal });
              if (!response.ok) throw new Error(`Published tile ${area.id}/${x}_${y}: ${response.status}`);
              const bmp = await createImageBitmap(await response.blob(), { colorSpaceConversion: 'none', premultiplyAlpha: 'none' });
              try {
                const w = Math.min(tile, area.w - x * tile), h = Math.min(tile, area.h - y * tile);
                if (bmp.width !== w || bmp.height !== h) throw new Error('Published tile dimensions do not match atlas.json.');
                ctx.clearRect(0, 0, tile, tile);
                ctx.drawImage(bmp, 0, 0);
                const rgba = ctx.getImageData(0, 0, w, h).data;
                for (let yy = 0; yy < h; yy++) for (let xx = 0; xx < w; xx++) pixels[(y * tile + yy) * area.w + x * tile + xx] = rgba[(yy * w + xx) * 4];
              } finally { bmp.close(); }
              done++;
              if (app.busy) app.busy.label = `Loading published images (${done} of ${total} tiles)`;
            }
          } catch (error) { controller.abort(); throw error; }
        }));
        const failed = jobs.find(r => r.status === 'rejected');
        if (failed?.status === 'rejected') throw failed.reason;
        controller.signal.throwIfAborted();
        const mosaic = await core.loadBundleArea(area.id, area.w, area.h, pixels);
        const raw = project.areas.find(a => a.id === area.id);
        if (!raw) throw new Error(`Published area ${area.id} is missing from the project.`);
        const rawDisplay = (project.grainatlas_ui as { tone?: Record<string, import('./core/types').ToneParams> })?.tone?.[area.id] ?? raw.params.tone;
        sources[area.id] = { rawCurve: structuredClone(raw.params.tone), rawDisplay: structuredClone(rawDisplay), sha256: mosaic.sha256 };
        const overlay = overlayFor(project, area.id, mosaic.sha256);
        await core.setGrains(area.id, JSON.parse(JSON.stringify(app.grains[area.id] ?? [])));
        const toned = isNeutral(overlay.curve) ? mosaic : await core.retone(area.id, overlay.curve);
        mosaics[area.id] = { ...app.stitched[area.id], mosaic: toned };
      }
      await core.setPackageDigest(remote.atlas.package.content_sha256, remote.atlas.package.content_files);
      controller.signal.throwIfAborted();
      if (app.remote?.projectId !== remote.projectId || app.readOnly) throw new Error('Published image loading stopped.');
      for (const [id, source] of Object.entries(sources)) {
        const overlay = overlayFor(project, id, source.sha256);
        app.params[id] = { ...app.params[id], tone: overlay.curve };
        app.tone[id] = overlay.display;
      }
      app.stitched = mosaics;
      app.bundle = sources;
      app.remoteTiles = null;
      app.liveKeys = {};
      app.pixelsVersion++;
      return true;
    });
    return !!result;
  })().finally(() => { loading.delete(app); app.cancelTask = null; });
  loading.set(app, promise);
  return promise;
}
