import { canWrite } from './presentation';
import { chooseOpenTab, existingProject, holdsWork, pathGroups, sourceKey } from './open-target';
// Actions shared by the screens: open, stitch, tone, project save/load.

import { beforeCheckedAction } from './checks';
import { findGrainsInArea, findGrainsInAreas } from './grain-finding';

import type { PackageFile } from './core/core';
import type { AreaParams, Grain, ImageEntry, LooseSpec, OpenResult, PackageInfo, Project, ToneParams } from './core/types';
import { bundlePixels, curveTone, neutralTone } from './bundle-tone';
import { current, shared, tabs, type AppState, type LiveState } from './store.svelte';
import { isHosted, notify, type OpenedFiles } from '@lab/bridge';
import { download, fromShell } from './files';
import { packageNameFor, parseDeepLink, readRecent, sameId, savedProject } from './deeplink';
import { clearStoredCopy, newerStoredCopy, storedCopyDate } from './stored-copy';
import { keepPrintEdits } from './strip-edit';

/** Where something newly opened goes: the active tab when nothing is open there, else a new tab
 * (opening never replaces the project of a tab). */
export function targetTab(uuid?: string, active = current()): AppState {
  const tab = chooseOpenTab(tabs.list, active, () => tabs.add(), uuid);
  tabs.select(tab.id);
  return tab;
}

/** Same project or the same files already open: show that tab. `spare`, the tab this open had
 * claimed, is dropped when nothing else is in it. */
function selectExisting(tab: AppState, spare?: AppState): AppState {
  if (spare && spare !== tab && !spare.pkg && !spare.project && !spare.remote && !spare.pendingLoose) {
    spare.packageSource = null;
    tabs.remove(spare.id);
  }
  // From that tab's own Open screen, show the project again.
  if (tab.screen === 'open' && (tab.pkg || tab.remote)) tab.screen = tab.pkg ? 'area' : 'canvas';
  tabs.select(tab.id);
  return tab;
}

/**
 * Desktop: command-line, dropped or OS-opened paths (Finder, `open -b`, a second launch). Folders
 * (packages, area folders, image folders, `.grainatlas` projects) and `.gaproj` files open one tab
 * each; image files open together in one. Each claims its tab before the first await, so opens
 * that arrive together never share a tab. Returns the tabs opened or switched to.
 */
export async function openPathsInTabs(paths: string[]): Promise<AppState[]> {
  const opened = await Promise.all(pathGroups(paths).map((g) => openPaths(g)));
  return opened.filter((t, i) => opened.indexOf(t) === i);
}

/** A saved project: a `.grainatlas/` folder or a `.gaproj` file. */
const PROJECT_PATH = /\.(grainatlas\/?|gaproj)$/i;

/** Web: a package folder, an area folder or loose images, handed over as files. */
export async function openFiles(name: string, files: PackageFile[], app: AppState = current()) {
  const pending = shared.pendingProjects.find(p => p.project.package.name === name);
  const uuid = pending?.project.uuid ?? savedProject(name)?.uuid;
  const existing = (uuid ? existingProject(tabs.list, uuid) : undefined) ?? tabs.list.find(t => t.pkg?.name === name);
  // Another tab has it: show that tab. This tab opening its own files again reloads them here.
  if (existing && existing !== app) return selectExisting(existing);
  if (existing !== app) app = targetTab(undefined, app);
  app.packageSource = { name, files };
  const core = await app.core;
  const r = await app.run(`Reading ${name}`, (p) => core.openFiles(name, files, p));
  if (r) await opened(app, r);
  else app.packageSource = null;
  return app;
}

/**
 * Hosted: files the user dropped on UPb Lab's window or opened through the OS
 * (bridge `onOpenFiles`, ux-conventions rule 4). They open exactly like a drop on
 * the Open screen; `kind` is only the shell's hint, the core still decides. With
 * a project open, a new tab keeps the earlier project intact.
 */
