// The one interface the UI talks to. Two implementations: the wasm core in a
// Web Worker (static web page) and the native core through Tauri commands.

import type {
  AreaParams, Grain, LayoutParams, LayoutResult, MosaicJson, PackageInfo, Progress, Spot, Label, StitchSummary, ToneParams, SegmentParams, Project,
} from './types';

export interface RawImage { w: number; h: number; data: Uint8Array }

export interface PackageFile { path: string; file: Blob }

export interface Core {
  readonly kind: 'wasm' | 'tauri';
  /** Web: hand over the files of a package folder. */
  openFiles(name: string, files: PackageFile[], onProgress?: (p: Progress) => void): Promise<PackageInfo>;
  /** Desktop: open a package folder by path. */
  openPath(path: string): Promise<PackageInfo>;
  loadProject(json: string): Promise<void>;
  projectJson(): Promise<string>;
  navigation(): Promise<RawImage | null>;
  tileThumb(area: number, tile: number, max: number): Promise<RawImage>;
  stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary>;
  /** Stops the running step (the area stays unstitched). */
  cancel(): void;
  pyramidTile(areaId: string, level: number, col: number, row: number): Promise<RawImage>;
  histogram(areaId: string): Promise<Uint32Array>;
  retone(areaId: string, tone: ToneParams): Promise<MosaicJson>;
  toneLut(tone: ToneParams): Promise<Uint8Array>;
  segment(areaId: string, params: SegmentParams, previous: Grain[]): Promise<Grain[]>;
  /** Click-grow a grain at a point (manual add). */
  grainAt(areaId: string, xUm: number, yUm: number, params: SegmentParams): Promise<Grain | null>;
  /** Split a grain along a line (um), returns the parts. */
  splitGrain(areaId: string, grain: Grain, line: [number, number, number, number]): Promise<Grain[]>;
  mergeGrains(areaId: string, grains: Grain[]): Promise<Grain>;
  /** GrabCut refinement of one grain. */
  refineGrain(areaId: string, grain: Grain): Promise<Grain>;
  renumber(grains: Grain[], tolerance: number, from?: { grainId: string; number: number }): Promise<Grain[]>;
  exportSpots(spots: Spot[], grains: Grain[], format: 'csv' | 'geojson'): Promise<string>;
  exportGrains(areaId: string, grains: Grain[]): Promise<string>;
  exportPng(areaId: string, tone: ToneParams): Promise<Uint8Array>;
  layout(params: LayoutParams, grains: Record<string, Grain[]>, spots: Spot[], labels: Label[], tone: Record<string, ToneParams>): Promise<LayoutResult>;
  pdf(pages: string[]): Promise<Uint8Array>;
  /** Desktop: write the project folder (project.json, pyramid, masks, spots). */
  saveProject(project: Project, extras: Record<string, string>): Promise<string | null>;
}

let instance: Promise<Core> | null = null;

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function getCore(): Promise<Core> {
  if (!instance) {
    instance = isTauri() ? import('./tauri').then((m) => new m.TauriCore()) : import('./wasm').then((m) => new m.WasmCore());
  }
  return instance;
}
