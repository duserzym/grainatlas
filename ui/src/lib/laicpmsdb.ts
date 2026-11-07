// Links and hand-overs to laicpmsdb (the sessions database), through the UPb
// Studio shell. Not hosted, the bridge's open() resolves false.

import type { Spot } from './core/types';

/** The laicpmsdb link for a spot: its acquisition when known, else a search by label (= spot name). */
export function spotLink(s: Pick<Spot, 'name' | 'acquisition_id'>): string {
  const acq = s.acquisition_id?.trim();
  if (acq) return `laicpmsdb://acquisition/${encodeURIComponent(acq)}`;
  return `laicpmsdb://search?label=${encodeURIComponent(s.name)}`;
}

/** Body of laicpmsdb's `POST /v1/mounts/{mount_id}/spots` (mount id goes in the path). */
export interface MountSpots {
  project_id: string;
  sample: string;
  spots: { id: string; name: string; grain: string; area: string; x_um: number; y_um: number; diameter_um: number; kind: 'spot' }[];
}

/**
 * The point spots of one sample in mosaic micrometres (y down, origin at the
 * mosaic's top-left). Lines and rasters are not part of the endpoint yet and
 * are left out; `skipped` counts them.
 */
export function mountSpots(projectId: string, sample: string, spots: Spot[]): { body: MountSpots; skipped: number } {
  const points = spots.filter((s) => s.kind === 'spot');
  const round = (v: number) => Math.round(v * 1000) / 1000;
  return {
    body: {
      project_id: projectId,
      sample,
      spots: points.map((s) => ({
        id: s.id,
        name: s.name,
        grain: s.grain_number == null ? '' : `${s.area_id}/${s.grain_number}`,
        area: s.area_id,
        x_um: round(s.x_um),
        y_um: round(s.y_um),
        diameter_um: s.diameter_um,
        kind: 'spot',
      })),
    },
    skipped: spots.length - points.length,
  };
}

// ---- the laicpmsdb server: where it is, the token, and the calls grainatlas makes ----------
//
// Endpoints (upblab/docs/WORKFLOW.md v1 and laicpmsdb's schemas/openapi.json):
//   GET  /v1/mounts                         mount picker (MountOut list)
//   POST /v1/mounts/{id}/spots              spot sync (MountSpots body)
//   POST /v1/mounts/{id}/atlas              publish: multipart/form-data, field `bundle` = the zip
//   GET  /v1/mounts/{id}/results            results overlay (MountResults)
//   GET  /atlas/projects/<project_id>/<path> a published bundle, read-only (open from server)

/** The user's laicpmsdb settings on this computer (never in the project). */
export interface LinkSettings {
  /** Base URL of the laicpmsdb service, e.g. `http://localhost:8000`; empty = none. */
  server: string;
  /** Bearer token pasted when grainatlas runs on its own (hosted, the shell's token is used). */
  token: string;
  /** Keep the token across browser sessions (else it lasts until the tab closes). */
  rememberToken: boolean;
  /** Atlas tiles: 100 = lossless WebP, 90 to 99 = near-lossless. */
  quality: number;
  /** Where raw SEM packages live on the lab storage; the bundle records `<root>/<package name>`. */
  nasRoot: string;
}

const LINK_KEY = 'grainatlas.laicpmsdb';
const TOKEN_KEY = 'grainatlas.laicpmsdb.token';

export const DEFAULT_LINK: LinkSettings = { server: '', token: '', rememberToken: false, quality: 100, nasRoot: '' };

export function loadLink(): LinkSettings {
  const out = { ...DEFAULT_LINK };
  try {
    Object.assign(out, JSON.parse(localStorage.getItem(LINK_KEY) || '{}'));
    out.token = (out.rememberToken ? localStorage.getItem(TOKEN_KEY) : sessionStorage.getItem(TOKEN_KEY)) || '';
  } catch {
    /* storage may be unavailable */
  }
  return out;
}

