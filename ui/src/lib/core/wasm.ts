// Core implementation over the wasm worker. Cancelling terminates the worker
// (wasm cannot be interrupted from outside without shared memory), starts a
// fresh one, hands the package files over again and re-runs the areas that were
// already stitched, so only the cancelled step is lost.

import type { Core, PackageFile, RawImage } from './core';
import type { AreaParams, AreaRegistration, BundleOptions, Fiducial, Grain, LaserAdapterInfo, LaserFit, Published, LayoutParams, LayoutResult, LooseSpec, MosaicJson, OpenResult, PackageInfo, Progress, Project, SegmentParams, Spot, Label, StitchSummary, ToneParams } from './types';

type Pending = { resolve: (v: any) => void; reject: (e: Error) => void; onProgress?: (p: Progress) => void };

export class WasmCore implements Core {
  readonly kind = 'wasm' as const;
  private worker!: Worker;
  private seq = 0;
  private pending = new Map<number, Pending>();
  private opened: { name: string; files: PackageFile[] } | null = null;
  private loose: LooseSpec | null = null;
  private project: string | null = null;
  private stitched = new Map<string, AreaParams>();
  private grains = new Map<string, Grain[]>();
  private restoring: Promise<void> | null = null;

  constructor() {
    this.spawn();
  }

  private spawn() {
    this.worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    this.worker.onmessage = (e) => {
      const m = e.data;
      const p = this.pending.get(m.id);
      if (!p) return;
      if (m.progress) {
        p.onProgress?.(m.progress);
        return;
      }
      this.pending.delete(m.id);
      if (m.ok) p.resolve(m.value);
      else p.reject(new Error(m.error));
    };
    this.worker.onerror = (e) => {
      for (const p of this.pending.values()) p.reject(new Error(e.message || 'worker error'));
      this.pending.clear();
    };
  }

