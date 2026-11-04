// Actions shared by the screens: open, stitch, tone, project save/load.

import type { PackageFile } from './core/core';
import type { AreaParams, Grain, ImageEntry, LooseSpec, OpenResult, PackageInfo, Project, ToneParams } from './core/types';
import { app } from './store.svelte';
import { download } from './files';
import { packageNameFor, parseDeepLink, sameId, savedProject } from './deeplink';

/** Web: a package folder, an area folder or loose images, handed over as files. */
export async function openFiles(name: string, files: PackageFile[]) {
  const core = await app.core;
  const r = await app.run(`Reading ${name}`, (p) => core.openFiles(name, files, p));
  if (r) await opened(r);
}

/** Desktop: one folder (package, area folder, `.grainatlas` project) or a set of files, by path. */
export async function openPaths(paths: string[]) {
  const core = await app.core;
  if (paths.length === 1 && /\.grainatlas\/?$/.test(paths[0])) {
    const r = await app.run(`Opening ${paths[0]}`, () => core.openProject(paths[0]));
    if (!r) return;
    await adoptPackage(r.info, r.project);
    return;
  }
  const label = paths.length === 1 ? paths[0] : `${paths.length} files`;
  const r = await app.run(`Reading ${label}`, () => core.openPaths(paths));
  if (r) await opened(r);
}

const SCALE_KEY = 'grainatlas.looseScale';

/** The last scale typed for loose images in this session (the dialog's default). */
export function rememberedScale(): { mode: 'um_per_px' | 'field_width'; um_per_px: number; field_width_um: number } | null {
  try {
    const s = sessionStorage.getItem(SCALE_KEY);
    return s ? JSON.parse(s) : null;
  } catch {
    return null;
  }
}

function sameFiles(a: string[] | undefined, images: ImageEntry[]) {
  if (!a?.length) return false;
  const b = images.map((i) => i.file);
  return a.length === b.length && [...a].sort().every((f, i) => f === [...b].sort()[i]);
}

async function opened(r: OpenResult) {
  if (r.status === 'opened') {
    await adoptPackage(r.info);
    return;
  }
  // Images without calibration. A project saved for this same set knows its scale.
  const saved = savedProject(r.name);
  const spec = saved?.package.kind === 'loose' ? saved.package.loose : undefined;
  if (spec && sameFiles(spec.files, r.images)) {
    await openLoose(spec);
    if (app.pkg) app.notice = `Reopened ${r.images.length} images with the project's scale (${scaleText(spec)}).`;
    return;
  }
  app.pendingLoose = { name: r.name, images: r.images, spec: null };
}

export function scaleText(spec: LooseSpec) {
  return spec.field_width_um ? `field width ${spec.field_width_um} µm` : `${spec.um_per_px} µm per pixel`;
}

/** Opens the pending images with the scale (and optional grid) the user gave. */
export async function openLoose(spec: LooseSpec) {
  const core = await app.core;
  app.pendingLoose = null;
  try {
    sessionStorage.setItem(SCALE_KEY, JSON.stringify({ mode: spec.field_width_um ? 'field_width' : 'um_per_px', um_per_px: spec.um_per_px, field_width_um: spec.field_width_um ?? 0 }));
  } catch {
    /* storage may be unavailable */
  }
  const info = await app.run('Opening images', () => core.openLoose(spec));
  if (info) await adoptPackage(info);
}

/** Loose images: ask for the scale again (re-opens the same files; stitched areas are dropped). */
export function changeScale() {
  const p = app.pkg;
  if (!p || p.kind !== 'loose') return;
  const images = p.areas.flatMap((a) => a.tiles.map((t) => ({ file: t.file, w: t.w, h: t.h })));
  app.pendingLoose = { name: p.name, images, spec: (app.project?.package.loose as LooseSpec | undefined) ?? null };
}

/** A saved project fits an opened package when it was opened the same way, with the same areas. */
function fits(saved: Project, info: PackageInfo) {
  if ((saved.package.kind ?? 'package') !== info.kind) return false;
  if (info.kind !== 'loose') return true;
  const ids = info.areas.map((a) => a.id);
  return saved.areas.length === ids.length && saved.areas.every((a, i) => a.id === ids[i]);
}

