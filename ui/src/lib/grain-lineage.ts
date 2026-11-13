import type { Grain, Spot } from './core/types.ts';
import { contains } from './polygon-edit.ts';
/** Stable numeric component for sorting; label suffix for display and navigation. */
export const grainNumber = (g: Pick<Grain, 'number' | 'label'>): string => g.label?.split('/').at(-1) || String(g.number);
export function splitChildren(parent: Grain, parts: Grain[], area: string, newId = () => crypto.randomUUID()): Grain[] {
  if (parts.length !== 2) throw new Error('A split must produce exactly two grains.');
  return [...parts].sort((a, b) => b.area_um2 - a.area_um2).map((part, i) => {
    const { split_from: _s, merged_from: _m, ...geometry } = part;
    return { ...geometry, id: newId(), number: parent.number, label: `${parent.label ?? `${area}/${parent.number}`}${i ? 'b' : 'a'}`, sample: parent.sample, split_from: parent.id };
  });
}
export function mergedChild(parents: Grain[], geometry: Grain, area: string, newId = () => crypto.randomUUID()): Grain {
  const { split_from: _s, merged_from: _m, ...g } = geometry;
  const first = [...parents].sort((a, b) => a.number - b.number)[0];
  return { ...g, id: newId(), number: first.number, label: `${area}/${first.number}`, sample: first.sample, merged_from: parents.map(g => g.id) };
}
/** Preserve spot identity and name; boundary hits go to the largest child (including artifacts). */
export function reassignLineage(spots: Spot[], parents: Grain[], children: Grain[]): Spot[] {
  const ids = new Set(parents.map(g => g.id)), ordered = [...children].sort((a, b) => b.area_um2 - a.area_um2);
  return spots.map(s => {
    if (!s.grain_id || !ids.has(s.grain_id)) return s;
    const child = ordered.find(g => contains(g.polygon_um, [s.x_um, s.y_um]));
    if (!child) throw new Error(`Spot ${s.name} would lie outside the new grains. Move it before editing the grain.`);
    return { ...s, grain_id: child.id, grain_number: child.number };
  });
}
/** Explicit renumber changes the label's numeric prefix, retaining split suffixes. */
export function renumberLabels(before: Grain[], after: Grain[], area: string): Grain[] {
  return after.map(g => {
    const old = before.find(q => q.id === g.id);
    const suffix = old ? grainNumber(old).match(/^\d+([a-z]+)$/)?.[1] ?? '' : '';
    return { ...old, ...g, label: `${area}/${g.number}${suffix}` };
  });
}

/** Exports retain the numeric field and carry an unambiguous display label resolved by id. */
export function exportSpotLabels(spots: Spot[], grains: Record<string, Grain[]>): Spot[] {
  return spots.map(s => {
    const g = grains[s.area_id]?.find(g => g.id === s.grain_id);
    return g ? { ...s, grain_label: grainNumber(g) } : s;
  });
}
