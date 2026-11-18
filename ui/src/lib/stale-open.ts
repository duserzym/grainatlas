import { KINDS, storedElements, type ReadElements, type Kind } from './sync.ts';

export type MissingElements = Record<Kind, string[]>;

/** Only an opening/adoption can infer an old copy; later edits are deliberate. */
export function staleOpenElements(base: ReadElements | null | undefined, local: ReadElements, edited = false): MissingElements | null {
  if (!base || edited) return null;
  const active = storedElements(base);
  const missing = Object.fromEntries(KINDS.map(k => {
    const present = new Set(local[k].map(e => e.id));
    return [k, active[k].filter(e => !present.has(e.id)).map(e => e.id)];
  })) as MissingElements;
  return KINDS.some(k => missing[k].length) ? missing : null;
}

export function automaticSpotRemovals(base: ReadElements | null | undefined, deleted: string[]): string[] {
  const registered = new Set(base?.spots.map(s => s.id));
  const ids = [...new Set(deleted)].filter(id => registered.has(id));
  return ids.length > 5 ? ids : [];
}
