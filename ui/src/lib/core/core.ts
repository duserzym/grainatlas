// The one interface the UI talks to. Two implementations: the wasm core in a
// Web Worker (static web page, and the page hosted by UPb Studio) and the native
// core through grainatlas's own Tauri commands (desktop app).

import { isHosted } from '@lab/bridge';

import type {
  AreaParams, EditOp, AreaRegistration, BundleOptions, Fiducial, Grain, LaserAdapterInfo, LaserFit, LayoutParams, Published, LayoutResult, LooseSpec, MosaicJson, OpenResult, PackageInfo, Progress, Spot, Label, StitchSummary, ToneParams, SegmentParams, Project,
  Ownership, SheetParams, SheetResult,
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
  /** Drops the open package and project: an empty session for a project without its package. */
  blank(name: string): Promise<void>;
  loadProject(json: string): Promise<void>;
  projectJson(): Promise<string>;
  navigation(): Promise<RawImage | null>;
  tileThumb(area: number, tile: number, max: number): Promise<RawImage>;
  stitch(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary>;
  /** New parameters on a stitched area (re-tone, re-correct from the solved placement, or re-stitch). */
  recompute(areaId: string, params: AreaParams, onProgress?: (p: Progress) => void): Promise<StitchSummary>;
  /** The whole mosaic at pyramid `level` with `params`, for a live preview while a control moves. */
  preview(areaId: string, params: AreaParams, level: number): Promise<RawImage>;
  /** One-click Auto: `params` with background leveling on and an automatic tone (stretch, CLAHE). */
  autoParams(areaId: string, params: AreaParams): Promise<AreaParams>;
  /** The UI's grains of an area, for per-grain tone normalisation (applies on the next recompute). */
  setGrains(areaId: string, grains: Grain[]): Promise<void>;
  /** The core's default parameters (Reset). */
  defaultParams(): Promise<AreaParams>;
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
  /** A drawn outline (um, closed) snapped to the mosaic's edges within `widthUm` of it. */
  snapOutline(areaId: string, path: [number, number][], widthUm: number, contrast: number): Promise<[number, number][]>;
  /** A grain from a drawn outline: new, or `base` grown or cut by it; null when nothing is left. */
  outlineGrain(areaId: string, polygon: [number, number][], base: Grain | null, op: EditOp): Promise<Grain | null>;
  /** A grain from a brush stroke (um, diameter `sizeUm`) grown to the nearby edges. */
  brushGrain(areaId: string, stroke: [number, number][], sizeUm: number, base: Grain | null, op: EditOp): Promise<Grain | null>;
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
  /** The atlas bundle (WORKFLOW.md 3.1) of the full project as a zip, every stitched area included. */
  atlasZip(project: Project, options: BundleOptions): Promise<Uint8Array>;
  /** Desktop: save the atlas bundle as a `<project_id>.atlas/` folder or a zip (native dialogs). */
  saveAtlas(project: Project, options: BundleOptions, zip: boolean): Promise<string | null>;
  /** Laser registration from fiducials (stage units "mm" or "um"); throws when fewer than 3 or collinear. */
  laserFit(fiducials: Fiducial[], units: string): Promise<LaserFit>;
  laserAdapters(): Promise<LaserAdapterInfo[]>;
  /** Stores an area's laser registration in the project (null removes it). */
  setLaser(areaId: string, reg: AreaRegistration | null): Promise<void>;
  /** The area's spot list in the laser software's format. */
  laserExport(spots: Spot[], areaId: string, adapter?: string): Promise<string>;
  /** Records a publish in the project's laicpmsdb link. */
  recordPublished(p: Published): Promise<void>;
  /** Desktop: the window back to its default size, centred (Reset layout); nothing in a browser. */
  resetWindow(): Promise<void>;
  /** The project file `<name>.gaproj` (the project record, no pixels): desktop writes it to
   * `path`, next to the package, or where a dialog says (`ask`); the web downloads it. */
  saveProjectFile(project: Project, path: string | null, ask: boolean): Promise<string | null>;
  /** Desktop: write the project folder (project.json, pyramid, masks, spots): into `dir` (the
   * folder it was opened from) when given, else `<name>.grainatlas` next to the package. */
  saveProject(project: Project, extras: Record<string, string>, dir?: string | null): Promise<string | null>;
  /** Which area owns each detection (a grain seen in overlapping areas is kept once). */
  ownership(grains: Record<string, Grain[]>): Promise<Ownership>;
  /** Print sheets of the mount: pages (SVG), the print_sheet registration, stats and ownership. */
  printSheets(params: SheetParams, grains: Record<string, Grain[]>, tone: Record<string, ToneParams>): Promise<SheetResult>;
  /** The tab closed: free the session (desktop) or the worker (web). */
  dispose(): Promise<void>;
}

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

/** The core of one tab: a native session keyed by `tab` in the desktop app, its own wasm worker on the web. */
export function newCore(tab: number): Promise<Core> {
  return isDesktop() ? import('./tauri').then((m) => new m.TauriCore(tab)) : import('./wasm').then((m) => new m.WasmCore());
}
