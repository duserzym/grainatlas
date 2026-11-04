// Results overlay (WORKFLOW.md section 5): what laicpmsdb holds for the spots of a mount, from
// `GET /v1/mounts/{id}/results` (the preferred run of each session by default). Shown in the
// spot panel and as a coloured layer; never written into the project.

import type { Spot } from './core/types';
import { authToken } from './atlas';
import { getResults } from './laicpmsdb';
import { app } from './store.svelte';

export interface Value {
  metric: string;
  value: number;
  sigma2?: number | null;
  unit?: string | null;
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

/** Results by grainatlas spot id: by `atlas_spot_id`, else by label = spot name (case-insensitive). */
export function matchResults(spots: Spot[], r: MountResults): Record<string, SpotResult> {
  const byId = new Map<string, SpotResult>();
  const byLabel = new Map<string, SpotResult>();
  for (const s of r.spots ?? []) {
    if (s.atlas_spot_id) byId.set(s.atlas_spot_id.toLowerCase(), s);
    else if (s.label) byLabel.set(s.label.toLowerCase(), s);
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

// ---- fetching (on demand; kept in memory only) ----------------------------------------------

/** Fetches the linked mount's results again. Errors stay in `app.results.error` (the tab shows them). */
export async function refreshResults(): Promise<void> {
  const server = app.server;
  const mount = app.project?.laicpmsdb.mount_id ?? null;
  if (!server || !mount) {
    app.results = null;
    return;
  }
  const key = `${server}|${mount}`;
  app.results = { key, data: app.results?.key === key ? app.results.data : null, at: app.results?.at ?? null, error: null, loading: true };
  try {
    const data = await getResults(server, await authToken(), mount);
    if (app.results?.key === key) app.results = { key, data, at: new Date().toISOString(), error: null, loading: false };
  } catch (e) {
    if (app.results?.key === key) app.results = { ...app.results, error: e instanceof Error ? e.message : String(e), loading: false };
  }
}