async function adoptPackage(info: PackageInfo, fromFolder: Project | null = null) {
  const core = await app.core;
  app.remote = null;
  app.remoteTiles = null;
  app.pkg = info;
  app.params = {};
  app.stitched = {};
  app.grains = {};
  app.spots = [];
  app.labels = [];
  app.selection = [];
  app.tone = {};
  // A project saved earlier for this package (browser storage) is picked up again.
  let saved: Project | null = fromFolder ?? savedProject(info.name);
  if (saved && !fromFolder && !fits(saved, info)) saved = null;
  if (saved) {
    try {
      await core.loadProject(JSON.stringify(stripUi(saved)));
    } catch {
      saved = null;
    }
  }
  app.project = JSON.parse(await core.projectJson());
  if (saved) {
    app.adopt(saved);
    app.settings = { ...app.settings, ...(saved.settings ?? {}) };
  }
  fillParams();
  app.areaId = info.areas[0]?.id ?? null;
  app.screen = 'area';
  await resolveDeepLink();
}

/** The example package bundled with the web build (the test fixture), or null when absent. */
async function exampleManifest(): Promise<{ name: string; files: string[] } | null> {
  try {
    const r = await fetch('./demo/manifest.json');
    return r.ok ? await r.json() : null;
  } catch {
    return null;
  }
}

export async function openExample() {
  const m = await exampleManifest();
  if (!m) {
    app.error = 'This build has no example package.';
    return;
  }
  const files = await Promise.all(m.files.map(async (path: string) => ({ path, file: await fetch(`./demo/${path}`).then((r) => r.blob()) })));
  await openFiles(m.name, files);
}

/**
 * Start-up: `?project=<id>&spot=<id>` (from UPb Studio, or typed); with `&server=<base>` the
 * project's published bundle opens read-only from that server instead (laicpmsdb's links). The query is
 * dropped from the address afterwards so a reload does not repeat it. The
 * browser keeps no pixels, so the package must be opened again: the bundled
 * example opens by itself, any other package waits for the user on the Open
 * screen, which names the project. Without the parameters nothing happens.
 */
export async function startDeepLink(search = location.search) {
  const link = parseDeepLink(search);
  if (!link) return;
  try {
    history.replaceState(null, '', location.pathname + location.hash);
  } catch {
    /* some file:// pages refuse */
  }
  if (link.server) {
    // A published bundle: everything comes from the server, no package needed.
    app.serverOverride = link.server;
    const { openFromServer } = await import('./remote');
    await openFromServer(link.server, link.project, link.spot, link.access ?? null, link.mount ?? null);
    return;
  }
  app.deepLink = { ...link, name: packageNameFor(link.project) };
  if (sameId(app.project?.uuid, link.project)) {
    await resolveDeepLink();
    return;
  }
  app.screen = 'open';
  const name = app.deepLink.name;
  if (name && (await exampleManifest())?.name === name) await openExample();
}

/** After a package opened: if a deep link waits for this project, select its spot and center the canvas on it. */
async function resolveDeepLink() {
  const link = app.deepLink;
  if (!link || !app.project) return;
  if (!sameId(app.project.uuid, link.project)) {
    // Stays pending: loading the project's saved project.json next resolves it.
    app.notice = `This package's project is ${app.project.uuid}, not the linked ${link.project}: load its project.json on the Open screen to continue.`;
    return;
  }
  app.deepLink = null;
  if (!link.spot) return;
  const spot = app.spots.find((s) => sameId(s.id, link.spot));
  if (!spot) {
    app.notice = `Spot ${link.spot} is not in project ${app.project.name || app.project.uuid}.`;
    return;
  }
  if (!app.stitched[spot.area_id] && !(await stitch(spot.area_id))) return;
  app.areaId = spot.area_id;
  app.selection = [spot.id];
  app.focusSpot = spot.id;
  app.screen = 'canvas';
}

export async function loadProjectFile(file: File) {
  const text = await file.text();
  const p = JSON.parse(text) as Project;
  const core = await app.core;
  await core.loadProject(JSON.stringify(stripUi(p)));
  app.project = JSON.parse(await core.projectJson());
  app.adopt(p);
  app.settings = { ...app.settings, ...(p.settings ?? {}) };
  fillParams();
  await resolveDeepLink();
}

/** UI-held parameters from older saves lack newer fields: fill them from the core's record. */
function fillParams() {
  for (const a of app.project?.areas ?? []) {
    const base = a.params;
    const p = app.params[a.id] ?? base;
    const t = app.tone[a.id] ?? p.tone ?? base.tone;
    const tone = { ...base.tone, ...t, grain_norm: { ...base.tone.grain_norm, ...(t.grain_norm ?? {}) } };
    app.params[a.id] = { ...base, ...p, tone };
    app.tone[a.id] = tone;
  }
}

