import { canWrite } from './presentation.ts';
import { appStorage } from './app-storage.ts';
import { grainMapWithoutBlankArtifacts } from './sync.ts';
import type { Grain, Label, Project, Spot, ToneParams } from './core/types';
import type { AppState } from './store.svelte';
import { PROJECT_PREFIX, readRecent, RECENT_KEY } from './deeplink.ts';

type CopyStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export interface StoredCopy {
  saved_at: string;
  path: string;
  project: Project;
  /** Do not offer the choice until the file and its mosaics have finished loading. */
  ready: boolean;
}

export function storedCopyKey(uuid: string, path: string): string {
  return `grainatlas.copy.${uuid}|${path}`;
}

export function projectChanges(project: Project) {
  const ui = project.grainatlas_ui as {
    grains?: Record<string, Grain[]>; spots?: Spot[]; labels?: Label[]; tone?: Record<string, ToneParams>;
  } | undefined;
  return { grains: ui?.grains ?? {}, spots: ui?.spots ?? [], labels: ui?.labels ?? [], tone: ui?.tone ?? {}, image_edits: project.image_edits, print_sheets: project.print_sheets };
}

// Object key order in JSON is not a content change.
function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v: unknown) => v && typeof v === 'object' && !Array.isArray(v)
    ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b))) : v);
}

/** Prefer this path's copy; an absent copy falls back to the package-name key. */
export function newerStoredCopy(file: Project, path: string, modifiedMs: number, storage?: CopyStorage): StoredCopy | null {
  try {
    const target = storage ?? appStorage;
    const scoped = target.getItem(storedCopyKey(file.uuid, path));
    let copy: StoredCopy;
    if (scoped) {
      copy = JSON.parse(scoped) as StoredCopy;
      if (copy.path !== path) return null;
    } else {
      const legacy = target.getItem(PROJECT_PREFIX + file.package.name);
      if (!legacy) return null;
      const recent = JSON.parse(target.getItem(RECENT_KEY) || '[]') as ReturnType<typeof readRecent>;
      copy = { project: JSON.parse(legacy) as Project, path, saved_at: recent.find(r => r.name === file.package.name && !r.path)?.when ?? '', ready: false };
    }
    if (copy.project.uuid !== file.uuid || !copy.saved_at || !Number.isFinite(modifiedMs)) return null;
    const savedMs = Date.parse(copy.saved_at);
    if (!Number.isFinite(savedMs) || savedMs <= modifiedMs) return null;
    const { tone: _fileTone, ...fileContent } = projectChanges(file);
    const { tone: _copyTone, ...copyContent } = projectChanges(copy.project);
    if (canonical(fileContent) === canonical(copyContent)) return null;
    return { ...copy, ready: false };
  } catch {
    return null;
  }
}

/** Saved projects use their uuid and location; packages alone retain their legacy storage. */
export function persistProject(project: Project, path: string | null, when = new Date().toISOString(), storage?: CopyStorage): boolean {
  if (!canWrite()) return true;
  try {
    const target = storage ?? appStorage;
    if (path) target.setItem(storedCopyKey(project.uuid, path), JSON.stringify({ saved_at: when, path, project }));
    else target.setItem(PROJECT_PREFIX + project.package.name, JSON.stringify(project));
    const recent = JSON.parse(target.getItem(RECENT_KEY) || '[]') as ReturnType<typeof readRecent>;
    const thumbnail = recent.find(r => r.id === project.uuid)?.thumbnail;
    const entries = recent.filter((r) => r.name !== project.package.name);
    entries.unshift({ ...(thumbnail ? { thumbnail } : {}), id: project.uuid, name: project.package.name, when, areas: project.areas.length, spots: projectChanges(project).spots.length, ...(path ? { path } : project.package.path ? { package_path: project.package.path } : {}) });
    target.setItem(RECENT_KEY, JSON.stringify(entries.slice(0, 12)));
    return true;
  } catch {
    return false;
  }
}

export function clearStoredCopy(uuid: string, path: string, storage?: CopyStorage) {
  if (!canWrite()) return;
  try { (storage ?? appStorage).removeItem(storedCopyKey(uuid, path)); } catch { /* storage unavailable */ }
}

/** Short English date in the viewer's local time. */
export function storedCopyDate(when: string | null | undefined): string | null {
  if (!when) return null;
  const date = new Date(when);
  if (!Number.isFinite(date.getTime())) return null;
  return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', hour12: false })
    .format(date).replace(',', '');
}

export function storedCopyCounts(file: Project, copy: Project) {
  const changes = projectChanges(copy);
  const ids = new Set(projectChanges(file).spots.map((s) => s.id));
  return { areas: Object.values(changes.grains).filter((g) => g.length).length, spots: changes.spots.length, extra: changes.spots.filter((s) => !ids.has(s.id)).length };
}

/** Keep the guard through the checkpoint so it cannot overwrite the unanswered copy. */
export function answerStoredCopy(app: AppState, restore: boolean, storage?: CopyStorage) {
  if (!canWrite()) return;
  const pending = app.storedCopy;
  if (!pending?.ready) return;
  if (restore) {
    app.checkpoint();
    const changes = structuredClone(projectChanges(pending.project));
    app.grains = grainMapWithoutBlankArtifacts(changes.grains);
    app.spots = changes.spots;
    app.labels = changes.labels;
    app.tone = changes.tone;
    if (app.project) {
      if (changes.image_edits !== undefined) app.project.image_edits = changes.image_edits;
      else delete app.project.image_edits;
      if (changes.print_sheets !== undefined) app.project.print_sheets = changes.print_sheets;
      else delete app.project.print_sheets;
    }
    app.selection = [];
    app.notice = `Restored changes from ${storedCopyDate(pending.saved_at)}. Save the project to keep them in the folder; ⌘Z undoes.`;
  } else {
    clearStoredCopy(pending.project.uuid, pending.path, storage);
    app.notice = 'Kept the project file.';
  }
  app.storedCopy = null;
  if (restore) app.persist();
  void app.sync?.check();
}
