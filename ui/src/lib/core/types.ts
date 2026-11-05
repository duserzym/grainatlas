// Types mirrored from grainatlas-core (serde JSON).

export interface TileMeta {
  index: number;
  file: string;
  w: number;
  h: number;
  pos_um: [number, number];
  stage_nm: [number, number] | null;
  pixel_size_nm: number;
  grid: [number, number] | null;
  meta: Record<string, string>;
}

export interface AreaMeta {
  id: string;
  name: string;
  grid: [number, number] | null;
  pixel_size_nm: number;
  field_um: [number, number] | null;
  pitch_um: [number, number] | null;
  stage_sign: [number, number];
  tiles: TileMeta[];
  meta: Record<string, string>;
}

/** How a package was opened: an instrument package folder, one area folder alone, or loose images with a stated scale. */
export type OpenKind = 'package' | 'area' | 'loose';

export interface PackageInfo {
  reader: string;
  kind: OpenKind;
  name: string;
  areas: AreaMeta[];
  meta: Record<string, string>;
  navigation: { file: string; top_left_nm: [number, number]; bottom_right_nm: [number, number] } | null;
}

export interface ImageEntry { file: string; w: number; h: number }

/** Result of opening a folder or files: a package, or images that need a scale first. */
export type OpenResult = { status: 'opened'; info: PackageInfo } | { status: 'needs_scale'; name: string; images: ImageEntry[] };

export type GridOrder = 'rows' | 'snake' | 'columns';

/** Scale and arrangement for loose images (core `readers::loose::LooseSpec`). */
export interface LooseSpec {
  um_per_px: number;
  field_width_um?: number;
  grid?: { cols: number; rows: number; overlap_pct: number; order: GridOrder; name: string };
  files: string[];
}

export interface LevelInfo { level: number; w: number; h: number; cols: number; rows: number; scale: number }

export interface MosaicJson {
  w: number;
  h: number;
  um_per_px: number;
  origin_um: [number, number];
  tile_size: number;
  format: string;
  levels: LevelInfo[];
  sha256: string;
}

export interface PairResult {
  a: number; b: number; dir: 'h' | 'v';
  prior: [number, number];
  measured: [number, number] | null;
  psr: number; used: boolean; residual: number;
}

export interface TilePlacement {
  index: number; prior: [number, number]; pos: [number, number];
  x: number; y: number; w: number; h: number; drift: [number, number];
}

export interface Placement {
  tiles: TilePlacement[]; pairs: PairResult[];
  width: number; height: number; rms_residual: number; max_residual: number;
}

export interface TileOverride { index: number; gain: number | null; offset: number | null; no_background?: boolean }

/** Per-grain tone normalisation (core `photometric::grain_norm::GrainNorm`). */
export interface GrainNorm { enabled: boolean; strength: number; reference: string | null; feather_um: number }

export interface ToneParams {
  low_pct: number; high_pct: number; black: number; white: number; gamma: number;
  clahe: boolean; clahe_tile: number; clahe_clip: number;
  grain_norm: GrainNorm;
}

export interface AreaParams {
  stitch: { coarse_factor: number; min_psr: number; max_drift_um: number; outlier_px: number; refine_full: boolean; consensus_px: number };
  photometric: {
    model: 'affine' | 'gain' | 'none';
    regularisation: number;
    background: boolean;
    background_params: { model: 'poly2' | 'smooth'; cell: number; smooth_cells: number; threshold: number | null; dilate: number; edge_steps: boolean };
    overrides: TileOverride[];
  };
  blend: { mode: 'multiband' | 'feather'; seam_factor: number; levels: number };
  tone: ToneParams;
  segment: SegmentParams;
}

export interface OutlinePad { margin_um: number; smooth: boolean }

export interface SegmentParams {
  /** 'both': bright grains plus grains brighter or darker than the local epoxy (CL-dark zircons). */
  polarity: 'bright' | 'dark' | 'both';
  method: 'background' | 'otsu';
  noise_k: number;
  contrast_frac: number;
  threshold: number | null;
  smooth_px: number;
  open_px: number;
  close_px: number;
  min_size_um: number;
  split: boolean;
  split_h: number;
  neck_ratio: number;
  grabcut: boolean;
  simplify_px: number;
  row_tolerance: number;
  work_scale: number;
  dark_rel: number;
  background_window_um: number;
  /** Artifact classifier: flag (never delete) bubbles and their halos, seam slivers, tile blocks, specks. */
  flag_bubbles: boolean;
  flag_slivers: boolean;
  flag_blocks: boolean;
  flag_specks: boolean;
  /** Grain size the artifact sizes scale with (µm); 0 = the area's median grain size. */
  reference_size_um: number;
  /** 0 = no limit */
  max_size_um: number;
}

