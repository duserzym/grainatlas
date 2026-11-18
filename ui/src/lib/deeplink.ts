import { appStorage } from './app-storage.ts';
// Deep links: `?project=<id>&spot=<id>` in the page URL, and `?server=<base>&project=<id>&spot=<id>`
// for a bundle published to UPbDB (opened read-only from `<base>/atlas/projects/<id>/`). UPb Lab maps
// `grainatlas://project/<id>/spot/<id>` to `index.html?project=<id>&spot=<id>`
// (its docs/deep-links.md). The browser keeps projects without pixels in
// appStorage under `grainatlas.project.<package name>`; this file finds the
// package name for a project id. Opening and selecting is in actions.ts.

import type { Project } from './core/types';

export interface DeepLink {
  project: string;
  spot: string | null;
  /** `?server=<base>`: open the project's published bundle from that UPbDB server. */
  server?: string | null;
  /** `?access=`: UPbDB's signed read-only token for that project's files and its mount's results. */
  access?: string | null;
  /** `?mount=`: the mount the project is published on (UPbDB's viewer_url carries it). */
  mount?: string | null;
}

export interface RecentEntry {
  /** Project uuid (missing in entries written before deep links). */
  id?: string;
  name: string;
  /** Saved project location; absent for packages opened alone. */
  path?: string;
  /** Package folder, for packages opened alone (entries from before 2026-11-11 lack it). */
  package_path?: string;
  when: string;
  areas: number;
  spots: number;
  /** Small local overview, kept only on this computer. */
  thumbnail?: string;
}

/** Same id rule as the shell's resolver: UUIDs or URL-safe slugs. */
const ID = /^[A-Za-z0-9][A-Za-z0-9._~-]{0,127}$/;

export function parseDeepLink(search: string): DeepLink | null {
  const q = new URLSearchParams(search);
  const project = q.get('project');
  if (!project || !ID.test(project)) return null;
  const spot = q.get('spot');
  const server = q.get('server');
  return { project, spot: spot && ID.test(spot) ? spot : null, server: server && /^https?:\/\//i.test(server) ? server : null, access: q.get('access'), mount: (() => { const m = q.get('mount'); return m && ID.test(m) ? m : null; })() };
}

/** Ids compare case-insensitively (a UUID may arrive upper-case from another tool). */
export function sameId(a: string | null | undefined, b: string | null | undefined): boolean {
  return !!a && !!b && a.toLowerCase() === b.toLowerCase();
}

export const RECENT_KEY = 'grainatlas.recent';
export const PROJECT_PREFIX = 'grainatlas.project.';

export function readRecent(): RecentEntry[] {
  try {
    const r = JSON.parse(appStorage.getItem(RECENT_KEY) || '[]');
    return Array.isArray(r) ? r : [];
  } catch {
    return [];
  }
}

export function savedProject(name: string): Project | null {
  try {
    const s = appStorage.getItem(PROJECT_PREFIX + name);
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
    for (let i = 0; i < appStorage.length; i++) {
      const key = appStorage.key(i);
      if (!key?.startsWith(PROJECT_PREFIX)) continue;
      const name = key.slice(PROJECT_PREFIX.length);
      if (sameId(savedProject(name)?.uuid, id)) return name;
    }
  } catch {
    /* storage unavailable */
  }
  return null;
}