export async function openOffered(set: OpenedFiles) {
  const app = current();
  const told = (outcome: string) => void notify({ type: 'grainatlas.files.opened', kind: set.kind, name: set.name, files: set.files.length, outcome });
  if (set.kind === 'project') {
    if (!isHosted()) return;
    try {
      const file = set.files.find((f) => f.path.split('/').pop() === 'project.json')?.file;
      if (!file) throw new Error('The project folder has no project.json.');
      const project = JSON.parse(await file.text()) as Project;
      if (!project.uuid || typeof project.uuid !== 'string' || !project.package?.name || typeof project.package.name !== 'string' || !Array.isArray(project.areas)) throw new Error('project.json is missing its project id, package name or areas.');
      const existing = existingProject(tabs.list, project.uuid);
      if (existing) { selectExisting(existing); told('selected existing project'); return; }
      const pending = { file, project, name: set.name, files: set.files.length };
      shared.pendingProjects = [...shared.pendingProjects.filter((p) => p.project.package.name !== project.package.name), pending];
      const tab = tabs.list.find((t) => t.pkg?.name === project.package.name);
      if (tab) {
        tabs.select(tab.id);
        await applyPendingProject(tab);
      } else {
        app.screen = 'open';
        told(`waiting: package ${project.package.name}`);
      }
    } catch (e) {
      app.error = String(e);
      told(`failed: ${app.error}`);
    }
    return;
  }
  const s = fromShell(set);
  if (!s.files.length) {
    app.error = `Nothing to open in ${set.name}: drop a package folder, an area folder or image files.`;
    told('refused: nothing to open');
    return;
  }
  const openedTab = await openFiles(s.name, s.files);
  // Read again: the open just changed it (TypeScript still has it narrowed to null).
  const pkg = openedTab.pkg as PackageInfo | null;
  told(openedTab.pendingLoose ? 'asked: scale' : pkg ? `opened: ${pkg.kind}, ${pkg.areas.length} area(s)` : `failed: ${openedTab.error ?? 'not opened'}`);
}

export function pendingProjectNotice(project: Project): string {
  return `Open the SEM package ${project.package.name} to continue (${project.package.path})`;
}

/** Package picks, drops and shell offers all meet here after opening. */
async function applyPendingProject(app: AppState, initial = false): Promise<boolean> {
  if (!isHosted()) return false;
  const pending = shared.pendingProjects.find((p) => p.project.package.name === app.pkg?.name);
  if (!pending) return false;
  if (app.notice === pendingProjectNotice(pending.project)) app.notice = null;
  try {
    await loadProjectFile(pending.file, app, initial);
    shared.pendingProjects = shared.pendingProjects.filter((p) => p !== pending);
    void notify({ type: 'grainatlas.files.opened', kind: 'project', name: pending.name, files: pending.files, outcome: `opened: project ${pending.project.package.name}` });
  } catch (e) {
    app.error = String(e);
    void notify({ type: 'grainatlas.files.opened', kind: 'project', name: pending.name, files: pending.files, outcome: `failed: ${app.error}` });
  }
  return true;
}

/** Legacy offers still route through the same non-replacing open actions. */
export async function answerOffer(open: boolean) {
  const app = current();
  const s = app.offer;
  app.offer = null;
  if (!s) return;
  void notify({ type: 'grainatlas.files.opened', name: s.name, files: s.files.length, outcome: open ? 'opened' : 'kept the open project' });
  if (s.projectFile) {
    const pending = shared.pendingProjects.find((p) => p.file === s.projectFile);
    if (open) {
      try {
        await loadProjectFile(s.projectFile, app);
        void notify({ type: 'grainatlas.files.opened', kind: 'project', name: s.name, files: pending?.files ?? 1, outcome: `opened: project ${app.pkg?.name}` });
      } catch (e) { app.error = String(e); return; }
    }
    shared.pendingProjects = shared.pendingProjects.filter((p) => p !== pending);
  } else if (open) await openFiles(s.name, s.files, app);
}

