// Types mirrored from grainmap-core (serde JSON).

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

export interface PackageInfo {
  reader: string;
  name: string;
  areas: AreaMeta[];
  meta: Record<string, string>;
  navigation: { file: string; top_left_nm: [number, number]; bottom_right_nm: [number, number] } | null;
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

export interface ToneParams {
  low_pct: number; high_pct: number; black: number; white: number; gamma: number;
  clahe: boolean; clahe_tile: number; clahe_clip: number;
}

export interface AreaParams {
  stitch: { coarse_factor: number; min_psr: number; max_drift_um: number; outlier_px: number; refine_full: boolean };
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

export interface SegmentParams {
  polarity: 'bright' | 'dark';
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
}

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
  comment: string;
  created: string;
  created_by: string;
}

export interface Label { id: string; area_id: string; x_um: number; y_um: number; text: string }

export interface Settings {
  naming_pattern: string;
  spot_presets_um: number[];
  default_diameter_um: number;
  user: string;
  theme: 'dark' | 'light';
  snap: boolean;
}

export interface Project {
  schema_version: number;
  uuid: string;
  name: string;
  created: string;
  package: { path: string; reader: string; name: string; meta: Record<string, string> };
  laicpmsdb: { mount_id: string | null };
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

export interface Progress { stage: string; fraction: number }
