/** Provisional B1 transport: keep mock URLs, wire types and mappings isolated here. */
import type { Project } from './core/types';
import { storedElement, type Changes, type Counts, type Element, type Elements, type Kind } from './sync.ts';
export interface ProjectStatus {
  project_uuid: string; mount_id: string; mount_label: string; atlas_version: number | null; revision: number;
  counts: Counts; hashes: Record<keyof Counts, string>; changed_at: string; changed_by: string;
}
export interface Candidate { project_uuid: string; mount_id: string; mount_label: string; package_name: string; revision: number }
export interface SyncBody extends Changes { project_uuid: string; base_revision: number | null; mode: 'snapshot' | 'delta' }
export interface ChangeEvent { revision: number; kind: 'spot' | 'grain' | 'label'; id: string; op: 'upsert' | 'delete' | 'retire'; data: Element | null; project_uuid: string | null; at: string; by: string }
export interface ElementsReply extends Elements { mount_id: string; revision: number; hashes: Record<Kind, string> }
export interface HistoryReply { mount_id: string; base_revision: number; current_revision: number; changes: ChangeEvent[] }
export interface SyncSuccess {
  ok: true; mount_id: string; project_uuid: string; mode: SyncBody['mode']; previous_revision: number; revision: number;
  applied: Record<Kind, unknown>; ids: Record<Kind, Record<string, string>>; acquisitions_linked: unknown[];
  warnings: string[]; rebased_over: unknown[]; not_in_snapshot: Record<Kind, string[]>;
}
export interface SyncConflict { ok: false; detail: string; base_revision: number; current_revision: number; changes: ChangeEvent[] }
export type SyncReply = SyncSuccess | SyncConflict;
/** History is mount-wide. Only this uuid and project-less elements enter its three-way merge. */
export function changesFromEvents(events: ChangeEvent[], uuid: string): Changes {
  const changes: Changes = { spots: { upsert: [], delete: [] }, grains: { upsert: [], retire: [], delete: [] }, labels: { upsert: [], delete: [] } };
  for (const event of [...events].sort((a, b) => a.revision - b.revision)) {
    if (event.project_uuid !== null && event.project_uuid !== uuid) continue;
    const kind = `${event.kind}s` as Kind;
    const change = changes[kind];
    change.upsert = change.upsert.filter(e => e.id !== event.id) as typeof change.upsert;
    change.delete = change.delete.filter(id => id !== event.id);
    if (kind === 'grains') changes.grains.retire = changes.grains.retire.filter(id => id !== event.id);
    if (event.op === 'upsert' && event.data && !(kind === 'grains' && 'retired' in event.data && event.data.retired)) (change.upsert as Element[]).push({ ...event.data, id: event.id });
    else if (kind === 'grains') changes.grains.retire.push(event.id);
    else change.delete.push(event.id);
  }
  return changes;
}
/** UUIDs stay stable: ids in the 200 reply map them to database server_id values. */
export function syncWireBody(body: SyncBody) {
  const narrow = <K extends Kind>(kind: K) => body[kind].upsert.map(e => ({
    ...storedElement(kind, e),
    ...(kind === 'grains' && 'split_from' in e && e.split_from ? { split_from: e.split_from } : {}),
    ...(kind === 'grains' && 'merged_from' in e && e.merged_from ? { merged_from: e.merged_from } : {}),
  }));
  return {
    project_uuid: body.project_uuid, base_revision: body.base_revision, mode: body.mode,
    spots: { upsert: narrow('spots'), delete: body.mode === 'snapshot' ? [] : body.spots.delete },
    grains: { upsert: narrow('grains'), delete: body.mode === 'snapshot' ? [] : [...new Set([...body.grains.delete, ...body.grains.retire])] },
    labels: { upsert: narrow('labels'), delete: body.mode === 'snapshot' ? [] : body.labels.delete },
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
    if (!r.ok && r.status !== 404 && r.status !== 409) throw new Error(`Database: ${r.status} ${await r.text()}`);
    return r;
  }
  async lookup(uuid: string): Promise<ProjectStatus | null> {
    const r = await this.call(`/v1/atlas/projects/${encodeURIComponent(uuid)}`);
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`Lookup: ${r.status}`);
    return r.json();
  }
  async candidates(name: string): Promise<Candidate[]> {
    const r = await this.call(`/v1/atlas/projects?package=${encodeURIComponent(name)}`);
    if (!r.ok) throw new Error(`Project candidates: ${r.status}`);
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
