import { draftElements, storedDraftProject, type Draft, type DraftStorage } from './drafts.ts';
import { comparablePixelKey, pixelSignature } from './sync-pixels.ts';
import { hasChanges, KINDS, type Counts } from './sync.ts';

/** A live key wins over the recorded key. A legacy stored copy can only clear a true flag.
 * Null (or an unreadable base) is unknown. Undefined retains the pre-publication
 * warning, including when loaded pixels first establish that there are images. */
export function pixelsPending(draft: Draft, storage?: DraftStorage, liveKey: string | null = null): boolean {
  if (draft.pixels_base === undefined) return liveKey !== null || !!draft.pixels_changed;
  const base = comparablePixelKey(draft.pixels_base);
  if (base === null) return false;
  const key = liveKey ?? draft.pixels_key;
  if (key === undefined) {
    if (!draft.pixels_changed) return false;
    const project = storedDraftProject(draft, storage);
    return !project || comparablePixelKey(pixelSignature(project)) !== base;
  }
  const current = comparablePixelKey(key);
  return current !== null && current !== base;
}

/** Shared local database work for the Open list, session chip and quit dialog.
 * Loaded sessions supply their memoised key and element counts, so unsaved edits
 * take precedence over the last tracked snapshot. Listing never writes storage. */
export function draftSyncStatus(draft: Draft, storage?: DraftStorage, live?: {
  changes: Counts; pixelsKey: string | null;
  /** Resolve stored copies at tracking boundaries, never from a rendered getter. */
  stored?: { pixels: boolean; unlinked: boolean };
}) {
  const pending = live ? null : draftElements(draft, storage);
  const elements = hasChanges(live?.changes ?? draft.changes) || !!pending && KINDS.some(k => pending[k].length > 0);
  const pixels = live?.pixelsKey == null && live?.stored ? live.stored.pixels : pixelsPending(draft, storage, live?.pixelsKey);
  const unlinked = live?.stored?.unlinked ?? (!draft.mount && (!!draft.link_candidates?.length || !!storedDraftProject(draft, storage)));
  return { elements, pixels, unlinked, pending: elements || pixels || unlinked };
}

export type PlainSyncStatus = 'Saved' | 'Syncing…' | 'Up to date' | 'Offline, will sync later' | 'Needs your attention';
export interface SyncFlags {
  busy?: boolean; reachable?: boolean | null; error?: boolean;
  attention?: boolean; serverRevision?: number | null; checked?: boolean;
}
/** The sole plain-language status table, shared by live sessions and stored drafts.
 * A revision check can establish freshness, but can never acknowledge pending work. */
export function plainSyncStatus(draft: Draft | null, work: { pending: boolean }, flags: SyncFlags = {}): PlainSyncStatus {
  if (flags.attention || draft?.incomplete || draft?.link_conflict || draft?.held_back?.length) return 'Needs your attention';
  if (flags.busy) return 'Syncing…';
  if (flags.reachable === false) return 'Offline, will sync later';
  if (flags.error) return 'Needs your attention';
  if (work.pending || draft?.pending_request) return 'Saved';
  const revision = flags.serverRevision ?? draft?.server_revision;
  const checked = flags.checked ?? (!!draft?.checked_at && Date.now() - Date.parse(draft.checked_at) < 600_000);
  return checked && revision != null && revision === draft?.base_revision ? 'Up to date' : 'Saved';
}
