// Actions shared by the screens: open, stitch, tone, project save/load.

import type { PackageFile } from './core/core';
import type { AreaParams, Project, ToneParams } from './core/types';
import { app } from './store.svelte';
import { download } from './files';

export async function openFiles(name: string, files: PackageFile[]) {
  const core = await app.core;
  const info = await app.run(`Reading ${name}`, (p) => core.openFiles(name, files, p));
  if (!info) return;
  await adoptPackage(info);
}

export async function openPath(path: string) {
  const core = await app.core;
  if (/\.grainmap\/?$/.test(path)) {
    const r = await app.run(`Opening ${path}`, () => core.openProject(path));
    if (!r) return;
    await adoptPackage(r.info, r.project);
    return;
  }
  const info = await app.run(`Reading ${path}`, () => core.openPath(path));
  if (!info) return;
  await adoptPackage(info);
}

async function adoptPackage(info: import('./core/types').PackageInfo, fromFolder: Project | null = null) {
  const core = await app.core;
  app.pkg = info;
  app.stitched = {};
  app.grains = {};
  app.spots = [];
  app.labels = [];
  app.selection = [];
  app.tone = {};
  // A project saved earlier for this package (browser storage) is picked up again.
  let saved: Project | null = fromFolder;
  if (!saved) {
    try {
      const s = localStorage.getItem(`grainmap.project.${info.name}`);
      if (s) saved = JSON.parse(s);
    } catch {
      saved = null;
    }
  }
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
  for (const a of app.project!.areas) {
    if (!app.params[a.id]) app.params[a.id] = a.params;
    if (!app.tone[a.id]) app.tone[a.id] = a.params.tone;
  }
  app.areaId = info.areas[0]?.id ?? null;
  app.screen = 'area';
}

export async function loadProjectFile(file: File) {
  const text = await file.text();
  const p = JSON.parse(text) as Project;
  const core = await app.core;
  await core.loadProject(JSON.stringify(stripUi(p)));
  app.project = JSON.parse(await core.projectJson());
  app.adopt(p);
  app.settings = { ...app.settings, ...(p.settings ?? {}) };
}

function stripUi(p: Project): Project {
  const { grainmap_ui: _ui, ...rest } = p as Project & { grainmap_ui?: unknown };
  return rest as Project;
}

export async function stitch(areaId: string) {
  const core = await app.core;
  const params = $snapshotParams(areaId);
  const r = await app.run(`Stitching ${areaId}`, (p) => core.stitch(areaId, params, p));
  if (!r) return false;
  app.stitched = { ...app.stitched, [areaId]: r };
  app.project = JSON.parse(await core.projectJson());
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

/** Stretch percentiles and CLAHE change pixels in the core; black/white/gamma are a display LUT. */
export async function retone(areaId: string, tone: ToneParams) {
  const core = await app.core;
  const r = await app.run('Applying tone', () => core.retone(areaId, tone));
  if (!r) return;
  const s = app.stitched[areaId];
  if (s) app.stitched = { ...app.stitched, [areaId]: { ...s, mosaic: r } };
  app.pixelsVersion++;
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
  download(JSON.stringify(p, null, 2), `${p.package.name}.grainmap.project.json`, 'application/json');
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