function stripUi(p: Project): Project {
  const { grainatlas_ui: _ui, ...rest } = p as Project & { grainatlas_ui?: unknown };
  return rest as Project;
}

export async function stitch(areaId: string) {
  const core = await app.core;
  const params = $snapshotParams(areaId);
  await syncGrains(areaId);
  const r = await app.run(`Stitching ${areaId}`, (p) => core.stitch(areaId, params, p));
  if (!r) return false;
  const l = live[areaId];
  if (l) {
    clearTimeout(l.timer);
    l.settled = l.seq;
  }
  app.liveBusy = { ...app.liveBusy, [areaId]: false };
  // The project first: it records the parameters this stitch used (App.svelte compares them).
  app.project = JSON.parse(await core.projectJson());
  app.stitched = { ...app.stitched, [areaId]: r };
  app.preview = { ...app.preview, [areaId]: null };
  app.pixelsVersion++;
  app.persist();
  return true;
}

export async function stitchAll() {
  for (const a of app.pkg?.areas ?? []) {
    if (!(await stitch(a.id))) return;
  }
}

function $snapshotParams(areaId: string): AreaParams {
  const p = JSON.parse(JSON.stringify(app.params[areaId])) as AreaParams;
  p.tone = JSON.parse(JSON.stringify(app.tone[areaId] ?? p.tone));
  return p;
}

// ---- live rendering --------------------------------------------------------
// Every photometric control (correction, background leveling, blending, stretch,
// CLAHE, per-tile gain and offset) re-renders a stitched area promptly: a
// low-resolution preview of the whole mosaic while the control moves, and a
// full-resolution recompute from the solved placement once it settles.
// Black / white / gamma need neither: they are the display LUT.

const PREVIEW_PIXELS = 1.5e6;
const live: Record<string, { seq: number; settled: number; timer: number; previewing: boolean; again: boolean; settling: Promise<void> | null }> = {};

/** The parameters that change pixels after the stitch (black / white / gamma excluded), and the
 * grains when per-grain normalisation is on. */
export function liveKey(p: AreaParams | undefined, t: ToneParams | undefined, grains?: Grain[]): string {
  if (!p) return '';
  const { black: _b, white: _w, gamma: _g, ...tone } = (t ?? p.tone) as ToneParams;
  return JSON.stringify([p.photometric, p.blend, tone, tone.grain_norm?.enabled ? grainsKey(grains) : null]);
}

function grainsKey(grains: Grain[] | undefined) {
  return (grains ?? []).map((g) => `${g.id}:${g.polygon_um.length}:${g.bbox_um.map((v) => v.toFixed(1)).join(',')}`).join(';');
}

const syncedGrains: Record<string, string> = {};

/** Hands the area's current grains to the core when per-grain normalisation needs them. */
async function syncGrains(areaId: string) {
  const grains = app.grains[areaId] ?? [];
  const key = grainsKey(grains);
  if (syncedGrains[areaId] === key) return;
  const core = await app.core;
  await core.setGrains(areaId, JSON.parse(JSON.stringify(grains)) as Grain[]);
  syncedGrains[areaId] = key;
}

/** Call after a photometric parameter of a stitched area changed. */
export function liveChanged(areaId: string, settleMs = 450) {
  if (!app.stitched[areaId]) return;
  const l = (live[areaId] ??= { seq: 0, settled: 0, timer: 0, previewing: false, again: false, settling: null });
  l.seq++;
  if (!app.liveBusy[areaId]) app.liveBusy = { ...app.liveBusy, [areaId]: true };
  void livePreview(areaId);
  clearTimeout(l.timer);
  l.timer = window.setTimeout(() => void settle(areaId), settleMs);
}

function previewLevel(areaId: string) {
  const levels = app.stitched[areaId].mosaic.levels;
  let L = Math.min(Math.max(0, app.drawLevel), levels.length - 1);
  while (L < levels.length - 1 && levels[L].w * levels[L].h > PREVIEW_PIXELS) L++;
  return L;
}

async function livePreview(areaId: string) {
  const l = live[areaId];
  if (l.previewing) {
    l.again = true;
    return;
  }
  l.previewing = true;
  const core = await app.core;
  try {
    do {
      l.again = false;
      const seq = l.seq;
      const level = previewLevel(areaId);
      await syncGrains(areaId);
      const img = await core.preview(areaId, $snapshotParams(areaId), level);
      if (seq > l.settled && img) app.preview = { ...app.preview, [areaId]: { level, img, over: app.pixelsVersion } };
    } while (l.again);
  } catch {
    /* a preview is best effort; the settle reports errors */
  } finally {
    l.previewing = false;
  }
}

