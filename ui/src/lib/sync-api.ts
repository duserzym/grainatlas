import { fetchWithTimeout, HttpNetworkError, readBodyText } from './http.ts';
import { syncCache } from './sync-cache.ts';
import { idempotentWrite, type WriteIdentity } from './idempotency.ts';
import { oversightSession, serverFetch, ServerReadOnlyError } from './oversight.ts';
import { canWrite, requireWrite } from './presentation.ts';
/** UPbDB B1 wire contract (docs/atlas-sync.md). */
import type { Project } from './core/types';
import { nameList, type Changes, type Counts, type Element, type HeldItem, type ReadElements, type ReadGrain, type Kind } from './sync.ts';
export interface ProjectStatus {
  project_uuid: string; mount_id: string; mount_label: string; name: string; package_name: string;
  atlas_version: number | null; atlas_bundle_id: string | null; revision: number;
  counts: Counts & { grains_retired: number; grains_without_atlas_id: number };
  hashes: Record<Kind, string>; changed_at: string | null; changed_by: string | null; can_edit: boolean;
  current_project_uuid: string | null;
  project_uuids: { project_uuid: string; name: string; package_name: string; is_current: boolean }[];
}
export interface Candidate {
  project_uuid: string | null; mount_id: string; mount_label: string; matched_by: 'package' | 'project_name' | 'mount_label';
  atlas_version: number | null; revision: number; spot_count: number; grain_count: number; can_edit: boolean;
}
export interface SyncBody extends Changes { project_uuid: string; base_revision: number; mode: 'snapshot' | 'delta'; dry_run?: boolean; confirm_bulk?: boolean }
export interface ChangeEvent { revision: number; kind: 'spot' | 'grain' | 'label'; id: string; op: 'upsert' | 'delete' | 'retire'; data: Element | ReadGrain | null; project_uuid: string | null; at: string; by: string | null }
export interface ElementsReply extends ReadElements { mount_id: string; revision: number; hashes: Record<Kind, string>; project_uuids: string[]; current_project_uuid: string | null }
export interface HistoryReply { mount_id: string; since: number; revision: number; changes: ChangeEvent[] }
export interface SyncSuccess {
  ok: true; mount_id: string; project_uuid: string; mode: SyncBody['mode']; previous_revision: number; revision: number;
  applied: { spots: { created: number; updated: number; deleted: number; unchanged: number }; grains: { created: number; adopted: number; updated: number; retired: number; unchanged: number }; labels: { created: number; updated: number; retired: number; unchanged: number } };
  ids: Record<Kind, Record<string, string>>; acquisitions_linked: number;
  complete: boolean; warnings: string[]; rebased_over: number; not_in_snapshot: Record<Kind, string[]>; dry_run: boolean;
  /** Partial syncs (contract 2026-11): items the server did not apply; replayed = a stored reply to a retried request. */
  held_back?: HeldItem[]; replayed?: boolean;
}
export interface SyncConflict { ok: false; detail: string; base_revision: number; current_revision: number; changes: ChangeEvent[] }
export type SyncReply = SyncSuccess | SyncConflict;
export class SyncApiError extends Error {
  status: number;
  messages: string[];
  code: string | null = null;
  get transient() { return this.status >= 500 || this.code === 'idempotency_in_progress'; }
  /** Raw status and body, for the details view and the console only. */
  raw = '';
  missing: Record<Kind, string[]> | null = null;
  problems: HeldItem[] = [];
  bulkConfirmation = false;
  confirmation?: import('@lab/bridge').AtlasSyncConfirmation;
  constructor(status: number, messages: string[]) {
    const nonempty = messages.filter(Boolean);
    if (!nonempty.length) nonempty.push('Invalid sync request');
    super(nonempty.join(' · ')); this.status = status; this.messages = nonempty;
  }
}
/** The request did not reach the server or its reply was lost: the same request may be retried. */
export { HttpNetworkError as SyncNetworkError };
const SyncNetworkError = HttpNetworkError;
/** One sentence for an error reply: the server's own detail or message when it sends a code
 * (contract 2026-11), else by status. Status 0 means the request never reached the server. */