export function saveLink(l: LinkSettings) {
  try {
    const { token, ...rest } = l;
    localStorage.setItem(LINK_KEY, JSON.stringify(rest));
    localStorage.removeItem(TOKEN_KEY);
    sessionStorage.removeItem(TOKEN_KEY);
    if (token) (l.rememberToken ? localStorage : sessionStorage).setItem(TOKEN_KEY, token);
  } catch {
    /* ignore */
  }
}

/** `http://host:8000/` → `http://host:8000` (no trailing slash); null for an empty or non-http value. */
export function normalizeServer(s: string | null | undefined): string | null {
  const t = (s ?? '').trim().replace(/\/+$/, '');
  if (!t) return null;
  try {
    const u = new URL(t);
    return u.protocol === 'http:' || u.protocol === 'https:' ? t : null;
  } catch {
    return null;
  }
}

/**
 * The laicpmsdb server: `?server=` of this page load, else Settings, else this page's own
 * origin when grainatlas is served by laicpmsdb itself (under `/atlas/`).
 */
export function resolveServer(override: string | null, configured: string): string | null {
  const o = normalizeServer(override) ?? normalizeServer(configured);
  if (o) return o;
  try {
    if ((location.protocol === 'http:' || location.protocol === 'https:') && /\/atlas\//.test(location.pathname)) return location.origin;
  } catch {
    /* no location */
  }
  return null;
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function failure(r: Response, what: string): Promise<ApiError> {
  let detail = '';
  try {
    const t = await r.text();
    try {
      const j = JSON.parse(t);
      detail = typeof j.detail === 'string' ? j.detail : JSON.stringify(j.detail ?? j);
    } catch {
      detail = t.slice(0, 200);
    }
  } catch {
    /* no body */
  }
  const hint = r.status === 401 ? ' (sign in, or paste a token in Settings)' : r.status === 403 ? ' (no permission on this mount)' : '';
  return new ApiError(r.status, `${what}: ${r.status} ${r.statusText}${hint}${detail ? ` — ${detail}` : ''}`);
}

function headers(token: string | null, extra: Record<string, string> = {}): Record<string, string> {
  return token ? { ...extra, Authorization: `Bearer ${token}` } : extra;
}

async function call<T>(base: string, path: string, token: string | null, init: RequestInit, what: string): Promise<T> {
  let r: Response;
  try {
    r = await fetch(`${base}${path}`, { ...init, headers: { ...headers(token), ...(init.headers as Record<string, string> | undefined) } });
  } catch (e) {
    throw new ApiError(0, `${what}: cannot reach ${base} (${e instanceof Error ? e.message : e})`);
  }
  if (!r.ok) throw await failure(r, what);
  return (await r.json()) as T;
}

/** A mount as `GET /v1/mounts` lists it. */
export interface MountOut {
  id: string;
  label: string;
  mount_type: string;
  grain_count: number;
  spot_count: number;
  can_edit: boolean;
}

export function listMounts(base: string, token: string | null): Promise<MountOut[]> {
  return call(base, '/v1/mounts', token, { method: 'GET' }, 'Listing mounts');
}

/** laicpmsdb's answer to a spot upload (SpotImportOut). */
export interface SpotImportOut {
  mount_id: string;
  project_id: string;
  sample_id: string | null;
  sample_name: string | null;
  created: number;
  updated: number;
  unchanged: number;
  grains_created: number;
  acquisitions: { id: string; label: string; session_id: string; session_key: string; spot_id: string; atlas_spot_id: string; linked: boolean }[];
}

export function postSpots(base: string, token: string | null, mountId: string, body: MountSpots): Promise<SpotImportOut> {
  return call(base, `/v1/mounts/${encodeURIComponent(mountId)}/spots`, token, { method: 'POST', body: JSON.stringify(body), headers: { 'Content-Type': 'application/json' } }, 'Sending spots');
}

/** What grainatlas reads from the publish response; other fields are kept as they come. */
export interface AtlasPublishOut {
  mount_id?: string;
  project_id?: string;
  version?: number | null;
  sha256?: string;
  url?: string;
  [k: string]: unknown;
}

export function publishAtlas(base: string, token: string | null, mountId: string, projectId: string, zip: Uint8Array): Promise<AtlasPublishOut> {
  const form = new FormData();
  form.append('bundle', new Blob([zip as BlobPart], { type: 'application/zip' }), `${projectId}.atlas.zip`);
  return call(base, `/v1/mounts/${encodeURIComponent(mountId)}/atlas`, token, { method: 'POST', body: form }, 'Publishing the atlas');
}

/** URL of a file of a published bundle. */
export function atlasUrl(base: string, projectId: string, path = ''): string {
  return `${base}/atlas/projects/${encodeURIComponent(projectId)}/${path}`;
}

/** With no bearer token, laicpmsdb's viewer `access` token (read-only, that project's mount only) works too. */
export function getResults(base: string, token: string | null, mountId: string, access: string | null = null): Promise<import('./results').MountResults> {
  const q = !token && access ? `?access=${encodeURIComponent(access)}` : '';
  return call(base, `/v1/mounts/${encodeURIComponent(mountId)}/results${q}`, token, { method: 'GET' }, 'Mount results');
}

/** laicpmsdb's own page for an acquisition (when the shell cannot route a laicpmsdb:// link). */
export function acquisitionPage(base: string, id: string): string {
  return `${base}/app/#/acquisitions/${encodeURIComponent(id)}`;
}

/** All project areas, grouped by stored sample (falling back to the area's sample/name). */
export function mountSpotDocuments(project: import('./core/types').Project, spots: Spot[]): { documents: MountSpots[]; skipped: number } {
  const groups = new Map<string, Spot[]>();
  for (const s of spots) {
    const area = project.areas.find((a) => a.id === s.area_id);
    const sample = s.sample || area?.sample || area?.name || '';
    const list = groups.get(sample) ?? [];
    list.push(s);
    groups.set(sample, list);
  }
  let skipped = 0;
  const documents = [...groups].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([sample, ss]) => {
    const r = mountSpots(project.uuid, sample, ss);
    skipped += r.skipped;
    return r.body;
  }).filter((d) => d.spots.length);
  return { documents, skipped };
}

// Instrument labels belong to acquisitions, independently of physical spot links.
const acquisitionLabels = new Map<string, Promise<string[]>>();
const labelsKey = (base: string, mountId: string) => JSON.stringify([base.replace(/\/+$/, ''), mountId.toLowerCase()]);

export function clearMountAcquisitionLabels(base: string, mountId: string) {
  acquisitionLabels.delete(labelsKey(base, mountId));
}

/** Visible sessions that measured this mount, with at most four session requests in flight. */
export function mountAcquisitionLabels(base: string, token: string | null, mountId: string): Promise<string[]> {
  base = base.replace(/\/+$/, '');
  const key = labelsKey(base, mountId);
  const cached = acquisitionLabels.get(key);
  if (cached) return cached;
  const request = (async () => {
    const sessions = await call<{ id: string }[]>(base, '/v1/sessions', token, { method: 'GET' }, 'Listing acquisition sessions');
    const labels: string[] = [];
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(4, sessions.length) }, async () => {
      while (next < sessions.length) {
        const session = sessions[next++];
        const mounts = await call<MountOut[]>(base, `/v1/mounts?session=${encodeURIComponent(session.id)}`, token, { method: 'GET' }, 'Listing session mounts');
        if (!mounts.some((m) => m.id.toLowerCase() === mountId.toLowerCase())) continue;
        const acquisitions = await call<{ instrument_label: string; reference_material_name: string }[]>(base, `/v1/sessions/${encodeURIComponent(session.id)}/acquisitions`, token, { method: 'GET' }, 'Loading acquisition labels');
        labels.push(...acquisitions.filter((a) => !a.reference_material_name?.trim()).map((a) => a.instrument_label).filter((l) => l?.trim()));
      }
    }));
    return [...new Set(labels)].sort();
  })();
  acquisitionLabels.set(key, request);
  void request.catch(() => { if (acquisitionLabels.get(key) === request) acquisitionLabels.delete(key); });
  return request;
}