/** Desktop: one folder (package, area folder, `.grainatlas` project) or a set of files, by path. */
export async function openPaths(paths: string[], app: AppState = current()): Promise<AppState> {
  const key = sourceKey(paths);
  const existing = tabs.list.find(t => t.packageSource && 'paths' in t.packageSource && sourceKey(t.packageSource.paths) === key);
  if (existing && existing !== app) return selectExisting(existing);
  if (existing !== app) app = targetTab(undefined, app);
  app.packageSource = { paths };
  const core = await app.core;
  if (paths.length === 1 && PROJECT_PATH.test(paths[0])) {
    const r = await app.run(`Opening ${paths[0]}`, () => core.openProject(paths[0]));
    if (!r) { app.packageSource = null; return app; }
    const existing = existingProject(tabs.list, r.project.uuid);
    if (existing && existing !== app) return selectExisting(existing, app);
    const path = { kind: /\.gaproj$/i.test(paths[0]) ? 'file' as const : 'folder' as const, path: paths[0].replace(/\/$/, '') };
    app.loadingProject = true;
    app.clearHistory(); app.viewMode = false; app.draftHere = null;
    try {
      await adoptPackage(app, r.info, r.project, path, r.modified_ms);
      await restoreAreas(app);
      if (app.storedCopy) app.storedCopy = { ...app.storedCopy, ready: true };
    } finally { app.loadingProject = false; }
    await app.sync.check();
    return app;
  }
  const label = paths.length === 1 ? paths[0] : `${paths.length} files`;
  const r = await app.run(`Reading ${label}`, () => core.openPaths(paths));
  if (r) {
    const saved = r.status === 'opened' ? savedProject(r.info.name) : savedProject(r.name);
    const existing = saved && existingProject(tabs.list, saved.uuid);
    if (existing && existing !== app) return selectExisting(existing, app);
    await opened(app, r);
  } else app.packageSource = null;
  return app;
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

async function opened(app: AppState, r: OpenResult) {
  if (r.status === 'opened') {
    await adoptPackage(app, r.info);
    return;
  }
  // Images without calibration. A project saved for this same set knows its scale.
  const saved = savedProject(r.name);
  const spec = saved?.package.kind === 'loose' ? saved.package.loose : undefined;
  if (spec && sameFiles(spec.files, r.images)) {
    await openLoose(spec, app);
    if (app.pkg) app.notice = `${app.notice ? `${app.notice} ` : ''}Reopened ${r.images.length} images with the project's scale (${scaleText(spec)}).`;
    return;
  }
  app.pendingLoose = { name: r.name, images: r.images, spec: null };
}

export function scaleText(spec: LooseSpec) {
  return spec.field_width_um ? `field width ${spec.field_width_um} µm` : `${spec.um_per_px} µm per pixel`;
}

/** Opens the pending images with the scale (and optional grid) the user gave. */
export async function openLoose(spec: LooseSpec, app: AppState = current()) {
  const core = await app.core;
  app.pendingLoose = null;
  try {
    if (canWrite()) sessionStorage.setItem(SCALE_KEY, JSON.stringify({ mode: spec.field_width_um ? 'field_width' : 'um_per_px', um_per_px: spec.um_per_px, field_width_um: spec.field_width_um ?? 0 }));
  } catch {
    /* storage may be unavailable */
  }
  const info = await app.run('Opening images', () => core.openLoose(spec));
  if (info) await adoptPackage(app, info);
}

/** Loose images: ask for the scale again (re-opens the same files; stitched areas are dropped). */
export function changeScale() {
  const app = current();
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

async function adoptPackage(app: AppState, info: PackageInfo, fromFolder: Project | null = null, path: AppState['projectPath'] = null, modifiedMs = NaN) {
  const core = await app.core;
  const stored = savedProject(info.name);
  // Set the guard before changing any UI state: effects and rendering can call persist.
  app.storedCopy = fromFolder && path ? newerStoredCopy(fromFolder, path.path, modifiedMs) : null;
  app.notice = null;
  app.projectPath = path;
  app.bundle = null;
  app.remote = null;
  app.remoteTiles = null;
  app.pkg = info;
  app.params = {};
  app.stitched = {};
  app.grains = {};
  app.spots = [];
  app.findings = [];
  app.checkLabels = null;
  app.checkLabelsError = null;
  app.mountLabelsCount = null;
  app.checksRan = false;
  app.settings.area_patterns = {};
  app.labels = [];
  app.selection = [];
  app.tone = {};
  // A project saved earlier for this package (browser storage) is picked up again.
  let saved: Project | null = fromFolder ?? stored;
  if (saved && !fromFolder && !fits(saved, info)) saved = null;
  if (saved) {
    try {
      await core.loadProject(JSON.stringify(stripUi(saved)));
    } catch {
      saved = null;
      app.storedCopy = null;
    }
  }
  app.project = JSON.parse(await core.projectJson());
  if (saved) {
    app.adopt(saved);
    app.settings = { ...app.settings, ...(saved.settings ?? {}), area_patterns: app.project!.settings.area_patterns ?? {}, spot_cursor_colour: app.project!.settings.spot_cursor_colour, outline_margin_um: app.project!.settings.outline_margin_um, outline_smooth: app.project!.settings.outline_smooth };
    if (!fromFolder) {
      const date = storedCopyDate(readRecent().find((r) => r.name === info.name && !r.path)?.when);
      app.notice = `Opened the copy of ${info.name} kept in this app${date ? ` (saved ${date})` : ''}. A .grainatlas project folder is not read from here: open that folder to use what is saved in it.`;
    }
  }
  fillParams(app);
  app.areaId = info.areas[0]?.id ?? null;
  app.screen = 'area';
  app.dirty = false;
  if (!isHosted() || !(await applyPendingProject(app, true))) await resolveDeepLink(app);
  await app.sync.check();
}

/**
 * Start-up: `?project=<id>&spot=<id>` (from UPb Lab, or typed); with `&server=<base>` the
 * project's published bundle opens read-only from that server instead (UPbDB's links). The query is
 * dropped from the address afterwards so a reload does not repeat it. The
 * browser keeps no pixels, so the package must be opened again: it waits for
 * the user on the Open screen, which names the project. Without the parameters nothing happens.
 */
export async function startDeepLink(search = location.search) {
  const link = parseDeepLink(search);
  if (!link) return;
  const app = targetTab(link.project);
  try {
    history.replaceState(null, '', location.pathname + location.hash);
  } catch {
    /* some file:// pages refuse */
  }
  if (link.server) {
    // A published bundle: everything comes from the server, no package needed.
    app.serverOverride = link.server;
    const { openFromServer } = await import('./remote');
    await openFromServer(link.server, link.project, link.spot, link.access ?? null, link.mount ?? null, app);
    return;
  }
  app.deepLink = { ...link, name: packageNameFor(link.project) };
  if (sameId(app.project?.uuid, link.project)) {
    await resolveDeepLink(app);
    return;
  }
  app.screen = 'open';
}

/** After a package opened: if a deep link waits for this project, select its spot and center the canvas on it. */
async function resolveDeepLink(app: AppState) {
  const link = app.deepLink;
  if (!link || !app.project) return;
  if (!sameId(app.project.uuid, link.project)) {
    // Stays pending: loading the project's saved project.json next resolves it.
    app.notice = `This package's project is ${app.project.uuid}, not the linked ${link.project}: load its project.json on the Open screen to continue.`;
    return;
  }
  app.deepLink = null;
  if (app.pendingCommand) {
    const cmd = app.pendingCommand;
    app.pendingCommand = null;
    const { applyShellFocus } = await import('./shellcmd');
    await applyShellFocus(app, cmd);
    return;
  }
  if (!link.spot) return;
  const spot = app.spots.find((s) => sameId(s.id, link.spot));
  if (!spot) {
    app.notice = `Spot ${link.spot} is not in project ${app.project.name || app.project.uuid}.`;
    return;
  }
  if (!app.stitched[spot.area_id] && !(await stitch(spot.area_id, app))) return;
  app.areaId = spot.area_id;
  app.selection = [spot.id];
  app.focusSpot = spot.id;
  app.screen = 'canvas';
}

/**
 * A project file (Open screen, UPb Lab's project folder). Its own tab when it is open already;
 * the active tab when that holds no work (empty, or the blank project a package opens with);
 * otherwise a new tab that reopens the same pixels, so no open project is replaced.
 */
export async function loadProjectFile(file: File, app: AppState = current(), initial = false) {
  const p = JSON.parse(await file.text()) as Project;
  if (!initial) {
    const existing = existingProject(tabs.list, p.uuid);
    if (existing) app = selectExisting(existing);
    else if (holdsWork(app)) {
      const from = app;
      const source = tabs.list.find(t => t.pkg?.name === p.package.name && t.packageSource)?.packageSource ?? null;
      app = targetTab(undefined, from);
      app.packageSource = source ?? { name: p.package.name, files: [] };
      if (from.deepLink && sameId(from.deepLink.project, p.uuid)) { app.deepLink = from.deepLink; from.deepLink = null; }
      if (source) {
        const core = await app.core;
        const result = await app.run(`Reading ${p.package.name}`, async (): Promise<OpenResult> => 'paths' in source
          ? source.paths.length === 1 && PROJECT_PATH.test(source.paths[0])
            ? { status: 'opened', info: (await core.openProject(source.paths[0])).info }
            : await core.openPaths(source.paths)
          : await core.openFiles(source.name, source.files));
        if (result?.status === 'opened') app.pkg = result.info;
        else if (result && p.package.loose) app.pkg = (await app.run('Opening images', () => core.openLoose(p.package.loose as LooseSpec))) ?? null;
        app.areaId = app.pkg?.areas[0]?.id ?? null;
        if (app.pkg) app.screen = 'area';
      }
    }
  }
  const core = await app.core;
  const path = file.webkitRelativePath || file.name;
  const previous = app.storedCopy;
  app.loadingProject = true;
    app.clearHistory(); app.viewMode = false; app.draftHere = null;
  app.storedCopy = newerStoredCopy(p, path, file.lastModified);
  try {
    try {
      await core.loadProject(JSON.stringify(stripUi(p)));
    } catch (e) {
      app.storedCopy = previous;
      throw e;
    }
    app.notice = null;
    app.projectPath = { kind: 'file', path };
    app.project = JSON.parse(await core.projectJson());
    app.spots = [];
    app.grains = {};
    app.labels = [];
    app.tone = {};
    app.stitched = {};
    app.adopt(p);
    app.settings = { ...app.settings, ...(p.settings ?? {}), area_patterns: app.project!.settings.area_patterns ?? {}, spot_cursor_colour: app.project!.settings.spot_cursor_colour, outline_margin_um: app.project!.settings.outline_margin_um, outline_smooth: app.project!.settings.outline_smooth };
    fillParams(app);
    app.dirty = false;
    await restoreAreas(app);
    if (app.storedCopy) app.storedCopy = { ...app.storedCopy, ready: true };
    await resolveDeepLink(app);
  } finally { app.loadingProject = false; }
  await app.sync.check();
}

/** `base` with the values of `over` (objects merged at every depth, in `base`'s key order). */
function deepFill<T>(base: T, over: unknown): T {
  if (!over || typeof over !== 'object' || Array.isArray(over) || !base || typeof base !== 'object' || Array.isArray(base)) return (over ?? base) as T;
  const out: Record<string, unknown> = {};
  const o = over as Record<string, unknown>;
  for (const [k, v] of Object.entries(base as Record<string, unknown>)) out[k] = k in o ? deepFill(v, o[k]) : v;
  for (const [k, v] of Object.entries(o)) if (!(k in out)) out[k] = v;
  return out as T;
}

/** UI-held parameters from older saves lack newer fields (at any depth): fill them from the
 * core's record, so what the UI holds is what the core renders. */
function fillParams(app: AppState) {
  for (const a of app.project?.areas ?? []) {
    const base = a.params;
    const p = deepFill(base, app.params[a.id] ?? base);
    const tone = deepFill(base.tone, app.tone[a.id] ?? p.tone);
    app.params[a.id] = { ...p, segment: { ...p.segment, signal: p.segment.signal ?? null }, tone };
    app.tone[a.id] = tone;
  }
}

function stripUi(p: Project): Project {
  const { grainatlas_ui: _ui, ...rest } = p as Project & { grainatlas_ui?: unknown };
  return rest as Project;
}

export async function stitch(areaId: string, app: AppState = current()) {
  const core = await app.core;
  const params = $snapshotParams(app, areaId);
  await syncGrains(app, areaId);
  const r = await app.run(`Stitching ${areaId}`, (p) => core.stitch(areaId, params, p));
  if (!r) return false;
  const l = app.live[areaId];
  if (l) {
    clearTimeout(l.timer);
    l.settled = l.seq;
  }
  app.liveBusy = { ...app.liveBusy, [areaId]: false };
  // The project first: it records the parameters this stitch used (App.svelte compares them).
  app.project = keepPrintEdits(JSON.parse(await core.projectJson()), app.project);
  app.stitched = { ...app.stitched, [areaId]: r };
  app.preview = { ...app.preview, [areaId]: null };
  app.pixelsVersion++;
  app.persist();
  // The first stitch of an area that never had grains finds them (Re-stitch never re-finds).
  if (app.grains[areaId] === undefined) await findGrainsInArea(app, areaId);
  return true;
}

/**
 * Renders every area the project has a solved placement for, from that placement (no
 * re-stitch): a reopened project comes back as it was saved, grains and spots on the mosaics.
 * Areas whose placement no longer fits (another stitch setting) are left for Stitch.
 */
export async function restoreAreas(app: AppState = current()) {
  const core = await app.core;
  const todo = (app.project?.areas ?? []).filter((a) => a.placement && !app.stitched[a.id] && app.pkg?.areas.some((m) => m.id === a.id));
  for (const [k, a] of todo.entries()) {
    // The placement was solved with the recorded stitch settings: render with exactly those (the
    // UI's copy can differ in key order or in fields added since), so the core never re-stitches.
    app.params = { ...app.params, [a.id]: { ...app.params[a.id], stitch: a.params.stitch } };
    const params = $snapshotParams(app, a.id);
    await syncGrains(app, a.id);
    const r = await app.run(`Rendering ${a.id} (${k + 1} of ${todo.length})`, (p) => core.recompute(a.id, params, p));
    if (!r) return;
    app.stitched = { ...app.stitched, [a.id]: r };
    app.preview = { ...app.preview, [a.id]: null };
    app.pixelsVersion++;
  }
  if (todo.length) {
    app.project = keepPrintEdits(JSON.parse(await core.projectJson()), app.project);
    if (app.screen === 'area' && Object.keys(app.stitched).length) app.screen = 'canvas';
  }
}

/** Stitches every area; with `missing`, only those not rendered yet (command-line `--stitch`). */
export async function stitchAll(app: AppState = current(), missing = false) {
  for (const a of app.pkg?.areas ?? []) {
    if (missing && app.stitched[a.id]) continue;
    if (!(await stitch(a.id, app))) return;
  }
}

function $snapshotParams(app: AppState, areaId: string): AreaParams {
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

/** The parameters that change pixels after the stitch (black / white / gamma excluded), and the
 * grains when per-grain normalisation is on. */
export function liveKey(p: AreaParams | undefined, t: ToneParams | undefined, grains?: Grain[]): string {
  if (!p) return '';
  const { black: _b, white: _w, gamma: _g, ...tone } = (t ?? p.tone) as ToneParams;
  return canonical([p.photometric, p.blend, tone, tone.grain_norm?.enabled ? grainsKey(grains) : null]);
}

/** JSON with object keys sorted (the same parameters give the same key whatever their order). */
function canonical(v: unknown): string {
  return JSON.stringify(v, (_k, x) => (x && typeof x === 'object' && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : x));
}

function grainsKey(grains: Grain[] | undefined) {
  return (grains ?? []).map((g) => `${g.id}:${g.polygon_um.length}:${g.bbox_um.map((v) => v.toFixed(1)).join(',')}`).join(';');
}

/** Hands the area's current grains to the core when per-grain normalisation needs them. */
async function syncGrains(app: AppState, areaId: string) {
  const grains = app.grains[areaId] ?? [];
  const key = grainsKey(grains);
  if (app.syncedGrains[areaId] === key) return;
  const core = await app.core;
  await core.setGrains(areaId, JSON.parse(JSON.stringify(grains)) as Grain[]);
  app.syncedGrains[areaId] = key;
}

/** Call after a photometric parameter of a stitched area changed. */
export function liveChanged(areaId: string, settleMs = 450, app: AppState = current()) {
  if (app.readOnly || (app.remote && !bundlePixels(app)) || !app.stitched[areaId]) return;
  if (bundlePixels(app)) app.params[areaId] = { ...app.params[areaId], tone: curveTone(JSON.parse(JSON.stringify(app.tone[areaId] ?? app.params[areaId].tone))) };
  const l: LiveState = (app.live[areaId] ??= { seq: 0, settled: 0, timer: 0, previewing: false, again: false, settling: null });
  l.seq++;
  if (!app.liveBusy[areaId]) app.liveBusy = { ...app.liveBusy, [areaId]: true };
  void livePreview(app, areaId);
  clearTimeout(l.timer);
  l.timer = window.setTimeout(() => void settle(app, areaId), settleMs);
}

function previewLevel(app: AppState, areaId: string) {
  const levels = app.stitched[areaId].mosaic.levels;
  let L = Math.min(Math.max(0, app.drawLevel), levels.length - 1);
  while (L < levels.length - 1 && levels[L].w * levels[L].h > PREVIEW_PIXELS) L++;
  return L;
}

async function livePreview(app: AppState, areaId: string) {
  const l = app.live[areaId];
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
      const level = previewLevel(app, areaId);
      await syncGrains(app, areaId);
      const params = $snapshotParams(app, areaId);
      if (bundlePixels(app)) params.tone = curveTone(params.tone);
      const img = await core.preview(areaId, params, level);
      if (seq > l.settled && img) app.preview = { ...app.preview, [areaId]: { level, img, over: app.pixelsVersion } };
    } while (l.again);
  } catch {
    /* a preview is best effort; the settle reports errors */
  } finally {
    l.previewing = false;
  }
}

async function settle(app: AppState, areaId: string) {
  const l = app.live[areaId];
  if (l.settling) await l.settling;
  const seq = l.seq;
  if (seq <= l.settled) return;
  const run = (async () => {
    const core = await app.core;
    app.rendering++;
    try {
      await syncGrains(app, areaId);
      if (app.readOnly || (app.remote && !bundlePixels(app))) { l.settled = Math.max(l.settled, seq); return; }
      const params = $snapshotParams(app, areaId);
      if (bundlePixels(app)) params.tone = JSON.parse(JSON.stringify(app.params[areaId].tone));
      const published = bundlePixels(app);
      const r = published ? { ...app.stitched[areaId], mosaic: await core.retone(areaId, params.tone) } : await core.recompute(areaId, params);
      l.settled = Math.max(l.settled, seq);
      // A published image's re-tone leaves the core's record as it was: keep the adopted project.
      if (!published) app.project = keepPrintEdits(JSON.parse(await core.projectJson()), app.project);
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
export async function liveIdle(areaId: string, app: AppState = current()) {
  const l = app.live[areaId];
  while (l && (l.settling || l.seq > l.settled)) {
    if (l.settling) await l.settling;
    else await new Promise((r) => setTimeout(r, 50));
  }
}

// ---- one-click Auto, Before / After, Reset ------------------------------------

/** Auto: background leveling on, stretch and CLAHE from the histogram. Remembers what was there
 * for Before / After; every manual control stays editable. */
export async function autoTone(areaId: string) {
  const app = current();
  const core = await app.core;
  if (!app.stitched[areaId]) return;
  const before = $snapshotParams(app, areaId);
  const auto = await app.run('Auto tone', () => core.autoParams(areaId, before));
  if (!auto) return;
  app.compare = { ...app.compare, [areaId]: { other: before, showing: 'after' } };
  applyParams(app, areaId, auto);
}

/** Swaps between the parameters before Auto and the current ones. */
export function toggleBeforeAfter(areaId: string) {
  const app = current();
  const c = app.compare[areaId];
  if (!c) return;
  const now = $snapshotParams(app, areaId);
  app.compare = { ...app.compare, [areaId]: { other: now, showing: c.showing === 'after' ? 'before' : 'after' } };
  applyParams(app, areaId, c.other);
}

/** Back to the defaults for correction, blending and tone (stitch and segmentation are kept). */
export async function resetTone(areaId: string) {
  const app = current();
  const core = await app.core;
  const cur = $snapshotParams(app, areaId);
  const d = bundlePixels(app) ? { ...cur, tone: neutralTone() } : await core.defaultParams();
  app.compare = { ...app.compare, [areaId]: null };
  applyParams(app, areaId, { ...cur, photometric: d.photometric, blend: d.blend, tone: d.tone });
}

function applyParams(app: AppState, areaId: string, p: AreaParams) {
  const cur = app.params[areaId];
  const curve = bundlePixels(app) ? curveTone(JSON.parse(JSON.stringify(p.tone))) : p.tone;
  app.params = { ...app.params, [areaId]: { ...cur, photometric: p.photometric, blend: p.blend, tone: curve } };
  app.tone = { ...app.tone, [areaId]: p.tone };
}

/**
 * Save (Cmd/Ctrl+S): where the tab's project came from, a `.grainatlas/` folder (rewritten with
 * its pyramid, as before) or a `.gaproj` file; a package opened on its own gets a `<name>.gaproj`
 * project file next to it. The web build downloads the project file.
 */
export async function saveProject(app: AppState = current()) {
  if (!canWrite()) return;
  for (const id of Object.keys(app.live)) await liveIdle(id, app);
  const p = app.fullProject();
  if (!p) return;
  const core = await app.core;
  if (core.kind === 'tauri' && app.projectPath?.kind === 'folder') {
    if (!(await beforeCheckedAction(app, 'save'))) return;
    const p = app.fullProject();
    if (!p) return;
    const dir = app.projectPath.path;
    if (!canWrite()) return;
    const where = await app.run('Saving project', () => core.saveProject(p, {}, dir));
    if (where) {
      app.dirty = false;
      clearStoredCopy(p.uuid, dir);
      app.notice = `Saved ${where}`;
      await app.sync.afterSave();
    }
    return where;
  }
  return saveProjectFile(app, false);
}

/** Saves the lightweight project file (`.gaproj`): `ask` = Save As (a dialog on the desktop). */
export async function saveProjectFile(app: AppState = current(), ask = true) {
  if (!canWrite()) return;
  for (const id of Object.keys(app.live)) await liveIdle(id, app);
  if (!(await beforeCheckedAction(app, 'save'))) return;
  const p = app.fullProject();
  if (!p) return;
  const core = await app.core;
  const prev = app.projectPath?.kind === 'file' ? app.projectPath.path : null;
  if (!canWrite()) return;
  const where = await app.run('Saving project file', () => core.saveProjectFile(p, prev, ask));
  if (core.kind !== 'tauri') {
    if (app.error) return null;
    app.dirty = false;
    if (app.projectPath) clearStoredCopy(p.uuid, app.projectPath.path);
    await app.sync.afterSave();
    return null;
  }
  if (where) {
    app.projectPath = { kind: 'file', path: where };
    clearStoredCopy(p.uuid, where);
    app.notice = `Saved ${where}`;
    app.dirty = false;
    await app.sync.afterSave();
  }
  return where;
}

export async function exportPng(areaId: string) {
  const app = current();
  const core = await app.core;
  const tone = app.tone[areaId];
  const png = await app.run('Exporting PNG', () => core.exportPng(areaId, tone));
  if (png) download(png, `${areaId}_mosaic.png`, 'image/png');
}

/** Find missing areas first, or rerun all areas when every area has a saved result. */
export async function findAllGrains(app: AppState = current()) {
  await findGrainsInAreas(app);
}