export interface PhotometricResult {
  auto_gain: number[]; auto_offset: number[]; gain: number[]; offset: number[];
  background_median: number; background_range: number[];
  overlap_rms_before: number; overlap_rms_after: number;
}

export interface StitchSummary {
  mosaic: MosaicJson;
  placement: Placement;
  photometric: PhotometricResult;
  timings: [string, number][];
}

export interface Grain {
  id: string;
  number: number;
  /** Polygon in mosaic micrometres (closed implicitly). */
  polygon_um: [number, number][];
  bbox_um: [number, number, number, number];
  centroid_um: [number, number];
  area_um2: number;
  eq_diameter_um: number;
  major_um: number;
  minor_um: number;
  orientation_deg: number;
  mean_intensity: number;
  manual: boolean;
  scale_bar_um: number;
  /** Not a grain, and why (shown dimmed, unnumbered, left off sheets and spots). */
  artifact?: ArtifactReason;
  /** The user kept this detection as a grain although it was flagged. */
  kept?: boolean;
  /** The outline was drawn or painted by hand; Find again keeps it as it is. */
  drawn?: boolean;
}

/** What a drawn outline or brush stroke does: a new grain, or grow or cut the grain. */
export type EditOp = 'new' | 'add' | 'cut';

export type ArtifactReason = 'bubble' | 'halo' | 'sliver' | 'block' | 'speck' | 'manual';

export const ARTIFACT_LABELS: Record<ArtifactReason, string> = {
  bubble: 'epoxy bubble or pit',
  halo: 'bubble halo or rim',
  sliver: 'tile-seam sliver',
  block: 'exposure-step tile block',
  speck: 'speck',
  manual: 'marked by you',
};

export type SpotKind = 'spot' | 'line' | 'raster';

export interface Spot {
  id: string;
  area_id: string;
  name: string;
  sample: string;
  grain_id: string | null;
  grain_number: number | null;
  n: number;
  x_um: number;
  y_um: number;
  /** End point for lines, opposite corner offset for rasters. */
  x2_um: number | null;
  y2_um: number | null;
  diameter_um: number;
  kind: SpotKind;
  /** laicpmsdb acquisition this spot was measured as (UI-held; set by hand or by a later import). */
  acquisition_id?: string | null;
  comment: string;
  created: string;
  created_by: string;
}

export interface Label { id: string; area_id: string; x_um: number; y_um: number; text: string }

export interface Settings {
  naming_pattern: string;
  spot_presets_um: number[];
  default_diameter_um: number;
  spot_cursor_colour: string;
  user: string;
  /** Unused by the UI (the theme is the viewer's preference, not the project's). */
  theme?: string;
  snap: boolean;
  outline_margin_um: number;
  outline_smooth: boolean;
}

export interface Project {
  schema_version: number;
  uuid: string;
  name: string;
  created: string;
  package: { path: string; reader: string; name: string; kind: OpenKind; loose?: LooseSpec; files?: string[]; meta: Record<string, string> };
  laicpmsdb: { mount_id: string | null; published?: Published[] };
  /** Laser stage registration per area (phase 11). */
  laser?: { areas: Record<string, AreaRegistration> };
  areas: AreaRecord[];
  settings: Settings;
  [k: string]: unknown;
}

export interface AreaRecord {
  uuid: string;
  id: string;
  name: string;
  sample: string;
  pixel_size_nm: number;
  stage_sign: [number, number];
  tiles: { file: string; sha256: string; pos_um: [number, number]; stage_nm: [number, number] | null; pixel_size_nm: number; w: number; h: number }[];
  params: AreaParams;
  placement: Placement | null;
  photometric: PhotometricResult | null;
  mosaic: MosaicJson | null;
  area_meta: Record<string, string>;
}

export interface Paper { name: string; w_mm: number; h_mm: number }

