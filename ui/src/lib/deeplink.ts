// Deep links: `?project=<id>&spot=<id>` in the page URL. UPb Studio maps
// `grainatlas://project/<id>/spot/<id>` to `index.html?project=<id>&spot=<id>`
// (its docs/deep-links.md). The browser keeps projects without pixels in
// localStorage under `grainatlas.project.<package name>`; this file finds the
// package name for a project id. Opening and selecting is in actions.ts.

import type { Project } from './core/types';

export interface DeepLink {
  project: string;
  spot: string | null;
}

export interface RecentEntry {
  /** Project uuid (missing in entries written before deep links). */
  id?: string;
  name: string;
  when: string;
  areas: number;
  spots: number;
}

/** Same id rule as the shell's resolver: UUIDs or URL-safe slugs. */
const ID = /^[A-Za-z0-9][A-Za-z0-9._~-]{0,127}$/;

export function parseDeepLink(search: string): DeepLink | null {
  const q = new URLSearchParams(search);
  const project = q.get('project');
  if (!project || !ID.test(project)) return null;
  const spot = q.get('spot');
  return { project, spot: spot && ID.test(spot) ? spot : null };
}

/** Ids compare case-insensitively (a UUID may arrive upper-case from another tool). */
export function sameId(a: string | null | undefined, b: string | null | undefined): boolean {
  return !!a && !!b && a.toLowerCase() === b.toLowerCase();
}

export const RECENT_KEY = 'grainatlas.recent';
export const PROJECT_PREFIX = 'grainatlas.project.';

export function readRecent(): RecentEntry[] {
  try {
    const r = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]');
    return Array.isArray(r) ? r : [];
  } catch {
    return [];
  }
}

export function savedProject(name: string): Project | null {
  try {
    const s = localStorage.getItem(PROJECT_PREFIX + name);
    return s ? (JSON.parse(s) as Project) : null;
  } catch {
    return null;
  }
}

/** Package name of the project with this uuid, from the recent list, else by scanning saved projects. */
export function packageNameFor(id: string): string | null {
  const hit = readRecent().find((r) => sameId(r.id, id));
  if (hit) return hit.name;
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key?.startsWith(PROJECT_PREFIX)) continue;
      const name = key.slice(PROJECT_PREFIX.length);
      if (sameId(savedProject(name)?.uuid, id)) return name;
    }
  } catch {
    /* storage unavailable */
  }
  return null;
}