  private call<T>(op: string, args: unknown = {}, onProgress?: (p: Progress) => void): Promise<T> {
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, onProgress });
      this.worker.postMessage({ id, op, args });
    });
  }

  private async ready() {
    if (this.restoring) await this.restoring;
  }

  async dispose(): Promise<void> {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error('cancelled'));
    this.pending.clear();
    this.opened = null;
  }

  cancel() {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error('cancelled'));
    this.pending.clear();
    this.spawn();
    const opened = this.opened;
    const project = this.project;
    const redo = [...this.stitched.entries()];
    this.restoring = (async () => {
      if (!opened) return;
      await this.call('open', opened);
      if (this.loose) await this.call('openLoose', { spec: this.loose });
      if (project) await this.call('loadProject', { json: project });
      for (const [areaId, grains] of this.grains) await this.call('setGrains', { areaId, grains });
      for (const [areaId, params] of redo) await this.call('stitch', { areaId, params });
    })().finally(() => {
      this.restoring = null;
    });
  }

  async openFiles(name: string, files: PackageFile[], onProgress?: (p: Progress) => void): Promise<OpenResult> {
    this.opened = { name, files };
    this.loose = null;
    this.project = null;
    this.stitched.clear();
    this.grains.clear();
    return this.call('open', { name, files }, onProgress);
  }

  async openPaths(): Promise<OpenResult> {
    throw new Error('opening by path needs the desktop app; choose the folder or files instead');
  }

  async openLoose(spec: LooseSpec): Promise<PackageInfo> {
    await this.ready();
    const info = await this.call<PackageInfo>('openLoose', { spec });
    this.loose = spec;
    this.project = null;
    this.stitched.clear();
    return info;
  }

  async openProject(): Promise<{ info: PackageInfo; project: Project }> {
    throw new Error('opening a project folder needs the desktop app; open the package, then load its project.json');
  }

  async blank(name: string): Promise<void> {
    await this.ready();
    this.opened = null;
    this.loose = null;
    this.project = null;
    this.stitched.clear();
    this.grains.clear();
    await this.call('blank', { name });
  }

  async loadProject(json: string) {
    await this.ready();
    this.project = json;
    await this.call('loadProject', { json });
  }

  async projectJson(): Promise<string> {
    await this.ready();
    const j = await this.call<string>('projectJson');
    this.project = j;
    return j;
  }

  async navigation(): Promise<RawImage | null> {
    await this.ready();
    return this.call('navigation');
  }

  async tileThumb(area: number, tile: number, max: number): Promise<RawImage> {
    await this.ready();
    return this.call('tileThumb', { area, tile, max });
  }

  async stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary> {
    await this.ready();
    const r = await this.call<StitchSummary>('stitch', { areaId, params }, onProgress);
    this.stitched.set(areaId, params);
    this.project = await this.call<string>('projectJson');
    return r;
  }

  async recompute(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary> {
    await this.ready();
    const r = await this.call<StitchSummary>('recompute', { areaId, params }, onProgress);
    this.stitched.set(areaId, params);
    this.project = await this.call<string>('projectJson');
    return r;
  }

  async autoParams(areaId: string, params: AreaParams): Promise<AreaParams> {
    await this.ready();
    return this.call('autoParams', { areaId, params });
  }

  async setGrains(areaId: string, grains: Grain[]): Promise<void> {
    await this.ready();
    this.grains.set(areaId, grains);
    await this.call('setGrains', { areaId, grains });
  }

  async defaultParams(): Promise<AreaParams> {
    return this.call('defaultParams');
  }

  async preview(areaId: string, params: AreaParams, level: number): Promise<RawImage> {
    await this.ready();
    return this.call('preview', { areaId, params, level });
  }

  async pyramidTile(areaId: string, level: number, col: number, row: number): Promise<RawImage> {
    await this.ready();
    return this.call('pyramidTile', { areaId, level, col, row });
  }

  async histogram(areaId: string): Promise<Uint32Array> {
    await this.ready();
    return this.call('histogram', { areaId });
  }

  async retone(areaId: string, tone: ToneParams): Promise<MosaicJson> {
    await this.ready();
    const p = this.stitched.get(areaId);
    if (p) this.stitched.set(areaId, { ...p, tone });
    return this.call('retone', { areaId, tone });
  }

  async toneLut(tone: ToneParams): Promise<Uint8Array> {
    return this.call('toneLut', { tone });
  }

  async segment(areaId: string, params: SegmentParams, previous: Grain[]): Promise<Grain[]> {
    await this.ready();
    return this.call('segment', { areaId, params, previous });
  }

  async grainAt(areaId: string, x: number, y: number, params: SegmentParams): Promise<Grain | null> {
    await this.ready();
    return this.call('grainAt', { areaId, x, y, params });
  }

  async splitGrain(areaId: string, grain: Grain, line: [number, number, number, number]): Promise<Grain[]> {
    await this.ready();
    return this.call('splitGrain', { areaId, grain, line });
  }

  async mergeGrains(areaId: string, grains: Grain[]): Promise<Grain> {
    await this.ready();
    return this.call('mergeGrains', { areaId, grains });
  }

  async refineGrain(areaId: string, grain: Grain): Promise<Grain> {
    await this.ready();
    return this.call('refineGrain', { areaId, grain });
  }

  async renumber(grains: Grain[], tolerance: number, from?: { grainId: string; number: number }): Promise<Grain[]> {
    return this.call('renumber', { grains, tolerance, from });
  }

  async setLink(mountId: string | null, samples: Record<string, string>): Promise<void> {
    await this.ready();
    await this.call('setLink', { mountId: mountId ?? '', samples });
    this.project = await this.call<string>('projectJson');
  }

  async exportSpots(spots: Spot[], grains: Record<string, Grain[]>, format: 'csv' | 'geojson'): Promise<string> {
    await this.ready();
    return this.call('exportSpots', { spots, grains, format });
  }

  async exportGrains(areaId: string, grains: Grain[]): Promise<string> {
    await this.ready();
    return this.call('exportGrains', { areaId, grains });
  }

  async exportPng(areaId: string, tone: ToneParams): Promise<Uint8Array> {
    await this.ready();
    return this.call('exportPng', { areaId, tone });
  }

  async layout(params: LayoutParams, grains: Record<string, Grain[]>, spots: Spot[], labels: Label[], tone: Record<string, ToneParams>): Promise<LayoutResult> {
    await this.ready();
    return this.call('layout', { params, grains, spots, labels, tone });
  }

  async register(pairs: [[number, number], [number, number]][]) {
    return this.call<{ matrix: [number, number, number, number, number, number]; rms_um: number; residuals_um: number[] } | null>('register', { pairs });
  }

  async pdf(pages: string[]): Promise<Uint8Array> {
    return this.call('pdf', { pages });
  }

  async atlasZip(project: Project, options: BundleOptions): Promise<Uint8Array> {
    await this.ready();
    return this.call('atlasZip', { project, options });
  }

  async saveAtlas(): Promise<string | null> {
    return null; // the web build saves through lib/atlas.ts (download, or a folder where the browser allows it)
  }

  async laserFit(fiducials: Fiducial[], units: string): Promise<LaserFit> {
    return this.call('laserFit', { fiducials, units });
  }

  async laserAdapters(): Promise<LaserAdapterInfo[]> {
    return this.call('laserAdapters');
  }

  async setLaser(areaId: string, reg: AreaRegistration | null): Promise<void> {
    await this.ready();
    await this.call('setLaser', { areaId, reg });
    this.project = await this.call<string>('projectJson');
  }

  async laserExport(spots: Spot[], areaId: string, adapter?: string): Promise<string> {
    await this.ready();
    return this.call('laserExport', { spots, areaId, adapter });
  }

  async recordPublished(published: Published): Promise<void> {
    await this.ready();
    await this.call('recordPublished', { published });
    this.project = await this.call<string>('projectJson');
  }

  async resetWindow(): Promise<void> {
    /* the browser owns the window */
  }

  async saveProjectFile(project: Project): Promise<string | null> {
    const { download } = await import('../files');
    download(JSON.stringify(project, null, 2), `${project.name || project.package.name}.gaproj`, 'application/json');
    return null;
  }
  async saveProject(_project: Project, _extras: Record<string, string>): Promise<string | null> {
    return null; // the web build downloads files instead (see lib/files.ts)
  }
}
