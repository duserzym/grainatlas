import type { Grain, Spot } from './core/types.ts';

/** Borrow the core's drawn-outline protection for grains linked to this area's spots. */
export function keepSpotGrains(grains: Grain[], spots: Spot[], areaId: string) {
  const linked = new Set(spots.filter(s => s.area_id === areaId && s.grain_id !== null).map(s => s.grain_id));
  const kept = grains.filter(g => linked.has(g.id));
  const previous = grains.map(g => linked.has(g.id) ? { ...g, drawn: true } : g);
  return { previous, kept };
}

/** Restore the entire saved record, including an absent drawn flag and UI-only fields. */
export function restoreSpotGrains(grains: Grain[], kept: Grain[]): Grain[] {
  const originals = new Map(kept.map(g => [g.id, g]));
  return grains.map(g => originals.get(g.id) ?? g);
}
