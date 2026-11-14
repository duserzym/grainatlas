/** Project identity wins over the active tab, including ids from links with different casing. */
export function existingProject<T extends { project: { uuid: string } | null }>(list: T[], uuid: string): T | undefined {
  return list.find(tab => tab.project?.uuid.toLowerCase() === uuid.toLowerCase());
}

export function chooseOpenTab<T extends { empty: boolean; project: { uuid: string } | null }>(
  list: T[], active: T, add: () => T, uuid?: string,
): T {
  return (uuid ? existingProject(list, uuid) : undefined) ?? (active.empty ? active : add());
}

interface Held {
  project: { uuid: string } | null;
  dirty: boolean;
  projectPath: unknown;
  remote: unknown;
  spots: unknown[];
  grains: Record<string, unknown[] | undefined>;
  sync: { unsynced: boolean };
}

/** A tab whose project would lose something if another project replaced it. The blank project a
 * package opens with (nothing saved, edited or registered) holds no work. */
export function holdsWork(tab: Held): boolean {
  return !!tab.project && (tab.dirty || !!tab.projectPath || !!tab.remote || tab.spots.length > 0
    || Object.values(tab.grains).some((g) => !!g?.length) || tab.sync.unsynced);
}

/** Image files open together in one tab; anything else (a package, area or image folder, a
 * `.grainatlas` folder, a `.gaproj` file) is one open each. A folder may have a dot in its name. */
const IMAGE_FILE = /\.(jpe?g|png|tiff?|bmp)$/i;

export function pathGroups(paths: string[]): string[][] {
  const files = paths.filter((p) => IMAGE_FILE.test(p));
  return [...paths.filter((p) => !IMAGE_FILE.test(p)).map((p) => [p]), ...(files.length ? [files] : [])];
}

/** The same files or folder, however the path ends or the files are ordered. */
export const sourceKey = (paths: string[]) => paths.map((p) => p.replace(/[\\/]+$/, '')).sort().join('\n');
