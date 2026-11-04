// Core implementation over Tauri commands: the native core (all cores via
// rayon) with packages read from disk by path. Mirrors the wasm worker API.

import { invoke as tauriInvoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Core, PackageFile, RawImage } from './core';
import type { AreaParams, EditOp, AreaRegistration, BundleOptions, Fiducial, Grain, LaserAdapterInfo, LaserFit, Published, LayoutParams, LayoutResult, LooseSpec, MosaicJson, OpenResult, Ownership, PackageInfo, Progress, Project, SegmentParams, SheetParams, SheetResult, Spot, Label, StitchSummary, ToneParams } from './types';

function img(buf: ArrayBuffer | Uint8Array | number[]): RawImage | null {
  const b = buf instanceof Uint8Array ? buf : buf instanceof ArrayBuffer ? new Uint8Array(buf) : Uint8Array.from(buf);
  if (b.length < 8) return null;
  const dv = new DataView(b.buffer, b.byteOffset, 8);
  return { w: dv.getUint32(0, true), h: dv.getUint32(4, true), data: b.subarray(8) };
}

function bytes(buf: ArrayBuffer | Uint8Array | number[]): Uint8Array {
  return buf instanceof Uint8Array ? buf : buf instanceof ArrayBuffer ? new Uint8Array(buf) : Uint8Array.from(buf);
}

/** Commands that do not touch a session take no tab. */
const STATELESS = new Set(['default_params', 'tone_lut', 'renumber', 'register', 'pdf', 'laser_fit', 'laser_adapters', 'reset_window']);

/** The tab whose package folder a save dialog starts in (the active one). */
let saveTab = 0;
export function setSaveTab(tab: number) {
  saveTab = tab;
}

export class TauriCore implements Core {
  readonly kind = 'tauri' as const;
  private onProgress: ((p: Progress) => void) | null = null;
  private unlisten: Promise<() => void>;

  constructor(readonly tab: number) {
    // Progress of every tab arrives on one event; each core takes its own.
    this.unlisten = listen<Progress & { tab: number }>('grainatlas://progress', (e) => {
      if (e.payload.tab === this.tab) this.onProgress?.(e.payload);
    });
  }

  async dispose(): Promise<void> {
    (await this.unlisten)();
    await tauriInvoke('close_tab', { tab: this.tab });
  }

  private call<T = unknown>(cmd: string, args: InvokeArgs = {}): Promise<T> {
    return tauriInvoke<T>(cmd, STATELESS.has(cmd) ? args : { ...(args as Record<string, unknown>), tab: this.tab });
  }

