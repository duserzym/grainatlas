// Core implementation over Tauri commands: the native core (all cores via
// rayon) with packages read from disk by path. Mirrors the wasm worker API.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Core, PackageFile, RawImage } from './core';
import type { AreaParams, Grain, LayoutParams, LayoutResult, MosaicJson, PackageInfo, Progress, Project, SegmentParams, Spot, Label, StitchSummary, ToneParams } from './types';

function img(buf: ArrayBuffer | Uint8Array | number[]): RawImage | null {
  const b = buf instanceof Uint8Array ? buf : buf instanceof ArrayBuffer ? new Uint8Array(buf) : Uint8Array.from(buf);
  if (b.length < 8) return null;
  const dv = new DataView(b.buffer, b.byteOffset, 8);
  return { w: dv.getUint32(0, true), h: dv.getUint32(4, true), data: b.subarray(8) };
}

function bytes(buf: ArrayBuffer | Uint8Array | number[]): Uint8Array {
  return buf instanceof Uint8Array ? buf : buf instanceof ArrayBuffer ? new Uint8Array(buf) : Uint8Array.from(buf);
}

export class TauriCore implements Core {
  readonly kind = 'tauri' as const;
  private onProgress: ((p: Progress) => void) | null = null;

  constructor() {
    listen<Progress>('grainatlas://progress', (e) => this.onProgress?.(e.payload));
  }

  async openFiles(_name: string, _files: PackageFile[]): Promise<PackageInfo> {
    throw new Error('In the desktop app, choose or drop the package folder (it is read from disk).');
  }
  async openPath(path: string): Promise<PackageInfo> {
    return JSON.parse(await invoke<string>('open_path', { path }));
  }
  async openProject(path: string): Promise<{ info: PackageInfo; project: Project }> {
    return JSON.parse(await invoke<string>('open_project', { path }));
  }
  async loadProject(json: string) {
    await invoke('load_project', { json });
  }
  async projectJson(): Promise<string> {
    return invoke<string>('project_json');
  }
  async navigation(): Promise<RawImage | null> {
    return img(await invoke<ArrayBuffer>('navigation'));
  }
  async tileThumb(area: number, tile: number, max: number): Promise<RawImage> {
    return img(await invoke<ArrayBuffer>('tile_thumb', { area, tile, max }))!;
  }
  async stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary> {
    this.onProgress = onProgress ?? null;
    try {
      return JSON.parse(await invoke<string>('stitch', { areaId, params: JSON.stringify(params) }));
    } catch (e) {
      throw new Error(String(e) === 'cancelled' ? 'cancelled' : String(e));
    } finally {
      this.onProgress = null;
    }
  }
  cancel() {
    invoke('cancel');
  }
  async pyramidTile(areaId: string, level: number, col: number, row: number): Promise<RawImage> {
    return img(await invoke<ArrayBuffer>('pyramid_tile', { areaId, level, col, row }))!;
  }
  async histogram(areaId: string): Promise<Uint32Array> {
    return Uint32Array.from(await invoke<number[]>('histogram', { areaId }));
  }
  async retone(areaId: string, tone: ToneParams): Promise<MosaicJson> {
    return JSON.parse(await invoke<string>('retone', { areaId, tone: JSON.stringify(tone) }));
  }
  async toneLut(tone: ToneParams): Promise<Uint8Array> {
    return bytes(await invoke<ArrayBuffer>('tone_lut', { tone: JSON.stringify(tone) }));
  }
  async segment(areaId: string, params: SegmentParams, previous: Grain[]): Promise<Grain[]> {
    return JSON.parse(await invoke<string>('segment', { areaId, params: JSON.stringify(params), previous: JSON.stringify(previous) }));
  }
  async grainAt(areaId: string, x: number, y: number, params: SegmentParams): Promise<Grain | null> {
    return JSON.parse(await invoke<string>('grain_at', { areaId, x, y, params: JSON.stringify(params) }));
  }
  async splitGrain(areaId: string, grain: Grain, line: [number, number, number, number]): Promise<Grain[]> {
    return JSON.parse(await invoke<string>('split_grain', { areaId, grain: JSON.stringify(grain), line }));
  }
  async mergeGrains(areaId: string, grains: Grain[]): Promise<Grain> {
    return JSON.parse(await invoke<string>('merge_grains', { areaId, grains: JSON.stringify(grains) }));
  }
  async refineGrain(areaId: string, grain: Grain): Promise<Grain> {
    return JSON.parse(await invoke<string>('refine_grain', { areaId, grain: JSON.stringify(grain) }));
  }
  async renumber(grains: Grain[], tolerance: number, from?: { grainId: string; number: number }): Promise<Grain[]> {
    return JSON.parse(await invoke<string>('renumber', { grains: JSON.stringify(grains), tolerance, from: from ? JSON.stringify(from) : '' }));
  }
  async exportSpots(spots: Spot[], grains: Record<string, Grain[]>, format: 'csv' | 'geojson'): Promise<string> {
    return invoke<string>('export_spots', { spots: JSON.stringify(spots), grains: JSON.stringify(grains), format });
  }
  async setLink(mountId: string | null, samples: Record<string, string>): Promise<void> {
    await invoke('set_link', { mountId: mountId ?? '', samples: JSON.stringify(samples) });
  }
  async exportGrains(areaId: string, grains: Grain[]): Promise<string> {
    return invoke<string>('export_grains', { areaId, grains: JSON.stringify(grains) });
  }
  async exportPng(areaId: string, tone: ToneParams): Promise<Uint8Array> {
    return bytes(await invoke<ArrayBuffer>('export_png', { areaId, tone: JSON.stringify(tone) }));
  }
  async layout(params: LayoutParams, grains: Record<string, Grain[]>, spots: Spot[], labels: Label[], tone: Record<string, ToneParams>): Promise<LayoutResult> {
    return JSON.parse(await invoke<string>('layout', { params: JSON.stringify(params), grains: JSON.stringify(grains), spots: JSON.stringify(spots), labels: JSON.stringify(labels), tone: JSON.stringify(tone) }));
  }
  async register(pairs: [[number, number], [number, number]][]) {
    return JSON.parse(await invoke<string>('register', { pairs: JSON.stringify(pairs) }));
  }
  async pdf(pages: string[]): Promise<Uint8Array> {
    return bytes(await invoke<ArrayBuffer>('pdf', { pages: JSON.stringify(pages) }));
  }
  async saveProject(project: Project, _extras: Record<string, string>): Promise<string | null> {
    return invoke<string | null>('save_project', { project: JSON.stringify(project), ask: false });
  }
}

/** Saves bytes or text through the native save dialog. */
export async function saveFile(name: string, data: Uint8Array | string): Promise<string | null> {
  const body = typeof data === 'string' ? new TextEncoder().encode(data) : data;
  return invoke<string | null>('save_bytes', body, { headers: { 'x-name': encodeURIComponent(name) } });
}
