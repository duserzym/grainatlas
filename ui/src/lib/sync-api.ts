/** laicpmsdb B1 wire contract (docs/atlas-sync.md). */
import type { Project } from './core/types';
import { storedElement, type Changes, type Counts, type Element, type ReadElements, type ReadGrain, type Kind } from './sync.ts';
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
}
export interface SyncConflict { ok: false; detail: string; base_revision: number; current_revision: number; changes: ChangeEvent[] }
export type SyncReply = SyncSuccess | SyncConflict;
export class SyncApiError extends Error {
  status: number;
  messages: string[];
  code: string | null = null;
  missing: Record<Kind, string[]> | null = null;
  bulkConfirmation = false;
  confirmation?: import('@lab/bridge').AtlasSyncConfirmation;
  constructor(status: number, messages: string[]) {
    const nonempty = messages.filter(Boolean);
    if (!nonempty.length) nonempty.push('Invalid sync request');
    super(nonempty.join(' · ')); this.status = status; this.messages = nonempty;
  }
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
/** UUIDs stay stable: ids in the 200 reply map them to database server_id values. */
export function syncWireBody(body: SyncBody) {
  const narrow = <K extends Kind>(kind: K) => body[kind].upsert.map(e => {
    const row = { ...storedElement(kind, e) } as unknown as Record<string, unknown>;
    // Sample names are hints resolved by the database, sent only in snapshots.
    if (body.mode !== 'snapshot') delete row.sample;
    else if ('sample' in e) row.sample = e.sample;
    if (kind === 'spots') {
      if ('comment' in row) row.comment ??= '';
      if ('kind' in row) row.kind ||= 'spot';
    }
    if (kind === 'spots' && (('detach_grain' in e && e.detach_grain === true) || ('grain_id' in e && e.grain_id === null))) { row.detach_grain = true; delete row.grain_id; }
    const keep = kind === 'spots' ? ['x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um'] : kind === 'grains' ? ['polygon_um', 'centroid_um', 'area_um2', 'number'] : [];
    for (const key of keep) if (row[key] == null) delete row[key];
    return row;
  });
  return {
    project_uuid: body.project_uuid, base_revision: body.base_revision, mode: body.mode,
    ...(body.dry_run ? { dry_run: true } : {}), ...(body.confirm_bulk ? { confirm_bulk: true } : {}),
    spots: { upsert: narrow('spots'), delete: body.spots.delete },
    grains: { upsert: narrow('grains'), delete: [...new Set([...body.grains.delete, ...body.grains.retire])] },
    labels: { upsert: narrow('labels'), delete: body.labels.delete },
  };
}
export class SyncApi {
  private server: string;
  private token: string | null;
  private access: string | null;
  constructor(server: string, token: string | null, access: string | null = null) { this.server = server; this.token = token; this.access = access; }
  private async call(path: string, init: RequestInit = {}): Promise<Response> {
    const url = new URL(this.server + path);
    if (this.access && !init.method) url.searchParams.set('access', this.access);
    const r = await fetch(url, { cache: 'no-cache', ...init, headers: { ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}), ...(init.headers ?? {}) } });
    if (!r.ok && r.status !== 404 && r.status !== 409) {
      if (r.status === 422) {
        const payload = await r.json();
        const detail = payload.detail;
        const message = (v: unknown) => typeof v === 'string' ? v : v && typeof v === 'object' && 'msg' in v ? String(v.msg) : JSON.stringify(v ?? 'Invalid sync request');
        const messages = Array.isArray(detail) ? detail.map(message) : [message(detail)];
        const error = new SyncApiError(422, messages);
        error.code = payload.code ?? null;
        error.missing = payload.missing ?? null;
        error.bulkConfirmation = error.code !== 'snapshot_incomplete' && !/not_in_snapshot|incomplete.snapshot/i.test(JSON.stringify(payload)) && /confirm_bulk|bulk|needs.confirmation/i.test(JSON.stringify(payload));
        const counts = payload.confirm ?? detail?.confirm;
        if (counts && ['retireGrains', 'grainsTotal', 'deleteSpots', 'spotsTotal'].every(k => Number.isInteger(counts[k]) && counts[k] >= 0) && counts.retireGrains <= counts.grainsTotal && counts.deleteSpots <= counts.spotsTotal) error.confirmation = counts;
        throw error;
      }
      throw new SyncApiError(r.status, [`Database: ${r.status} ${await r.text()}`]);
    }
    return r;
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
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}/link`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ mount_id: mount, make_current: false }) });
    if (r.status === 409) throw new SyncApiError(409, ['This project is linked to another mount']);
    if (!r.ok) throw new Error(`Link: ${r.status}`);
    return r.json();
  }
  async elements(uuid: string): Promise<ElementsReply> {
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}/elements`);
    if (!r.ok) throw new Error(`Project elements: ${r.status}`);
    return r.json();
  }
  async changes(mount: string, since: number): Promise<HistoryReply> {
    const r = await this.call(`/v1/mounts/${encodeURIComponent(mount)}/changes?since=${since}`);
    if (!r.ok) throw new Error(`Mount changes: ${r.status}`);
    return r.json();
  }
  async sync(mount: string, body: SyncBody): Promise<SyncReply> {
    const r = await this.call(`/v1/mounts/${encodeURIComponent(mount)}/sync`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(syncWireBody(body)) });
    if (r.status !== 200 && r.status !== 409) throw new Error(`Sync: ${r.status}`);
    return { ...await r.json(), ok: r.ok };
  }
  /** Existing bundle route, only to compare published pixels with a raw package. */
  async publishedProject(uuid: string): Promise<Project | null> {
    const r = await this.call(`/atlas/projects/${encodeURIComponent(uuid)}/project.json`);
    return r.status === 404 ? null : r.json();
  }
}