  async openFiles(_name: string, _files: PackageFile[]): Promise<OpenResult> {
    throw new Error('In the desktop app, choose or drop the folder or files (they are read from disk).');
  }
  async openPaths(paths: string[]): Promise<OpenResult> {
    return JSON.parse(await this.call<string>('open_paths', { paths }));
  }
  async openLoose(spec: LooseSpec): Promise<PackageInfo> {
    return JSON.parse(await this.call<string>('open_loose', { spec: JSON.stringify(spec) }));
  }
  async openProject(path: string): Promise<{ info: PackageInfo; project: Project }> {
    return JSON.parse(await this.call<string>('open_project', { path }));
  }
  async blank(name: string): Promise<void> {
    await this.call('blank', { name });
  }
  async loadProject(json: string) {
    await this.call('load_project', { json });
  }
  async projectJson(): Promise<string> {
    return this.call<string>('project_json');
  }
  async navigation(): Promise<RawImage | null> {
    return img(await this.call<ArrayBuffer>('navigation'));
  }
  async tileThumb(area: number, tile: number, max: number): Promise<RawImage> {
    return img(await this.call<ArrayBuffer>('tile_thumb', { area, tile, max }))!;
  }
  async stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary> {
    this.onProgress = onProgress ?? null;
    try {
      return JSON.parse(await this.call<string>('stitch', { areaId, params: JSON.stringify(params) }));
    } catch (e) {
      throw new Error(String(e) === 'cancelled' ? 'cancelled' : String(e));
    } finally {
      this.onProgress = null;
    }
  }
  async recompute(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary> {
    this.onProgress = onProgress ?? null;
    try {
      return JSON.parse(await this.call<string>('recompute', { areaId, params: JSON.stringify(params) }));
    } finally {
      this.onProgress = null;
    }
  }
  async preview(areaId: string, params: AreaParams, level: number): Promise<RawImage> {
    return img(await this.call<ArrayBuffer>('preview', { areaId, params: JSON.stringify(params), level }))!;
  }
  async autoParams(areaId: string, params: AreaParams): Promise<AreaParams> {
    return JSON.parse(await this.call<string>('auto_params', { areaId, params: JSON.stringify(params) }));
  }
  async setGrains(areaId: string, grains: Grain[]): Promise<void> {
    await this.call('set_grains', { areaId, grains: JSON.stringify(grains) });
  }
  async defaultParams(): Promise<AreaParams> {
    return JSON.parse(await this.call<string>('default_params'));
  }
  cancel() {
    this.call('cancel');
  }
  async pyramidTile(areaId: string, level: number, col: number, row: number): Promise<RawImage> {
    return img(await this.call<ArrayBuffer>('pyramid_tile', { areaId, level, col, row }))!;
  }
  async histogram(areaId: string): Promise<Uint32Array> {
    return Uint32Array.from(await this.call<number[]>('histogram', { areaId }));
  }
  async retone(areaId: string, tone: ToneParams): Promise<MosaicJson> {
    return JSON.parse(await this.call<string>('retone', { areaId, tone: JSON.stringify(tone) }));
  }
  async toneLut(tone: ToneParams): Promise<Uint8Array> {
    return bytes(await this.call<ArrayBuffer>('tone_lut', { tone: JSON.stringify(tone) }));
  }
  async segment(areaId: string, params: SegmentParams, previous: Grain[]): Promise<Grain[]> {
    return JSON.parse(await this.call<string>('segment', { areaId, params: JSON.stringify(params), previous: JSON.stringify(previous) }));
  }
  async grainAt(areaId: string, x: number, y: number, params: SegmentParams): Promise<Grain | null> {
    return JSON.parse(await this.call<string>('grain_at', { areaId, x, y, params: JSON.stringify(params) }));
  }
  async splitGrain(areaId: string, grain: Grain, line: [number, number, number, number]): Promise<Grain[]> {
    return JSON.parse(await this.call<string>('split_grain', { areaId, grain: JSON.stringify(grain), line }));
  }
  async mergeGrains(areaId: string, grains: Grain[]): Promise<Grain> {
    return JSON.parse(await this.call<string>('merge_grains', { areaId, grains: JSON.stringify(grains) }));
  }
  async refineGrain(areaId: string, grain: Grain): Promise<Grain> {
    return JSON.parse(await this.call<string>('refine_grain', { areaId, grain: JSON.stringify(grain) }));
  }
  async snapOutline(areaId: string, path: [number, number][], widthUm: number, contrast: number): Promise<[number, number][]> {
    return JSON.parse(await this.call<string>('snap_outline', { areaId, path: JSON.stringify(path), widthUm, contrast }));
  }
  async outlineGrain(areaId: string, polygon: [number, number][], base: Grain | null, op: EditOp): Promise<Grain | null> {
    return JSON.parse(await this.call<string>('outline_grain', { areaId, polygon: JSON.stringify(polygon), base: base ? JSON.stringify(base) : '', op }));
  }
  async brushGrain(areaId: string, stroke: [number, number][], sizeUm: number, base: Grain | null, op: EditOp): Promise<Grain | null> {
    return JSON.parse(await this.call<string>('brush_grain', { areaId, stroke: JSON.stringify(stroke), sizeUm, base: base ? JSON.stringify(base) : '', op }));
  }
  async renumber(grains: Grain[], tolerance: number, from?: { grainId: string; number: number }): Promise<Grain[]> {
    return JSON.parse(await this.call<string>('renumber', { grains: JSON.stringify(grains), tolerance, from: from ? JSON.stringify(from) : '' }));
  }
  async exportSpots(spots: Spot[], grains: Record<string, Grain[]>, format: 'csv' | 'geojson'): Promise<string> {
    return this.call<string>('export_spots', { spots: JSON.stringify(spots), grains: JSON.stringify(grains), format });
  }
  async setLink(mountId: string | null, samples: Record<string, string>): Promise<void> {
    await this.call('set_link', { mountId: mountId ?? '', samples: JSON.stringify(samples) });
  }
  async exportGrains(areaId: string, grains: Grain[]): Promise<string> {
    return this.call<string>('export_grains', { areaId, grains: JSON.stringify(grains) });
  }
  async exportPng(areaId: string, tone: ToneParams): Promise<Uint8Array> {
    return bytes(await this.call<ArrayBuffer>('export_png', { areaId, tone: JSON.stringify(tone) }));
  }
  async layout(params: LayoutParams, grains: Record<string, Grain[]>, spots: Spot[], labels: Label[], tone: Record<string, ToneParams>): Promise<LayoutResult> {
    return JSON.parse(await this.call<string>('layout', { params: JSON.stringify(params), grains: JSON.stringify(grains), spots: JSON.stringify(spots), labels: JSON.stringify(labels), tone: JSON.stringify(tone) }));
  }
  async register(pairs: [[number, number], [number, number]][]) {
    return JSON.parse(await this.call<string>('register', { pairs: JSON.stringify(pairs) }));
  }
  async pdf(pages: string[]): Promise<Uint8Array> {
    return bytes(await this.call<ArrayBuffer>('pdf', { pages: JSON.stringify(pages) }));
  }
  async atlasZip(project: Project, options: BundleOptions): Promise<Uint8Array> {
    return bytes(await this.call<ArrayBuffer>('atlas_zip', { project: JSON.stringify(project), options: JSON.stringify(options) }));
  }
  async saveAtlas(project: Project, options: BundleOptions, zip: boolean): Promise<string | null> {
    return this.call<string | null>('save_atlas', { project: JSON.stringify(project), options: JSON.stringify(options), zip });
  }
  async laserFit(fiducials: Fiducial[], units: string): Promise<LaserFit> {
    return JSON.parse(await this.call<string>('laser_fit', { fiducials: JSON.stringify(fiducials), units }));
  }
  async laserAdapters(): Promise<LaserAdapterInfo[]> {
    return JSON.parse(await this.call<string>('laser_adapters'));
  }
  async setLaser(areaId: string, reg: AreaRegistration | null): Promise<void> {
    await this.call('set_laser', { areaId, registration: reg ? JSON.stringify(reg) : '' });
  }
  async laserExport(spots: Spot[], areaId: string, adapter?: string): Promise<string> {
    return this.call<string>('laser_export', { spots: JSON.stringify(spots), areaId, adapter: adapter ?? '' });
  }
  async recordPublished(published: Published): Promise<void> {
    await this.call('record_published', { published: JSON.stringify(published) });
  }
  async resetWindow(): Promise<void> {
    await this.call('reset_window');
  }
  async saveProjectFile(project: Project, path: string | null, ask: boolean): Promise<string | null> {
    return this.call<string | null>('save_project_file', { project: JSON.stringify(project), path, ask });
  }
  async saveProject(project: Project, _extras: Record<string, string>): Promise<string | null> {
    return this.call<string | null>('save_project', { project: JSON.stringify(project), ask: false });
  }
  async ownership(grains: Record<string, Grain[]>): Promise<Ownership> {
    return JSON.parse(await this.call<string>('ownership', { grains: JSON.stringify(grains) }));
  }
  async printSheets(params: SheetParams, grains: Record<string, Grain[]>, tone: Record<string, ToneParams>): Promise<SheetResult> {
    return JSON.parse(await this.call<string>('print_sheets', { params: JSON.stringify(params), grains: JSON.stringify(grains), tone: JSON.stringify(tone) }));
  }
}

/** Saves bytes or text through the native save dialog. */
export async function saveFile(name: string, data: Uint8Array | string): Promise<string | null> {
  const body = typeof data === 'string' ? new TextEncoder().encode(data) : data;
  return tauriInvoke<string | null>('save_bytes', body, { headers: { 'x-name': encodeURIComponent(name), 'x-tab': String(saveTab) } });
}
