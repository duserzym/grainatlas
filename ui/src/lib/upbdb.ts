import { spotDiameter } from './spot-size.ts';
import { fetchWithDeadline, databaseOfflineMessage } from './http.ts';
import { idempotentWrite } from './idempotency.ts';
import { serverFetch, ServerReadOnlyError } from './oversight.ts';
import { canWrite, requireWrite } from './presentation.ts';
import { errorSentence } from './sync-api.ts';
// Links and hand-overs to UPbDB (the sessions database), through the UPb
// Studio shell. Not hosted, the bridge's open() resolves false.

import type { Grain, Spot } from './core/types';
import type { SyncSpot } from './sync';

/** The UPbDB link for a spot: its acquisition when known, else a search by label (= spot name). */
export function spotLink(s: Pick<Spot, 'name' | 'acquisition_id'>): string {
  const acq = s.acquisition_id?.trim();
  if (acq) return `upbdb://acquisition/${encodeURIComponent(acq)}`;
  return `upbdb://search?label=${encodeURIComponent(s.name)}`;
}

/** Body of UPbDB's `POST /v1/mounts/{mount_id}/spots` (mount id goes in the path). */
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
export function mountSpots(projectId: string, sample: string, spots: Spot[], grains: Record<string, Grain[]> = {}): { body: MountSpots; skipped: number } {
  if (spots.some(s => s.kind === 'spot' && spotDiameter(s) === null)) throw new Error('Some spots have no size. Use Fix sizes before exporting.');
  const points = spots.filter((s) => s.kind === 'spot');
  const round = (v: number) => Math.round(v * 1000) / 1000;
  return {
    body: {
      project_id: projectId,
      sample,
      spots: points.map((s) => ({
        id: s.id,
        name: s.name,
        grain: (() => { const g = grains[s.area_id]?.find(g => g.id === s.grain_id); return g?.label ?? (g ? `${s.area_id}/${g.number}` : s.grain_id ?? (s.grain_number == null ? '' : `${s.area_id}/${s.grain_number}`)); })(),
        area: s.area_id,
        x_um: round(s.x_um),
        y_um: round(s.y_um),
        diameter_um: spotDiameter(s)!,
        kind: 'spot',
      })),
    },
    skipped: spots.length - points.length,
  };
}

// ---- the UPbDB server: where it is, the token, and the calls grainatlas makes ----------
//
// Endpoints (upblab/docs/WORKFLOW.md v1 and UPbDB's schemas/openapi.json):
//   GET  /v1/mounts                         mount picker (MountOut list)
//   POST /v1/mounts                         create a mount owned by the signed-in user
//   POST /v1/mounts/{id}/atlas              publish: multipart/form-data, field `bundle` = the zip;
//                                           writes no spot rows (spots go by sync), answers spot_check
//   GET  /v1/mounts/{id}/results            results overlay (MountResults)
//   GET  /atlas/projects/<project_id>/<path> a published bundle, read-only (open from server)

