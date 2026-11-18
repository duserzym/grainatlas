import { canWrite } from './presentation.ts';
import { appStorage } from './app-storage.ts';
import type { Project } from './core/types.ts';
import { diffElements, elementsOf, emptyElements, withoutBlankArtifacts, type Counts, type Elements, type ReadElements } from './sync.ts';
import { draftSyncStatus } from './sync-status.ts';
export const DRAFT_PREFIX = 'grainatlas.draft.';
export const DRAFT_INDEX = 'grainatlas.drafts';
const DISCARDED = 'grainatlas.discarded-drafts';
export const DRAFTS_CHANGED = 'grainatlas:drafts-changed';
function changed() { if (typeof window !== 'undefined') window.dispatchEvent(new Event(DRAFTS_CHANGED)); }
export interface Draft {
  uuid: string; name: string; server: string | null; mount: string | null;
  base_revision: number | null; base: ReadElements | null; updated_at: string; changes: Counts | null;
  /** Content displayed when Edit began, before a first acknowledged sync. */
  base_seen?: ReadElements | null;
  /** Database additions pulled conservatively, retained for the details drawer. */
  database_added?: Elements;
  incomplete?: { reason: string; omissions: Record<keyof Elements, string[]> } | null;
  /** Narrowed unsynced elements. Missing fields identify legacy records. */
  pending?: Elements | null; pending_at?: string | null;
  /** Legacy pre-B1 alias. New links always register the local UUID. */
  linked_uuid?: string; link_offered?: boolean; link_conflict?: boolean; link_candidates?: import('./sync-api').Candidate[];
  /** Opaque hashes returned by the server, never computed by the browser. */
  base_hashes?: Record<keyof Counts, string> | null; synced?: boolean; synced_at?: string;
  source_keys?: string[]; pixels_base?: string | null; pixels_key?: string; pixels_changed?: boolean;
  /** Items the last partial sync held back (contract 2026-11); they stay in the pending changes. */
  held_back?: import('./sync').HeldItem[];
  /** The sync request awaiting a reply: a retry of the same body reuses its id. */
  /** Last lightweight revision check. This never acknowledges local edits. */
  checked_at?: string; server_revision?: number; server_can_edit?: boolean;
  pending_request?: import('./sync-api').PendingRequest | null;
}
export type DraftStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> & Partial<Pick<Storage, 'length' | 'key'>>;
const target = (storage?: DraftStorage) => storage ?? appStorage;
function arrayAt<T>(storage: DraftStorage, key: string): T[] {
  try { const value: unknown = JSON.parse(storage.getItem(key) || '[]'); return Array.isArray(value) ? value as T[] : []; } catch { return []; }
}
export function readDraft(uuid: string, storage?: DraftStorage): Draft | null {
  try {
    const draft = JSON.parse(target(storage).getItem(DRAFT_PREFIX + uuid) || 'null') as Draft | null;
    // Older drafts can hold the database's '' for "no artifact"; clean it in memory only.
    return draft?.pending ? { ...draft, pending: withoutBlankArtifacts(draft.pending) } : draft;
  } catch { return null; }
}
export function writeDraft(draft: Draft, storage?: DraftStorage): boolean {
  if (!canWrite()) return true;
  try {
    const s = target(storage);
    const bytes = JSON.stringify(draft);
    const different = s.getItem(DRAFT_PREFIX + draft.uuid) !== bytes;
    s.setItem(DRAFT_PREFIX + draft.uuid, bytes);
    s.setItem(DISCARDED, JSON.stringify(arrayAt<string>(s, DISCARDED).filter(id => id !== draft.uuid)));
    const index = arrayAt<string>(s, DRAFT_INDEX);
    if (!index.includes(draft.uuid)) s.setItem(DRAFT_INDEX, JSON.stringify([...index, draft.uuid]));
    if (different) changed();
    return true;
  } catch { return false; /* the open project's edits remain in memory */ }
}
export function deleteDraft(uuid: string, storage?: DraftStorage): boolean {
  if (!canWrite()) return true;
  try {
    const s = target(storage);
    s.removeItem(DRAFT_PREFIX + uuid);
    s.setItem(DRAFT_INDEX, JSON.stringify(arrayAt<string>(s, DRAFT_INDEX).filter(id => id !== uuid)));
    s.setItem(DISCARDED, JSON.stringify([...new Set([...arrayAt<string>(s, DISCARDED), uuid])]));
    changed();
    return true;
  } catch { return false; }
}
function newDraft(project: Project, server: string | null): Draft {
  return { uuid: project.uuid, name: project.name || project.package.name, server, mount: project.upbdb?.mount_id ?? null, base_revision: null, base: null, updated_at: new Date().toISOString(), changes: null };
}
export function ensureDraft(project: Project, server: string | null, storage?: DraftStorage): Draft {
  const previous = readDraft(project.uuid, storage);
  if (previous) return previous;
  // Creation is in memory only. The caller writes through its tracked save path,
  // where both immediate and IndexedDB failures can be reported to the tab.
  return { ...newDraft(project, server), pending: null, pending_at: null };
}
/** Listing is read-only, including legacy metadata. Adopt only after an explicit Check or a ready open. */
export function listDrafts(server: string | null = null, storage?: DraftStorage): Draft[] {
  try {
    const s = target(storage);
    const keys: string[] = [];
    for (let i = 0; i < (s.length ?? 0); i++) { const key = s.key?.(i); if (key) keys.push(key); }
    const found = new Map<string, Draft>();
    const ids = new Set([...arrayAt<string>(s, DRAFT_INDEX), ...keys.filter(k => k.startsWith(DRAFT_PREFIX)).map(k => k.slice(DRAFT_PREFIX.length))]);
    for (const id of ids) { const draft = readDraft(id, s); if (draft) found.set(id, draft); }
    for (const key of keys.filter(k => k.startsWith('grainatlas.project.') || k.startsWith('grainatlas.copy.'))) {
      try {
        const value = JSON.parse(s.getItem(key) || 'null');
        const project = (key.startsWith('grainatlas.copy.') ? value?.project : value) as Project | null;
        if (!project?.uuid || !project.package?.name || arrayAt<string>(s, DISCARDED).includes(project.uuid)) continue;
        const draft = found.get(project.uuid) ?? newDraft(project, server);
        const recent = arrayAt<{ id?: string; name?: string; when?: string }>(s, 'grainatlas.recent');
        const when = value.saved_at ?? recent.find(r => r.id === project.uuid || r.name === project.package.name)?.when;
        if (!(draft.source_keys ?? []).includes(key)) found.set(draft.uuid, { ...draft, source_keys: [...(draft.source_keys ?? []), key], ...(when && !draft.source_keys?.length && !draft.base ? { updated_at: when } : {}) });
      } catch { /* one damaged legacy entry must not hide the others */ }
    }
    return [...found.values()];
  } catch { return []; }
}
export function storedDraftProject(draft: Draft, storage?: DraftStorage): Project | null {
  try {
    const s = target(storage);
    // Prefer the newest scoped save; retain every source key for review/discard.
    const projects = (draft.source_keys ?? []).flatMap(key => {
      try {
        const value = JSON.parse(s.getItem(key) || 'null');
        const project = key.startsWith('grainatlas.copy.') ? value?.project : value;
        return project?.uuid === draft.uuid ? [{ project: project as Project, when: value.saved_at ?? '' }] : [];
      } catch { return []; }
    });
    return projects.sort((a, b) => b.when.localeCompare(a.when))[0]?.project ?? null;
  } catch { return null; }
}
/** Keep exactly the upsert fields, including new-grain lineage, without UI metadata. */
export function pendingElements(elements: ReadElements): Elements {
  const changes = diffElements(emptyElements(), elements);
  return { spots: changes.spots.upsert, grains: changes.grains.upsert, labels: changes.labels.upsert };
}
/** A cleared pending snapshot must never resurrect an older stored copy. */
export function draftElements(draft: Draft, storage?: DraftStorage): Elements | null {
  if (draft.pending !== undefined) return draft.pending;
  const project = storedDraftProject(draft, storage);
  return project ? elementsOf(project) : null;
}
/** Pending-list eligibility is read-only; unlinked copies remain available for linking. */
export function hasPendingChanges(draft: Draft, storage?: DraftStorage): boolean {
  return draftSyncStatus(draft, storage).pending;
}
/** Only call after the separate, explicit stored-copy deletion confirmation. */
export function deleteDraftCopies(draft: Draft, storage?: DraftStorage): void {
  if (!canWrite()) return;
  try {
    const s = target(storage);
    for (const key of draft.source_keys ?? []) {
      if (!key.startsWith('grainatlas.copy.') && !key.startsWith('grainatlas.project.')) continue;
      const value = JSON.parse(s.getItem(key) || 'null');
      if ((key.startsWith('grainatlas.copy.') ? value?.project : value)?.uuid === draft.uuid) s.removeItem(key);
    }
  } catch { /* storage unavailable */ }
}