async function settle(areaId: string) {
  const l = live[areaId];
  if (l.settling) await l.settling;
  const seq = l.seq;
  if (seq <= l.settled) return;
  const run = (async () => {
    const core = await app.core;
    app.rendering++;
    try {
      await syncGrains(areaId);
      const r = await core.recompute(areaId, $snapshotParams(areaId));
      l.settled = Math.max(l.settled, seq);
      app.project = JSON.parse(await core.projectJson());
      app.stitched = { ...app.stitched, [areaId]: r };
      app.pixelsVersion++;
      app.persist();
    } catch (e) {
      app.error = e instanceof Error ? e.message : String(e);
      l.settled = Math.max(l.settled, seq);
    } finally {
      app.rendering--;
    }
  })();
  l.settling = run;
  await run;
  if (l.settling === run) l.settling = null;
  if (l.seq <= l.settled && !l.settling) app.liveBusy = { ...app.liveBusy, [areaId]: false };
}

/** Waits until no live re-render of `areaId` is pending (tests, exports). */
export async function liveIdle(areaId: string) {
  const l = live[areaId];
  while (l && (l.settling || l.seq > l.settled)) {
    if (l.settling) await l.settling;
    else await new Promise((r) => setTimeout(r, 50));
  }
}

// ---- one-click Auto, Before / After, Reset ------------------------------------

/** Auto: background leveling on, stretch and CLAHE from the histogram. Remembers what was there
 * for Before / After; every manual control stays editable. */
export async function autoTone(areaId: string) {
  const core = await app.core;
  if (!app.stitched[areaId]) return;
  const before = $snapshotParams(areaId);
  const auto = await app.run('Auto tone', () => core.autoParams(areaId, before));
  if (!auto) return;
  app.compare = { ...app.compare, [areaId]: { other: before, showing: 'after' } };
  applyParams(areaId, auto);
}

/** Swaps between the parameters before Auto and the current ones. */
export function toggleBeforeAfter(areaId: string) {
  const c = app.compare[areaId];
  if (!c) return;
  const now = $snapshotParams(areaId);
  app.compare = { ...app.compare, [areaId]: { other: now, showing: c.showing === 'after' ? 'before' : 'after' } };
  applyParams(areaId, c.other);
}

/** Back to the defaults for correction, blending and tone (stitch and segmentation are kept). */
export async function resetTone(areaId: string) {
  const core = await app.core;
  const d = await core.defaultParams();
  const cur = $snapshotParams(areaId);
  app.compare = { ...app.compare, [areaId]: null };
  applyParams(areaId, { ...cur, photometric: d.photometric, blend: d.blend, tone: d.tone });
}

function applyParams(areaId: string, p: AreaParams) {
  const cur = app.params[areaId];
  app.params = { ...app.params, [areaId]: { ...cur, photometric: p.photometric, blend: p.blend, tone: p.tone } };
  app.tone = { ...app.tone, [areaId]: p.tone };
}

export async function saveProject() {
  const p = app.fullProject();
  if (!p) return;
  const core = await app.core;
  if (core.kind === 'tauri') {
    const where = await app.run('Saving project', () => core.saveProject(p, {}));
    if (where) app.notice = `Saved ${where}`;
    return where;
  }
  download(JSON.stringify(p, null, 2), `${p.package.name}.grainatlas.project.json`, 'application/json');
  return null;
}

export async function exportPng(areaId: string) {
  const core = await app.core;
  const tone = app.tone[areaId];
  const png = await app.run('Exporting PNG', () => core.exportPng(areaId, tone));
  if (png) download(png, `${areaId}_mosaic.png`, 'image/png');
}

/** Finds grains in every stitched area (keeps ids and numbers of grains found before). */
export async function findAllGrains() {
  const core = await app.core;
  for (const id of Object.keys(app.stitched)) {
    const params = JSON.parse(JSON.stringify(app.params[id].segment));
    const prev = JSON.parse(JSON.stringify(app.grains[id] ?? []));
    const r = await app.run(`Finding grains in ${id}`, () => core.segment(id, params, prev));
    if (!r) return;
    app.grains = { ...app.grains, [id]: r };
  }
  app.persist();
}