/** The user's UPbDB settings on this computer (never in the project). */
export interface LinkSettings {
  syncOnSave: boolean;
  syncAutomatically: boolean;
  syncPollSeconds: number;
  /** Base URL of the UPbDB service, e.g. `http://localhost:8000`; empty = none. */
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

export const LINK_KEY = 'grainatlas.upbdb';
const TOKEN_KEY = 'grainatlas.upbdb.token';

export const MIN_SYNC_POLL_SECONDS = 30;
export const MAX_SYNC_POLL_SECONDS = 3600;
export const DEFAULT_LINK: LinkSettings = {
  server: '', token: '', rememberToken: false, quality: 100, nasRoot: '',
  syncOnSave: true, syncAutomatically: true, syncPollSeconds: 120,
};

export function normalizeSyncPollSeconds(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.min(MAX_SYNC_POLL_SECONDS, Math.max(MIN_SYNC_POLL_SECONDS, Math.round(value)))
    : DEFAULT_LINK.syncPollSeconds;
}

export function loadLink(): LinkSettings {
  const out = { ...DEFAULT_LINK };
  try {
    Object.assign(out, JSON.parse(localStorage.getItem(LINK_KEY) || '{}'));
    out.token = (out.rememberToken ? localStorage.getItem(TOKEN_KEY) : sessionStorage.getItem(TOKEN_KEY)) || '';
  } catch {
    /* storage may be unavailable */
  }
  return { ...out, syncPollSeconds: normalizeSyncPollSeconds(out.syncPollSeconds) };
}

export function saveLink(l: LinkSettings) {
  if (!canWrite()) return;
  try {
    const { token, ...rest } = l;
    localStorage.setItem(LINK_KEY, JSON.stringify({ ...rest, syncPollSeconds: normalizeSyncPollSeconds(rest.syncPollSeconds) }));
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
 * The UPbDB server: `?server=` of this page load, else Settings, else this page's own
 * origin when grainatlas is served by UPbDB itself (under `/atlas/`).
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
  code = '';
  raw = '';
  retryAfter = 0;
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function failure(r: Response, what: string): Promise<ApiError> {
  let raw = '';
  let body: unknown = null;
  try {
    raw = await r.text();
    try { body = JSON.parse(raw); } catch { /* not JSON */ }
  } catch {
    /* no body */
  }
  const problem = body as { code?: string; detail?: { code?: string } } | null;
  const decorate = (error: ApiError) => {
    error.code = problem?.code ?? problem?.detail?.code ?? '';
    error.raw = raw;
    const after = r.headers.get('Retry-After');
    error.retryAfter = after ? (Number.isFinite(Number(after)) ? Number(after) * 1000 : Math.max(0, Date.parse(after) - Date.now())) : 0;
    return error;
  };
  // Raw status and body go to the console; people get one sentence.
  console.warn(`${what}: ${r.status} ${r.statusText}`, raw.slice(0, 2000));
  if (r.status === 409 || r.status === 422) {
    const detail = body && typeof body === 'object' ? (body as { detail?: unknown }).detail : null;
    const messages = (Array.isArray(detail) ? detail : [detail]).map(d => {
      const message = d && typeof d === 'object' ? (d as { msg?: unknown }).msg : d;
      return typeof message === 'string' ? message.trim() : '';
    }).filter(Boolean);
    if (messages.length) return decorate(new ApiError(r.status, `${what}: ${messages.join('; ')}`));
  }
  return decorate(new ApiError(r.status, `${what}: ${errorSentence(r.status, body)}`));
}

function headers(token: string | null, extra: Record<string, string> = {}): Record<string, string> {
  return token ? { ...extra, Authorization: `Bearer ${token}` } : extra;
}

async function call<T>(base: string, path: string, token: string | null, init: RequestInit, what: string, keyChanged?: (key: string) => void | Promise<void>, project?: string): Promise<T> {
  if (init.method && !['GET', 'HEAD'].includes(init.method.toUpperCase())) requireWrite();
  let r: Response;
  try {
    const send = (key?: string) => serverFetch(`${base}${path}`, { ...init, headers: { ...headers(token), ...(init.headers as Record<string, string> | undefined), ...(key ? { 'Idempotency-Key': key } : {}) } }, project, fetchWithDeadline);
    r = init.method === 'POST' ? await idempotentWrite(send, { key: new Headers(init.headers).get('Idempotency-Key') ?? undefined, keyChanged }) : await send();
  } catch (e) {
    if (e instanceof ServerReadOnlyError) throw e;
    console.warn(`${what}: cannot reach ${base}`, e);
    throw new ApiError(0, databaseOfflineMessage);
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
  creator?: { id: string; name: string } | null;
  lab?: { handle: string; display_name?: string } | null;
  role?: 'owner' | 'read' | 'write' | 'admin';
  sample_names?: string[];
  sample_count?: number;
  mineral?: string[];
  session_dates?: string[];
  created_at?: string | null;
  imaged_at?: string | null;
  last_changed_at?: string | null;
  content_revision?: number;
  atlas?: { project_id: string; thumbnail: string | null; uploaded_at?: string; version?: number } | null;
  thumbnail_data?: string;
}

export function listMounts(base: string, token: string | null): Promise<MountOut[]> {
  return call(base, '/v1/mounts?limit=50', token, { method: 'GET' }, 'Listing mounts');
}

export type MountType = 'epoxy_1in' | 'epoxy_1in_half' | 'thin_section' | 'grain_tape';

export function createMount(base: string, token: string | null, mount: { label: string; mount_type: MountType; notes?: string }): Promise<MountOut> {
  return call(base, '/v1/mounts', token, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(mount),
  }, 'Creating mount');
}

/** Publish compares the bundle's spots.json with the database (contract 2026-11, section 4). */
export interface SpotCheck { in_bundle: number; differ: number; not_in_database: number; only_in_database: number; names: string[] }
/** What grainatlas reads from the publish response; other fields are kept as they come. */
export interface AtlasPublishOut {
  mount_id?: string;
  project_id?: string;
  version?: number | null;
  sha256?: string;
  url?: string;
  spot_check?: SpotCheck;
  warnings?: string[];
  [k: string]: unknown;
}

export async function publishAtlas(base: string, token: string | null, mountId: string, projectId: string, zip: Uint8Array, key: string = crypto.randomUUID(), replaceCurrent = false, keyChanged?: (key: string) => void | Promise<void>, boundaryKey = key): Promise<AtlasPublishOut> {
  // A stable boundary preserves the exact request after a quit as well as an in-memory retry.
  const boundary = `grainatlas-${boundaryKey}`;
  const body = new Blob([
    `--${boundary}\r\nContent-Disposition: form-data; name="bundle"; filename="${encodeURIComponent(projectId)}.atlas.zip"\r\nContent-Type: application/zip\r\n\r\n`,
    zip as BlobPart,
    `\r\n--${boundary}\r\nContent-Disposition: form-data; name="replace_current"\r\n\r\n${replaceCurrent}\r\n--${boundary}--\r\n`,
  ], { type: `multipart/form-data; boundary=${boundary}` });
  return call(base, `/v1/mounts/${encodeURIComponent(mountId)}/atlas`, token, { method: 'POST', body,
    headers: { 'Idempotency-Key': key, 'Content-Type': body.type } }, 'Publishing the atlas', keyChanged, projectId);
}

/** URL of a file of a published bundle. */
export function atlasUrl(base: string, projectId: string, path = ''): string {
  return `${base}/atlas/projects/${encodeURIComponent(projectId)}/${path}`;
}

/** With no bearer token, UPbDB's viewer `access` token (read-only, that project's mount only) works too. */
export function getResults(base: string, token: string | null, mountId: string, access: string | null = null): Promise<import('./results').MountResults> {
  const q = !token && access ? `?access=${encodeURIComponent(access)}` : '';
  return call(base, `/v1/mounts/${encodeURIComponent(mountId)}/results${q}`, token, { method: 'GET' }, 'Mount results');
}

/** UPbDB's own page for an acquisition (when the shell cannot route a upbdb:// link). */
export function acquisitionPage(base: string, id: string): string {
  return `${base}/app/#/acquisitions/${encodeURIComponent(id)}`;
}

/** All project areas, grouped by stored sample (falling back to the area's sample/name). */
export function mountSpotDocuments(project: import('./core/types').Project, spots: Spot[], grains: Record<string, Grain[]> = (project.grainatlas_ui as { grains?: Record<string, Grain[]> })?.grains ?? {}): { documents: MountSpots[]; skipped: number } {
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
    const r = mountSpots(project.uuid, sample, ss, grains);
    skipped += r.skipped;
    return r.body;
  }).filter((d) => d.spots.length);
  return { documents, skipped };
}

export interface MountAcquisition { id: string; label: string }

/** Only links from the given project spots replace raw instrument labels. */
export function labelsForChecks(acquisitions: readonly MountAcquisition[], spots: readonly Pick<SyncSpot, 'name' | 'acquisition_ids'>[]): string[] {
  const linked = new Set(spots.flatMap(s => s.acquisition_ids ?? []));
  return [...new Set([
    ...acquisitions.filter(a => !linked.has(a.id)).map(a => a.label),
    ...spots.filter(s => s.acquisition_ids?.length).map(s => s.name),
  ])].sort();
}

// Both acquisition records and the legacy label list share one request cache.
const acquisitionLabels = new Map<string, Promise<MountAcquisition[]>>();
const labelsKey = (base: string, mountId: string) => JSON.stringify([base.replace(/\/+$/, ''), mountId.toLowerCase()]);

export function clearMountAcquisitionLabels(base: string, mountId: string) {
  acquisitionLabels.delete(labelsKey(base, mountId));
}

/** Visible sessions that measured this mount, with at most four session requests in flight. */
export function mountAcquisitions(base: string, token: string | null, mountId: string): Promise<MountAcquisition[]> {
  base = base.replace(/\/+$/, '');
  const key = labelsKey(base, mountId);
  const cached = acquisitionLabels.get(key);
  if (cached) return cached;
  const request = (async () => {
    const sessions = await call<{ id: string }[]>(base, '/v1/sessions', token, { method: 'GET' }, 'Listing acquisition sessions');
    const records: MountAcquisition[] = [];
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(4, sessions.length) }, async () => {
      while (next < sessions.length) {
        const session = sessions[next++];
        const mounts = await call<MountOut[]>(base, `/v1/mounts?session=${encodeURIComponent(session.id)}`, token, { method: 'GET' }, 'Listing session mounts');
        if (!mounts.some((m) => m.id.toLowerCase() === mountId.toLowerCase())) continue;
        const acquisitions = await call<{ id: string; instrument_label: string; reference_material_name: string }[]>(base, `/v1/sessions/${encodeURIComponent(session.id)}/acquisitions`, token, { method: 'GET' }, 'Loading acquisition labels');
        records.push(...acquisitions.filter((a) => !a.reference_material_name?.trim() && a.instrument_label?.trim()).map((a) => ({ id: a.id, label: a.instrument_label })));
      }
    }));
    return records;
  })();
  acquisitionLabels.set(key, request);
  void request.catch(() => { if (acquisitionLabels.get(key) === request) acquisitionLabels.delete(key); });
  return request;
}

/** Instrument labels for callers that do not need acquisition links. */
export async function mountAcquisitionLabels(base: string, token: string | null, mountId: string): Promise<string[]> {
  return [...new Set((await mountAcquisitions(base, token, mountId)).map(a => a.label))].sort();
}