export interface LayoutParams {
  paper: string;
  custom_w_mm: number;
  custom_h_mm: number;
  landscape: boolean;
  margin_mm: number;
  /** micrometres of sample per millimetre of paper; 0 = fit */
  um_per_mm: number;
  items: 'grains' | 'areas';
  area_ids: string[];
  grain_margin_um: number;
  show_spots: boolean;
  show_labels: boolean;
  show_outlines: boolean;
  cut_marks: boolean;
  title: string;
  dpi: number;
  by_number: boolean;
}

export interface LayoutResult { pages: string[]; um_per_mm: number; items: number; paper_mm: [number, number] }

/** Export-only enhancement: NLM denoise, unsharp mask, background strip. */
export interface EnhanceParams {
  denoise: boolean;
  denoise_h: number;
  sharpen: boolean;
  sharpen_radius: number;
  sharpen_amount: number;
  strip: boolean;
  /** 0 black … 255 white */
  strip_grey: number;
  strip_margin_um: number;
}

/** Print sheets: every owned grain of the mount at a fixed scale. */
export interface SheetParams {
  paper: string;
  landscape: boolean;
  margin_mm: number;
  um_per_mm: number;
  pad_um: number;
  gap_break_um: number;
  min_grain_um: number;
  max_grain_um: number;
  spot_sizes_um: number[];
  enhance: EnhanceParams;
  title: string;
  lab: string;
  project: string;
  registration_file: string;
  printed: string;
}

export interface BandRecord {
  sheet: number;
  sample: string;
  area: string;
  row: number;
  grains: { id: string; number: number }[];
  sheet_rect_mm: [number, number, number, number];
  mosaic_rect_um: [number, number, number, number];
  background: string;
}

/** The `print_sheet` frame: paper sheet ↔ mosaic (not the laser stage registration). */
export interface PrintRegistration {
  frame: 'print_sheet';
  note: string;
  package: string;
  project: string;
  paper: string;
  paper_mm: [number, number];
  um_per_mm: number;
  sheet_origin: string;
  mapping: string;
  bands: BandRecord[];
}

export interface SheetStats { sheets: number; bands: number; grains: number; duplicates: number; filtered: number; artifacts: number; stripped: number; strip_refused: number; grains_cut: number; grains_twice: number }

/** One detection and the area that owns the grain. */
export interface GrainOwner {
  area_id: string;
  grain_id: string;
  owner_area: string;
  /** The owner's detection of the same grain, when this one is a duplicate. */
  duplicate_of?: string;
  cut: boolean;
  interior_um: number;
  stage_nm?: [number, number];
}

export interface Ownership { grains: GrainOwner[]; samples: { sample: string; areas: string[] }[] }

export interface SheetResult { pages: string[]; registration: PrintRegistration; stats: SheetStats; ownership: Ownership }

export interface Progress { stage: string; fraction: number }

/** One publish of the atlas bundle to laicpmsdb (recorded in project.json). */
export interface Published {
  server: string;
  mount_id: string | null;
  version: number | null;
  /** sha256 of atlas.json (which lists the hash of every other file). */
  atlas_sha256: string;
  zip_sha256: string;
  at: string;
}

/** How the core writes an atlas bundle. */
export interface BundleOptions {
  /** 100 = lossless WebP tiles, 90 to 99 = near-lossless. */
  quality: number;
  exported: string;
  nas_path: string | null;
}

/** A fiducial: a spot in mosaic µm and the laser stage reading there. */
export interface Fiducial {
  spot_id: string | null;
  name: string;
  x_um: number;
  y_um: number;
  x_stage: number | null;
  y_stage: number | null;
}

/** Affine mosaic µm → laser stage ([a, b, c, d, e, f]: x' = a x + c y + e, y' = b x + d y + f). */
export interface LaserFit {
  matrix: [number, number, number, number, number, number];
  residuals_um: number[];
  rms_um: number;
  max_um: number;
  n: number;
  exact: boolean;
  warn: boolean;
  scale: [number, number];
  rotation_deg: number;
  mirrored: boolean;
}

export interface AreaRegistration {
  units: 'mm' | 'um';
  adapter: string;
  fiducials: Fiducial[];
  fit: LaserFit | null;
}

export interface LaserAdapterInfo { id: string; label: string; extension: string }
