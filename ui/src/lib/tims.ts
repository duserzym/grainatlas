import { PickNotesRefused, type PickNotesPlan, type PickNotesReply, type PickNotesRequest } from './tims-import';
import { SyncApi, SyncApiError } from './sync-api';
import type { WriteIdentity } from './idempotency';

export const MINERALS = ['zircon', 'baddeleyite', 'titanite', 'rutile', 'apatite', 'monazite', 'xenotime', 'other'] as const;
export type Mineral = typeof MINERALS[number];
export const MINERAL_LETTERS: Record<Mineral, string> = { zircon: 'z', baddeleyite: 'b', titanite: 't', rutile: 'r', apatite: 'a', monazite: 'm', xenotime: 'x', other: 'o' };
export const FRACTION_NAME = /^[a-z]{1,3}[0-9]{1,3}[a-z]?$/;
export interface SampleRef { id: string; name: string }
export interface SourceInput { atlas_grain_id: string; atlas_spot_id?: string | null; is_fragment?: boolean; fragment_note?: string }
export interface SourceOut extends SourceInput { id: string; spot_label?: string | null; retired_at?: string | null }
export interface FractionFields { name: string; mineral: Mineral; batch_id: string | null; capsule_number: number | null; notes: string }
export interface FractionOut extends FractionFields {
  id: string; sample: SampleRef; lab_id?: string; status: string; sources: SourceOut[];
  capsule_label?: string | null; capsule_set_name?: string | null; retired_at?: string | null;
}
export interface CapsuleOut { id: string; lab_id: string; lab_name?: string; name: string; code: string; positions: number; capsule_labels: string[] }
export interface BatchOut { id: string; capsule_set_id: string; started_on: string; operator_name: string; notes: string; retired_at?: string | null }
export interface BatchDetail extends BatchOut { capsule_map: Record<string, string | null>; capsule_set?: CapsuleOut; fractions?: FractionOut[] }
export interface BatchInput { capsule_set_id: string; started_on: string; notes?: string; recipe_id?: string | null }
export interface FractionInput extends FractionFields { sources: SourceInput[] }
export interface TimsPage { rows: unknown[]; etag: string | null; next: string | null }
export type TimsPages = Record<string, TimsPage>;
export interface TimsTransport {
  list<T>(path: string, pages: TimsPages, progress?: (rows: T[]) => void): Promise<T[]>;
  detail<T>(path: string, cached?: { value: T; etag: string | null }): Promise<{ value: T; etag: string | null }>;
  write<T>(path: string, method: 'POST' | 'PATCH', body: unknown, identity: WriteIdentity): Promise<T>;
  suggest(grain: string, mineral: Mineral): Promise<{ suggestion: string; sample: SampleRef }>;
}
export class TimsApi implements TimsTransport {
  private api: SyncApi;
  private project?: string;
  constructor(server: string, token: string | null, access: string | null = null, project?: string) { this.api = new SyncApi(server, token, access); this.project = project; }
  private async checked(r: Response) {
    if (r.ok || r.status === 304) return r;
    const raw = await r.text();
    let payload: { detail?: string; code?: string } = {};
    try { payload = JSON.parse(raw); } catch { /* Keep the status and reply excerpt for non-JSON failures. */ }
    const error = new SyncApiError(r.status, [payload.detail || `Database ${r.status}: ${raw.slice(0, 180)}`]);
    error.code = payload.code ?? null; error.raw = `${r.status} ${raw}`; throw error;
  }
  async list<T>(path: string, pages: TimsPages, progress?: (rows: T[]) => void): Promise<T[]> {
    const rows: T[] = [], cursors = new Set<string>();
    const started = performance.now(); let requests = 0;
    try {
      for (let next: string | null = path; next;) {
        if (cursors.has(next)) throw new Error('The database repeated a TIMS list cursor');
        cursors.add(next);
        const saved: TimsPage | undefined = pages[next];
        const r = await this.checked(await this.api.resource(next, { headers: saved?.etag ? { 'If-None-Match': saved.etag } : {} })); requests++;
        if (r.status !== 304) pages[next] = { rows: await r.json(), etag: r.headers.get('ETag'), next: r.headers.get('X-Next-Cursor') };
        const page: TimsPage | undefined = pages[next];
        if (!page) throw new Error('The database returned 304 without a cached TIMS page');
        rows.push(...page.rows as T[]); progress?.([...rows]);
        const cursor: string | null = page.next;
        // Keep all filters, including mount, for every keyset page.
        const query = new URL(path, 'https://local.invalid');
        if (cursor) query.searchParams.set('after', cursor);
        next = cursor ? query.pathname + query.search : null;
      }
      return rows;
    } finally { console.debug('TIMS list', path.split('?')[0], { requests, rows: rows.length, ms: Math.round(performance.now() - started) }); }
  }
  async detail<T>(path: string, cached?: { value: T; etag: string | null }) {
    const r = await this.checked(await this.api.resource(path, { headers: cached?.etag ? { 'If-None-Match': cached.etag } : {} }));
    if (r.status === 304 && cached) return cached;
    return { value: await r.json() as T, etag: r.headers.get('ETag') };
  }
  async write<T>(path: string, method: 'POST' | 'PATCH', body: unknown, identity: WriteIdentity): Promise<T> {
    const r = await this.checked(await this.api.resource(path, { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }, identity, this.project));
    return r.json();
  }
  async previewPickNotes(body: PickNotesRequest): Promise<PickNotesPlan> {
    const started = performance.now();
    try { return await (await this.checked(await this.api.resource('/v1/tims/pick-notes/preview', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }, {}, this.project))).json(); }
    finally { console.debug('TIMS pick preview', { requests: 1, ms: Math.round(performance.now() - started) }); }
  }
  async importPickNotes(body: PickNotesRequest, identity: WriteIdentity): Promise<PickNotesReply> {
    const started = performance.now();
    try {
      const r = await this.api.resource('/v1/tims/pick-notes/import', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }, identity, this.project);
      if (r.status === 409) {
        const payload = await r.clone().json();
        if (payload.code === 'pick_notes_refused' && payload.plan) throw new PickNotesRefused(payload.plan, `Database 409: ${payload.detail}`);
      }
      return (await this.checked(r)).json();
    } finally { console.debug('TIMS pick import', { requests: 1, ms: Math.round(performance.now() - started) }); }
  }
  async suggest(grain: string, mineral: Mineral) {
    return (await this.detail<{ suggestion: string; sample: SampleRef }>(`/v1/fractions/suggest-name?${new URLSearchParams({ atlas_grain_id: grain, mineral })}`)).value;
  }
}
export function localSuggestion(fractions: FractionOut[], sample: SampleRef, mineral: Mineral) {
  const letter = MINERAL_LETTERS[mineral]; let max = 0;
  for (const fraction of fractions) {
    if (sample.id ? fraction.sample.id !== sample.id : fraction.sample.name !== sample.name) continue;
    const match = new RegExp(`^${letter}([0-9]+)[a-z]?$`).exec(fraction.name);
    if (match) max = Math.max(max, Number(match[1]));
  }
  return `${letter}${max + 1}`;
}
export function nextFreeCapsule(positions: number, occupancy: Record<string, string | null>) {
  for (let n = 1; n <= positions; n++) if (!occupancy[String(n)]) return n;
  return null;
}

/** Machine preference only; no mount or project content. */
export function lastCapsuleSet(): string {
  try { return localStorage.getItem('grainatlas.tims.capsule-set') ?? ''; } catch { return ''; }
}
export function rememberCapsuleSet(id: string) {
  try { localStorage.setItem('grainatlas.tims.capsule-set', id); } catch { /* The durable mount mirror still remembers the choice. */ }
}
export function orderedBatches(batches: BatchOut[], set: string) {
  return batches.filter(b => b.capsule_set_id === set && !b.retired_at).sort((a, b) => b.started_on.localeCompare(a.started_on) || b.id.localeCompare(a.id));
}
export function uniqueGrains<T extends { grain: string }>(grains: T[]): T[] {
  const seen = new Set<string>();
  return grains.filter(g => !seen.has(g.grain) && !!seen.add(g.grain));
}
export function sampleConflict(grains: { sample: SampleRef }[]): string {
  const names = [...new Set(grains.map(g => g.sample.name).filter(Boolean))];
  const ids = new Set(grains.map(g => g.sample.id).filter(Boolean));
  return names.length > 1 || ids.size > 1 ? `Choose grains from one sample: ${names.join(', ')}` : '';
}
