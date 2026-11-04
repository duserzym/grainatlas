// Open from server: a published atlas bundle read from `<server>/atlas/projects/<id>/`
// (WORKFLOW.md 3.1). Nothing is stitched: the canvas draws the bundle's WebP tiles, and the
// grains, spots, labels and scale bars come from its project.json. Read-only until "Edit a
// copy" makes it a local project (spots, grains and labels editable; new pixels need the raw
// package). This is how laicpmsdb's mount page embeds grainatlas and how a spot link from
// anywhere reaches the images (`?server=&project=&spot=`).

import type { RawImage } from './core/core';
import type { LevelInfo, MosaicJson, Project, StitchSummary } from './core/types';
import { atlasUrl, normalizeServer } from './laicpmsdb';
import { app } from './store.svelte';
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
  package: { name: string; content_sha256: string; nas_path: string | null; path: string };
  grainatlas_version: string;
  exported: string;
  files: { path: string; size: number; sha256: string }[];
}

export interface Remote {
  server: string;
  projectId: string;
  atlas: AtlasJson;
  readOnly: boolean;
  /** laicpmsdb serves bundles to a bearer token or a signed read-only `access` token (12 h). */
  token: string | null;
  access: string | null;
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
export async function openFromServer(serverIn: string, projectId: string, spot: string | null = null, access: string | null = null) {
  const server = normalizeServer(serverIn);
  if (!server) {
    app.error = `Not a server address: ${serverIn}`;
    return false;
  }
  const token = await authToken();
  const at = { server, projectId, token, access };
  const r = await app.run(`Opening ${projectId} from ${new URL(server).host}`, async () => {
    const atlas = await getJson<AtlasJson>(fileRequest(at, 'atlas.json'), 'atlas.json');
    if (atlas.format !== 'grainatlas-atlas') throw new Error('atlas.json: not a grainatlas atlas bundle');
    if (atlas.version > 1) throw new Error(`atlas.json: version ${atlas.version} is newer than this grainatlas reads (1)`);
    const project = await getJson<Project>(fileRequest(at, 'project.json'), 'project.json');
    return { atlas, project };
  });
  if (!r) return false;
  const remote: Remote = { server, projectId, atlas: r.atlas, readOnly: true, token, access };
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
 * version), kept in this browser like any project, editable. Pixels stay the server's until the
 * raw package is opened, which picks this project up by its package name.
 */
export async function editCopy() {
  const remote = app.remote;
  const p = app.project;
  if (!remote || !p) return;
  const existing = savedProject(p.package.name);
  if (existing && !confirm(`This computer already has a project for ${p.package.name}${sameId(existing.uuid, p.uuid) ? '' : ` (${existing.uuid})`}. Replace it with this copy from the server?`)) return;
  const core = await app.core;
  const ok = await app.run('Making a local copy', async () => {
    await core.blank(p.package.name);
    const { grainatlas_ui: _ui, ...record } = p as Project & { grainatlas_ui?: unknown };
    await core.loadProject(JSON.stringify(record));
    return true;
  });
  if (!ok) return;
  app.project = JSON.parse(await core.projectJson());
  app.remote = { ...remote, readOnly: false };
  app.persist();
  app.notice = `Editable copy of ${remote.atlas.name}. Spots, grains and labels can be changed; open the raw package (${p.package.name}) to re-stitch or publish new pixels.`;
}