export function errorSentence(status: number, body: unknown): string {
  const payload = body && typeof body === 'object' ? body as { code?: unknown; detail?: unknown; message?: unknown } : {};
  const text = (v: unknown) => typeof v === 'string' && v.trim() ? v.trim() : '';
  if (typeof payload.code === 'string' && payload.code) {
    const own = text(payload.message) || text(payload.detail);
    if (own) return own;
  }
  if (status === 0) return 'The database cannot be reached. Try again later.';
  if (status === 401) return 'Sign in again to use the database.';
  if (status === 403) return 'You do not have permission to change this mount.';
  if (status === 404) return 'The database does not have this project or mount.';
  if (status >= 500) return 'The database had a problem. Try again later.';
  return 'The database did not accept this request. Try again.';
}
/** A problems list in one line: no duplicates, at most three, then what to do. */
export function problemSummary(problems: string[], todo = 'Fix them here, then sync again.'): string {
  const unique = [...new Set(problems.map(p => p.trim()).filter(Boolean))];
  if (unique.length <= 1) return unique[0] ?? '';
  return `${unique.length} problems: ${nameList(unique)}. ${todo}`;
}
/** A retry of the same request keeps its id so the server can replay its stored reply. */
export interface PendingRequest {
  id: string; key: string;
  /** Exact request and acknowledgement context, durable before sending. Never rebuilt on retry. */
  wire_id?: string; body?: SyncBody; mine?: import('./sync').Elements; current?: ReadElements;
}
export function requestIdFor(pending: PendingRequest | null | undefined, key: string, newId: () => string = () => crypto.randomUUID()): PendingRequest {
  return pending && pending.key === key ? pending : { id: newId(), key };
}
/** Keep the server's exact messages in the error and details drawer. */
export function lineageFailureMessage(messages: string[]): string | null {
  const labels = [...new Set(messages.flatMap(message => {
    const match = /^grain (.+): parent (\S+) must (?:be a non-retired grain of this mount|already exist on this mount)$/.exec(message);
    return match ? [match[1]] : [];
  }))];
  if (!labels.length) return null;
  const shown = labels.slice(0, 5);
  if (labels.length > shown.length) shown.push(`${labels.length - shown.length} more`);
  const names = shown.length === 1 ? shown[0] : `${shown.slice(0, -1).join(', ')} and ${shown.at(-1)}`;
  return `The database could not use the original grains that ${names} came from. Your grains and spots are kept here. Sync again; if this repeats, send the details to whoever runs the database.`;
}
/** History is mount-wide. Only this uuid and project-less elements enter its three-way merge. */
export function changesFromEvents(events: ChangeEvent[], uuid: string): Changes {
  const changes: Changes = { spots: { upsert: [], delete: [] }, grains: { upsert: [], retire: [], delete: [] }, labels: { upsert: [], delete: [] } };
  for (const event of [...events].sort((a, b) => a.revision - b.revision)) {
    if (event.project_uuid !== null && event.project_uuid !== uuid) continue;
    // A change-row id can be a server id; it never becomes a client identity.
    if (event.data && event.data.id === null) continue;
    const kind = `${event.kind}s` as Kind;
    const change = changes[kind];
    change.upsert = change.upsert.filter(e => e.id !== event.id) as typeof change.upsert;
    change.delete = change.delete.filter(id => id !== event.id);
    if (kind === 'grains') changes.grains.retire = changes.grains.retire.filter(id => id !== event.id);
    if (event.op === 'upsert' && event.data && !(kind === 'grains' && 'retired' in event.data && event.data.retired)) (change.upsert as Element[]).push({ ...event.data, id: event.id } as Element);
    else if (kind === 'grains') changes.grains.retire.push(event.id);
    else change.delete.push(event.id);
  }
  return changes;
}
export { syncWireBody } from './sync-wire.ts';
import { syncCompute } from './sync-compute.ts';
export class SyncApi {
  private server: string;
  private token: string | null;
  private access: string | null;
  constructor(server: string, token: string | null, access: string | null = null) { this.server = server; this.token = token; this.access = access; }
  private async call(path: string, init: RequestInit = {}, identity: WriteIdentity = {}, project?: string, durableCache = false): Promise<Response> {
    if (init.method && !['GET', 'HEAD'].includes(init.method.toUpperCase())) requireWrite();
    const url = new URL(this.server + path);
    if (this.access && !init.method) url.searchParams.set('access', this.access);
    const viewAs = oversightSession.state?.viewAs;
    if (viewAs) url.searchParams.set('as', viewAs.username);
    const cache = syncCache(this.server, this.token, this.access);
    const cacheable = !init.method || init.method === 'GET';
    const cached = cacheable && !durableCache ? cache.get(url.href) : undefined;
    let r: Response;
    try {
      const send = (key?: string) => serverFetch(url, { cache: 'no-cache', ...init, headers: { ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}), ...(cached ? { 'If-None-Match': cached.etag } : {}), ...(init.headers ?? {}), ...(key ? { 'Idempotency-Key': key } : {}) } }, project, fetchWithTimeout);
      r = init.method && !['GET', 'HEAD'].includes(init.method.toUpperCase()) ? await idempotentWrite(send, identity) : await send();
    }
    catch (e) { if (e instanceof SyncApiError || e instanceof ServerReadOnlyError) throw e; console.warn(`Database request ${path} failed:`, e); throw new SyncNetworkError(errorSentence(0, null)); }
    // Read the reply here: a body lost mid-read is a network failure (a write may have landed,
    // so the retry keeps its key), never a raw TypeError in a caller. Replies are small JSON.
    if (r.status !== 204 && r.status !== 304) {
      let text: string;
      try { text = await readBodyText(r); }
      catch (e) { console.warn(`Database reply ${path} was cut off:`, e); throw new SyncNetworkError(errorSentence(0, null)); }
      r = new Response(text, { status: r.status, statusText: r.statusText, headers: r.headers });
    }
    if (r.status === 304 && cached) return new Response(cached.body, { headers: cached.headers });
    if (cacheable && !durableCache && r.ok) {
      const etag = r.headers.get('ETag');
      if (etag) {
        cache.set(url.href, { etag, body: await r.clone().text(), headers: new Headers(r.headers) });
        if (cache.size > 64) cache.delete(cache.keys().next().value!);
      } else cache.delete(url.href);
    }
    if (!r.ok && r.status !== 304 && r.status !== 404 && r.status !== 409) {
      const raw = await r.text();
      let payload: Record<string, unknown> = {};
      try { const parsed: unknown = JSON.parse(raw); if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) payload = parsed as Record<string, unknown>; } catch { /* not JSON */ }
      console.warn(`Database ${r.status} for ${path}:`, raw.slice(0, 2000));
      if (r.status === 422) {
        const detail = payload.detail as unknown;
        const message = (v: unknown) => typeof v === 'string' ? v : v && typeof v === 'object' && 'msg' in v ? String(v.msg) : JSON.stringify(v ?? 'Invalid sync request');
        const listed = Array.isArray(detail) ? detail.map(message) : detail == null ? [] : [message(detail)];
        const messages = Array.isArray(payload.problems) && payload.problems.length ? (payload.problems as unknown[]).map(p =>
          p && typeof p === 'object' && 'reason' in p ? String(p.reason) : p && typeof p === 'object' && 'message' in p ? String(p.message) : message(p)) : listed;
        const error = new SyncApiError(422, typeof payload.code === 'string' && typeof payload.message === 'string' && !messages.length ? [payload.message] : messages.length ? messages : [errorSentence(422, payload)]);
        error.code = typeof payload.code === 'string' ? payload.code : null;
        error.raw = `${r.status} ${raw}`;
        error.missing = (payload.missing as Record<Kind, string[]> | undefined) ?? null;
        if (Array.isArray(payload.problems)) error.problems = payload.problems.filter((p): p is HeldItem =>
          !!p && typeof p === 'object' && typeof p.kind === 'string' && typeof p.id === 'string').map(p => ({
            ...p, reason: p.reason || ('message' in p ? String(p.message) : error.messages[0]),
          }));
        error.bulkConfirmation = error.code === 'bulk_change' || (error.code !== 'snapshot_incomplete' && !/not_in_snapshot|incomplete.snapshot/i.test(raw) && /confirm_bulk|bulk|needs.confirmation/i.test(raw));
        const counts = (payload.confirm ?? (detail as { confirm?: unknown } | undefined)?.confirm) as Record<string, number> | undefined;
        if (counts && ['retireGrains', 'grainsTotal', 'deleteSpots', 'spotsTotal'].every(k => Number.isInteger(counts[k]) && counts[k] >= 0) && counts.retireGrains <= counts.grainsTotal && counts.deleteSpots <= counts.spotsTotal) error.confirmation = counts as unknown as import('@lab/bridge').AtlasSyncConfirmation;
        throw error;
      }
      const error = new SyncApiError(r.status, [errorSentence(r.status, payload)]);
      error.code = typeof payload.code === 'string' ? payload.code : null;
      error.raw = `${r.status} ${raw}`;
      throw error;
    }
    return r;
  }
  /** Resources with their own durable ETag cache share sync's transport and write gates.
   * Keep 304 intact: the caller owns the persisted body, including every list page. */
  resource(path: string, init: RequestInit = {}, identity: WriteIdentity = {}, project?: string) {
    return this.call(path, init, identity, project, true);
  }
  async lookup(uuid: string): Promise<ProjectStatus | null> {
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}`);
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`Lookup: ${r.status}`);
    return r.json();
  }
  async candidates(packageName: string, name = ''): Promise<Candidate[]> {
    if (!packageName && !name) return [];
    const query = new URLSearchParams();
    if (packageName) query.set('package', packageName);
    if (name) query.set('name', name);
    const r = await this.call(`/v1/atlas/projects?${query}`);
    if (!r.ok) throw new Error(`Project candidates: ${r.status}`);
    return r.json();
  }
  async link(uuid: string, mount: string): Promise<ProjectStatus> {
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}/link`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ mount_id: mount, make_current: false }) }, {}, uuid);
    if (r.status === 409) throw new SyncApiError(409, ['This project is linked to another mount']);
    if (!r.ok) throw new Error(`Link: ${r.status}`);
    return r.json();
  }
  async elements(uuid: string): Promise<ElementsReply> {
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}/elements`);
    if (!r.ok) throw new Error(`Project elements: ${r.status}`);
    return syncCompute({ op: 'parse', text: await r.text() });
  }
  async changes(mount: string, since: number): Promise<HistoryReply> {
    const r = await this.call(`/v1/mounts/${encodeURIComponent(mount)}/changes?since=${since}`);
    if (!r.ok) throw new Error(`Mount changes: ${r.status}`);
    return r.json();
  }
  /** `requestId` (never for a dry run) lets the server answer a retried request with its stored reply. */
  async sync(mount: string, body: SyncBody, requestId?: string, keyChanged?: WriteIdentity['keyChanged'], wireRequestId = requestId): Promise<SyncReply> {
    requireWrite();
    const r = await this.call(`/v1/mounts/${encodeURIComponent(mount)}/sync`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: await syncCompute({ op: 'wire', body, requestId: wireRequestId }) }, { key: requestId, keyChanged }, body.project_uuid);
    if (r.status !== 200 && r.status !== 409) throw new SyncApiError(r.status, [errorSentence(r.status, null)]);
    const reply = await r.json();
    if (r.status === 409 && !Array.isArray(reply?.changes)) {
      // A 409 without history (project linked elsewhere) is a refusal, not a conflict to review.
      const error = new SyncApiError(409, [errorSentence(409, reply)]);
      error.code = typeof reply?.code === 'string' ? reply.code : null; error.raw = `409 ${JSON.stringify(reply)}`;
      throw error;
    }
    return { ...reply, ok: r.ok };
  }
  /** Existing bundle route, only to compare published pixels with a raw package. */
  async publishedProject(uuid: string): Promise<Project | null> {
    const r = await this.call(`/atlas/projects/${encodeURIComponent(uuid)}/project.json`);
    return r.status === 404 ? null : syncCompute({ op: 'parse', text: await r.text() });
  }
}
