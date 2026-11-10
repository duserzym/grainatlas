import type { Grain, GrainOwner } from './core/types';

/** Stable across views: the owner's position in the project determines its colour. */
export function ownerColourToken(ownerArea: string, areas: readonly { id: string }[]): string {
  const index = Math.max(0, areas.findIndex((a) => a.id === ownerArea));
  return `var(--ov-owner-${index % 4 + 1})`;
}

/** Count actual grains here, excluding artifacts, in project area order. */
export function ownerLegend(grains: readonly Pick<Grain, 'id' | 'artifact'>[], owners: ReadonlyMap<string, GrainOwner>, areas: readonly { id: string }[]): { area: string; count: number; colour: string }[] {
  const counts = new Map<string, number>();
  for (const g of grains) {
    const o = owners.get(g.id);
    if (!g.artifact && o?.duplicate_of) counts.set(o.owner_area, (counts.get(o.owner_area) ?? 0) + 1);
  }
  const ids = [...areas.map((a) => a.id), ...[...counts.keys()].filter((id) => !areas.some((a) => a.id === id))];
  return ids.filter((id) => counts.has(id)).map((area) => ({ area, count: counts.get(area)!, colour: ownerColourToken(area, areas) }));
}
