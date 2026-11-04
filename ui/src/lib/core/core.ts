// The one interface the UI talks to. Two implementations: the wasm core in a
// Web Worker (static web page, and the page hosted by UPb Studio) and the native
// core through grainatlas's own Tauri commands (desktop app).

import { isHosted } from '@lab/bridge';

import type {
  AreaParams, Grain, LayoutParams, LayoutResult, LooseSpec, MosaicJson, OpenResult, PackageInfo, Progress, Spot, Label, StitchSummary, ToneParams, SegmentParams, Project,
} from './types';

export interface RawImage { w: number; h: number; data: Uint8Array }

export interface PackageFile { path: string; file: Blob }

export interface Core {
  readonly kind: 'wasm' | 'tauri';
  /** Web: hand over the files of a package folder, an area folder or loose images. */
  openFiles(name: string, files: PackageFile[], onProgress?: (p: Progress) => void): Promise<OpenResult>;
  /** Desktop: open one folder, or a set of files, by path. */
  openPaths(paths: string[]): Promise<OpenResult>;
  /** After `needs_scale`: open the same images as loose files with the user's scale. */
  openLoose(spec: LooseSpec): Promise<PackageInfo>;
  /** Desktop: open a saved `<name>.grainatlas/` folder (its package and project). */
  openProject(path: string): Promise<{ info: PackageInfo; project: Project }>;
  loadProject(json: string): Promise<void>;
  projectJson(): Promise<string>;
  navigation(): Promise<RawImage | null>;
  tileThumb(area: number, tile: number, max: number): Promise<RawImage>;
  stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary>;
  /** New parameters on a stitched area (re-tone, re-correct from the solved placement, or re-stitch). */
  recompute(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary>;
  /** The whole mosaic at pyramid `level` with `params`, for a live preview while a control moves. */
  preview(areaId: string, params: AreaParams, level: number): Promise<RawImage>;
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
  exportSpots(spots: Spot[], grains: Record<string, Grain[]>, format: 'csv' | 'geojson'): Promise<string>;
  /** laicpmsdb mount id and per-area sample names into the project record. */
  setLink(mountId: string | null, samples: Record<string, string>): Promise<void>;
  exportGrains(areaId: string, grains: Grain[]): Promise<string>;
  exportPng(areaId: string, tone: ToneParams): Promise<Uint8Array>;
  layout(params: LayoutParams, grains: Record<string, Grain[]>, spots: Spot[], labels: Label[], tone: Record<string, ToneParams>): Promise<LayoutResult>;
  pdf(pages: string[]): Promise<Uint8Array>;
  /** Reference-image registration: pairs of [image px, mosaic um] → affine (image px → um). */
  register(pairs: [[number, number], [number, number]][]): Promise<{ matrix: [number, number, number, number, number, number]; rms_um: number; residuals_um: number[] } | null>;
  /** Desktop: write the project folder (project.json, pyramid, masks, spots). */
  saveProject(project: Project, extras: Record<string, string>): Promise<string | null>;
}

let instance: Promise<Core> | null = null;

export { isHosted };

/**
 * True only in grainatlas's own desktop app, where its Tauri commands exist.
 * Ask the bridge first: UPb Studio hosts this page in a Tauri webview too
 * (`__TAURI_INTERNALS__` is present there), but only the bridge commands are
 * allowed in it, so a hosted page uses the wasm core and the web file paths.
 */
export function isDesktop(): boolean {
  if (typeof window === 'undefined' || isHosted()) return false;
  return '__TAURI_INTERNALS__' in window;
}

export function getCore(): Promise<Core> {
  if (!instance) {
    instance = isDesktop() ? import('./tauri').then((m) => new m.TauriCore()) : import('./wasm').then((m) => new m.WasmCore());
  }
  return instance;
}
