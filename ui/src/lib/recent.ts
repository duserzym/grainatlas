// The Open screen's Recent projects list: where each entry lives on disk, the
// counts it shows, and what a click does. Pure functions over localStorage-like
// storage and injected disk checks, so they run under node --test.

import type { Project } from './core/types';
import { PROJECT_PREFIX, RECENT_KEY, type RecentEntry } from './deeplink.ts';
import { projectChanges } from './stored-copy.ts';

type Store = Pick<Storage, 'getItem' | 'setItem'>;

/** A saved project file or folder, or a package folder opened on its own. */
export interface RecentLocation {
  kind: 'project' | 'package';
  path: string;
}

export type PathStatus = 'present' | 'missing' | 'unreachable';

function read(storage: Store): RecentEntry[] {
  try {
    const r = JSON.parse(storage.getItem(RECENT_KEY) || '[]');
    return Array.isArray(r) ? r : [];
  } catch {
    return [];
  }
}

function write(storage: Store, entries: RecentEntry[]) {
  try { storage.setItem(RECENT_KEY, JSON.stringify(entries)); } catch { /* quota or private mode */ }
}

export function storedProject(name: string, storage: Store): Project | null {
  try {
    const s = storage.getItem(PROJECT_PREFIX + name);
    return s ? (JSON.parse(s) as Project) : null;
  } catch {
    return null;
  }
}

/**
 * Saved projects by their file; packages by the folder recorded in the entry, else
 * by the package path inside the copy this app keeps (entries written before the
 * list recorded it).
 */
export function recentLocation(entry: RecentEntry, saved: Project | null): RecentLocation | null {
  if (entry.path) return { kind: 'project', path: entry.path };
  const path = entry.package_path ?? saved?.package?.path;
  return path ? { kind: 'package', path } : null;
}

/**
 * The numbers a row shows. For a package the kept copy is the project itself, so
 * its counts are current; a saved project's file is read later (`project_counts`),
 * until then the numbers written when it was last opened stand, marked cached.
 */
export function recentCounts(entry: RecentEntry, saved: Project | null): { areas: number; spots: number; cached: boolean } {
  if (!entry.path && saved && Array.isArray(saved.areas)) return { areas: saved.areas.length, spots: projectChanges(saved).spots.length, cached: false };
  return { areas: entry.areas, spots: entry.spots, cached: true };
}

/** Takes the entry off the list; the project copies kept by this app stay. */
export function removeRecent(name: string, storage: Store): RecentEntry[] {
  const entries = read(storage).filter((r) => r.name !== name);
  write(storage, entries);
  return entries;
}

export function updateRecentCounts(name: string, counts: { areas: number; spots: number }, storage: Store): RecentEntry[] {
  const entries = read(storage).map((r) => (r.name === name ? { ...r, ...counts } : r));
  write(storage, entries);
  return entries;
}

/** After Locate…: the entry, and for a package its kept copy, point at the new place. */
export function relocateRecent(name: string, to: RecentLocation, storage: Store): RecentEntry[] {
  const entries = read(storage).map((r) => {
    if (r.name !== name) return r;
    if (to.kind === 'project') return { ...r, path: to.path };
    const { path: _old, ...rest } = r;
    return { ...rest, package_path: to.path };
  });
  write(storage, entries);
  if (to.kind === 'package') {
    const saved = storedProject(name, storage);
    if (saved?.package) {
      try { storage.setItem(PROJECT_PREFIX + name, JSON.stringify({ ...saved, package: { ...saved.package, path: to.path } })); } catch { /* quota */ }
    }
  }
  return entries;
}

/** Last path component, for checking that a located package folder is the same package. */
export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? path;
}

/** A package folder picked by Locate… must carry the package's name. */
export function samePackage(name: string, picked: string): boolean {
  return baseName(picked) === name;
}

export type RecentOutcome =
  | { kind: 'focused' }
  | { kind: 'opened' }
  | { kind: 'missing' | 'unreachable'; path: string }
  | { kind: 'unlocated' }
  | { kind: 'unsupported'; path: string | null };

export interface RecentDeps {
  /** Desktop app only: the page can open folders by path. */
  canOpenPaths: boolean;
  /** Select the tab that already shows this package; false when none does. */
  focusExisting(name: string): boolean;
  status(path: string): Promise<PathStatus>;
  open(location: RecentLocation): Promise<void>;
}

/** A click (or Enter) on a recent row. */
export async function openRecent(entry: RecentEntry, saved: Project | null, deps: RecentDeps): Promise<RecentOutcome> {
  if (deps.focusExisting(entry.name)) return { kind: 'focused' };
  const location = recentLocation(entry, saved);
  if (!deps.canOpenPaths) return { kind: 'unsupported', path: location?.path ?? null };
  if (!location) return { kind: 'unlocated' };
  const status = await deps.status(location.path);
  if (status !== 'present') return { kind: status, path: location.path };
  await deps.open(location);
  return { kind: 'opened' };
}
