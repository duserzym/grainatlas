// Results overlay (WORKFLOW.md section 5): what laicpmsdb holds for the spots of a mount, from
// `GET /v1/mounts/{id}/results` (the preferred run of each session by default). Shown in the
// spot panel and as a coloured layer; never written into the project. Pure functions only;
// fetching is in results-load.ts.

import type { Spot } from './core/types';

export interface Value {
  metric: string;
  value: number;
  /** Absolute uncertainty (2σ, 2SE or 95% CI: `utype`). */
  sigma2?: number | null;
  unit?: string | null;
  /** Kind of uncertainty as laicpmsdb names it ("2se", "95ci"); the date's is 2σ when absent. */
  utype?: string | null;
  /** Dates: the method ("206Pb/238U") and whether it passes laicpmsdb's filters. */
  method?: string | null;
  passes?: boolean | null;
}

/** One spot's results as laicpmsdb returns them. */
export interface SpotResult {
  atlas_spot_id: string | null;
  physical_spot_id?: string | null;
  label: string;
  acquisition_id: string | null;
  session_id?: string | null;
  session_key?: string | null;
  run_id?: string | null;
  is_preferred?: boolean | null;
  /** The preferred date with its absolute 2σ. */
  date: Value | null;
  ratios: Value[];
  concentrations: Value[];
  flags: string[];
}

export interface MountResults {
  mount_id: string;
  run?: string;
  generated?: string;
  spots: SpotResult[];
}

type RawValue = { value?: unknown; uncertainty?: unknown; sigma2?: unknown; two_sigma?: unknown; uncertainty_type?: unknown; unit?: unknown; metric?: unknown; method?: unknown; passes?: unknown };
const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown) => (typeof v === 'string' ? v : null);

function values(raw: unknown): Value[] {
  // laicpmsdb sends maps keyed by name ({"206Pb/238U": {value, uncertainty, uncertainty_type, unit}});
  // lists of {metric, value, sigma2, unit} are read too.
  const entries: [string, RawValue][] = Array.isArray(raw)
    ? (raw as RawValue[]).map((v) => [String(v.metric ?? ''), v])
    : raw && typeof raw === 'object'
      ? (Object.entries(raw) as [string, RawValue][])
      : [];
  return entries
    .filter(([, v]) => v && num(v.value) != null)
    .map(([metric, v]) => ({ metric, value: num(v.value)!, sigma2: num(v.uncertainty) ?? num(v.sigma2) ?? num(v.two_sigma), unit: str(v.unit) || null, utype: str(v.uncertainty_type) }));
}

/** One entry of `GET /v1/mounts/{id}/results` in the form the panel and the layer use. */
export function normalize(e: Record<string, unknown>): SpotResult {
  const d = e.date as RawValue | null | undefined;
  const date: Value | null =
    d && num(d.value) != null
      ? { metric: str(d.metric) ?? 'date', value: num(d.value)!, sigma2: num(d.two_sigma) ?? num(d.sigma2) ?? num(d.uncertainty), unit: str(d.unit), utype: str(d.uncertainty_type), method: str(d.method), passes: typeof d.passes === 'boolean' ? d.passes : null }
      : null;
  const ratios = values(e.ratios);
  if (num(e.th_u) != null) ratios.push({ metric: 'Th/U', value: num(e.th_u)!, unit: null });
  if (num(e.discordance_percent) != null) ratios.push({ metric: 'discordance', value: num(e.discordance_percent)!, unit: '%' });
  return {
    atlas_spot_id: str(e.atlas_spot_id),
    physical_spot_id: str(e.physical_spot_id) ?? str(e.spot_id),
    label: str(e.label) ?? '',
    acquisition_id: str(e.acquisition_id),
    session_id: str(e.session_id),
    session_key: str(e.session_key),
    run_id: str(e.run_id),
    is_preferred: typeof e.is_preferred === 'boolean' ? e.is_preferred : null,
    date,
    ratios,
    concentrations: values(e.concentrations),
    flags: Array.isArray(e.flags) ? e.flags.map(String) : [],
  };
}

const hasValues = (r: SpotResult) => !!r.date || r.ratios.length > 0 || r.concentrations.length > 0;
/** Several acquisitions at one spot: the one with a date from a preferred run, else with a date, else any. */
const rank = (r: SpotResult) => (r.date ? 2 : 0) + (r.is_preferred === false ? 0 : 1);

/** Results by grainatlas spot id: by `atlas_spot_id`, else by label = spot name (case-insensitive).
 * Spots laicpmsdb lists without values (not measured yet) are left out. */
export function matchResults(spots: Spot[], r: MountResults): Record<string, SpotResult> {
  const byId = new Map<string, SpotResult>();
  const byLabel = new Map<string, SpotResult>();
  const keep = (m: Map<string, SpotResult>, k: string, s: SpotResult) => {
    const old = m.get(k);
    if (!old || rank(s) > rank(old)) m.set(k, s);
  };
  for (const s of r.spots ?? []) {
    if (!hasValues(s)) continue;
    if (s.atlas_spot_id) keep(byId, s.atlas_spot_id.toLowerCase(), s);
    else if (s.label) keep(byLabel, s.label.toLowerCase(), s);
  }
  const out: Record<string, SpotResult> = {};
  for (const sp of spots) {
    const hit = byId.get(sp.id.toLowerCase()) ?? byLabel.get(sp.name.toLowerCase());
    if (hit) out[sp.id] = hit;
  }
  return out;
}

/** Metric codes present, for "colour by": the date first, then ratios and concentrations. */
export function metricsOf(results: SpotResult[]): string[] {
  const set = new Set<string>();
  for (const r of results) for (const v of [...r.ratios, ...r.concentrations]) set.add(v.metric);
  return ['date', ...[...set].sort()];
}

/** The number a spot is coloured by: its preferred date, or a ratio or concentration. */
export function valueOf(r: SpotResult, metric: string): Value | null {
  if (metric === 'date') return r.date;
  return [...r.ratios, ...r.concentrations].find((v) => v.metric === metric) ?? null;
}

// Viridis at five stops: perceptually even and readable by colour-blind viewers, bright enough
// on the dark image surface.
const RAMP: [number, number, number][] = [
  [68, 1, 84],
  [59, 82, 139],
  [33, 145, 140],
  [94, 201, 98],
  [253, 231, 37],
];

export function rampColor(t: number): string {
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 0)) * (RAMP.length - 1);
  const i = Math.min(RAMP.length - 2, Math.floor(x));
  const f = x - i;
  const c = RAMP[i].map((a, k) => Math.round(a + (RAMP[i + 1][k] - a) * f));
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

export const RAMP_CSS = `linear-gradient(90deg, ${RAMP.map((c) => `rgb(${c.join(',')})`).join(', ')})`;

export function fmtValue(v: Value | null, digits?: number): string {
  if (!v || !Number.isFinite(v.value)) return '–';
  const d = digits ?? (Math.abs(v.value) >= 100 ? 1 : Math.abs(v.value) >= 1 ? 3 : 5);
  const s = v.sigma2 != null && Number.isFinite(v.sigma2) ? ` ± ${v.sigma2.toFixed(d)}` : '';
  return `${v.value.toFixed(d)}${s}${v.unit ? ` ${v.unit}` : ''}`;
}
