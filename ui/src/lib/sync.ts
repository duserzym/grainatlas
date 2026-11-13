import type { Grain, Label, Project, Spot } from './core/types.ts';
import { canonicalJson } from './sync-hash.ts';

export type SyncSpot = Pick<Spot, 'id' | 'name' | 'area_id' | 'grain_id' | 'x_um' | 'y_um' | 'x2_um' | 'y2_um' | 'diameter_um' | 'kind' | 'comment'> & Partial<Spot> & { acquisition_ids?: string[] };
export type SyncGrain = Pick<Grain, 'id' | 'number' | 'polygon_um' | 'centroid_um' | 'area_um2'> & Partial<Omit<Grain, 'artifact'>> & { artifact?: Grain['artifact'] | null; area_id: string; label?: string; sample?: string | null; split_from?: string; merged_from?: string[]; retired?: boolean; children?: string[] };
const FIELDS = {
  spots: ['id', 'name', 'area_id', 'grain_id', 'x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um', 'kind', 'comment'],
  grains: ['id', 'area_id', 'number', 'label', 'polygon_um', 'centroid_um', 'area_um2', 'sample', 'artifact'],
  labels: ['id', 'area_id', 'x_um', 'y_um', 'text'],
} as const;
/** Only database-stored fields participate in equality or ordinary upserts. */
export function storedElement<K extends Kind>(kind: K, element: Elements[K][number]): Elements[K][number] {
  const out = Object.fromEntries(FIELDS[kind].filter(key => key in element).map(key => [key, (element as unknown as Record<string, unknown>)[key]]));
  if (kind === 'grains') { out.label ??= `${out.area_id}/${out.number}`; out.sample ??= null; out.artifact ??= null; }
  return out as unknown as Elements[K][number];
}
export function storedElements(elements: Elements): Elements {
  return { spots: elements.spots.map(s => storedElement('spots', s)), grains: elements.grains.filter(g => !g.retired).map(g => storedElement('grains', g)), labels: elements.labels.map(l => storedElement('labels', l)) };
}
/** Wire and base snapshots use flat grain lists, each with area_id. */
export interface Elements { spots: SyncSpot[]; grains: SyncGrain[]; labels: Label[] }
export type Kind = keyof Elements;
export interface Counts { spots: number; grains: number; labels: number }
export interface Changes {
  spots: { upsert: SyncSpot[]; delete: string[] };
  grains: { upsert: SyncGrain[]; retire: string[]; delete: string[] };
  labels: { upsert: Label[]; delete: string[] };
}
export const KINDS: Kind[] = ['spots', 'grains', 'labels'];
export const emptyElements = (): Elements => ({ spots: [], grains: [], labels: [] });
export function elementsOf(project: Project): Elements {
  const ui = project.grainatlas_ui as { spots?: Spot[]; grains?: Record<string, Grain[]>; labels?: Label[] } | undefined;
  return { spots: ui?.spots ?? [], labels: ui?.labels ?? [], grains: Object.entries(ui?.grains ?? {}).flatMap(([area_id, grains]) => grains.map(g => ({ ...g, area_id, sample: g.sample !== undefined ? g.sample : project.areas?.find(a => a.id === area_id)?.sample ?? null }))) };
}
/** Rebuild UI-only geometry for narrowed server grains; preserve local display metadata. */
export function groupedGrains(grains: SyncGrain[], previous: Record<string, Grain[]> = {}): Record<string, Grain[]> {
  const out: Record<string, Grain[]> = {};
  const existing = new Map(Object.values(previous).flat().map(g => [g.id, g]));
  for (const { area_id, ...grain } of grains.filter(g => !g.retired)) {
    const points = grain.polygon_um ?? [];
    const xs = points.map(p => p[0]), ys = points.map(p => p[1]);
    const bbox: Grain['bbox_um'] = points.length ? [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)] : [0, 0, 0, 0];
    const old = existing.get(grain.id);
    const samePolygon = old && canonicalJson(old.polygon_um) === canonicalJson(points);
    (out[area_id] ??= []).push({
      eq_diameter_um: Math.sqrt(4 * grain.area_um2 / Math.PI), major_um: bbox[2] - bbox[0], minor_um: bbox[3] - bbox[1],
      orientation_deg: 0, mean_intensity: 0, manual: false, scale_bar_um: 0,
      ...old, ...grain, artifact: grain.artifact ?? undefined, bbox_um: samePolygon ? old.bbox_um : bbox,
    });
  }
  return out;
}
export function uiSpots(spots: SyncSpot[], grains: SyncGrain[], previous: Spot[] = [], samples: Record<string, string> = {}): Spot[] {
  const old = new Map(previous.map(s => [s.id, s]));
  return spots.map(s => ({
    sample: samples[s.area_id] ?? '', n: 1, created: '', created_by: '', ...old.get(s.id), ...s,
    grain_number: grains.find(g => g.id === s.grain_id)?.number ?? null,
  }));
}
const equal = (a: unknown, b: unknown) => canonicalJson(a) === canonicalJson(b);
export function diffElements(base: Elements, mine: Elements): Changes {
  const beforeElements = storedElements(base), afterElements = storedElements(mine);
  function diff<K extends Kind>(kind: K): { upsert: Elements[K]; delete: string[] } {
    const a = beforeElements[kind], b = afterElements[kind];
    const before = new Map(a.map(e => [e.id, e]));
    const after = new Map(b.map(e => [e.id, e]));
    return { upsert: b.filter(e => !equal(before.get(e.id), e)) as Elements[K], delete: a.filter(e => !after.has(e.id)).map(e => e.id) };
  }
  const grains = diff('grains');
  // Lineage is an explicit command, not a stored field to compare on subsequent polls.
  grains.upsert = grains.upsert.map(g => {
    if (base.grains.some(b => b.id === g.id)) return g;
    const source = mine.grains.find(m => m.id === g.id)!;
    return { ...g, ...(source.split_from ? { split_from: source.split_from } : {}), ...(source.merged_from ? { merged_from: source.merged_from } : {}) };
  });
  const spots = diff('spots');
  // Every grain removal retires the server grain, even without split/merge lineage.
  return { spots, grains: { upsert: grains.upsert, retire: grains.delete, delete: [] }, labels: diff('labels') };
}
export function validateLineage(base: Elements, mine: Elements, changes: Changes): void {
  const parents = new Set(changes.grains.upsert.flatMap(g => [...(g.split_from ? [g.split_from] : []), ...(g.merged_from ?? [])]));
  for (const spot of base.spots.filter(s => s.grain_id && parents.has(s.grain_id))) {
    const reassigned = changes.spots.upsert.find(s => s.id === spot.id);
    if (!reassigned?.grain_id || parents.has(reassigned.grain_id) || !mine.grains.some(g => g.id === reassigned.grain_id)) throw new Error('Reassign every spot on split or merged grains in the same sync.');
  }
}
export function changeCounts(diff: Changes): Counts {
  return { spots: diff.spots.upsert.length + diff.spots.delete.length, grains: diff.grains.upsert.length + diff.grains.delete.length + diff.grains.retire.length, labels: diff.labels.upsert.length + diff.labels.delete.length };
}
export const hasChanges = (counts: Counts | null | undefined): boolean => !!counts && KINDS.some(k => counts[k] > 0);
export function describeChanges(counts: Counts): string {
  return KINDS.filter(k => counts[k]).map(k => `${counts[k]} ${counts[k] === 1 ? k.slice(0, -1) : k}`).join(', ');
}
export type SyncState = 'checking' | 'unregistered' | 'in-sync' | 'local-changes' | 'server-newer' | 'both-changed' | 'offline';
export function syncState(input: { registered?: boolean; reachable: boolean | null; baseRevision: number | null; serverRevision: number | null; localChanges: boolean | Counts; pixelsChanged: boolean; serverChanged?: boolean }): SyncState {
  if (input.reachable === false) return 'offline';
  if (input.registered === undefined || input.reachable === null) return 'checking';
  if (!input.registered) return 'unregistered';
  const local = typeof input.localChanges === 'boolean' ? input.localChanges : hasChanges(input.localChanges);
  const newer = input.serverChanged ?? (input.serverRevision !== null && input.baseRevision !== null && input.serverRevision > input.baseRevision);
  return newer ? local ? 'both-changed' : 'server-newer' : local ? 'local-changes' : 'in-sync';
}
export function chipText(state: SyncState, counts: Counts, version: number | null, revision: number | null, pixelsChanged = false): string {
  const text: Record<SyncState, string> = {
    checking: 'Checking database…', unregistered: 'Not in the database', 'in-sync': `In sync · ${version == null ? '' : `v${version} `}r${revision ?? '?'}`,
    'local-changes': `Local changes · ${describeChanges(counts)}`, 'server-newer': 'Database is newer', 'both-changed': 'Both changed', offline: 'Offline, changes kept here',
  };
  return text[state] + (pixelsChanged ? ' · New images not published' : '');
}

export type Element = SyncSpot | SyncGrain | Label;
export interface Difference { kind: Kind; id: string; base?: Element; mine?: Element; theirs?: Element; collision: boolean; bothAllowed: boolean }
export interface Merge { combined: Elements; differences: Difference[] }
/** Every changed id is listed, including deletions and identical changes on both sides.
 * "Both" duplicates a spot or annotation with a fresh id; grains require a geometry decision. */
export function mergeElements(base: Elements, mine: Elements, theirs: Elements): Merge {
  const combined = emptyElements();
  const differences: Difference[] = [];
  for (const kind of KINDS) {
    const maps = [base, mine, theirs].map(s => new Map<string, Element>(s[kind].map(e => [e.id, e])));
    for (const id of new Set(maps.flatMap(m => [...m.keys()]))) {
      const [b, m, t] = maps.map(map => map.get(id));
      const normalized = (e: Element | undefined) => e ? storedElement(kind, e as Elements[typeof kind][number]) : undefined;
      const mc = !equal(normalized(b), normalized(m)), tc = !equal(normalized(b), normalized(t));
      const collision = mc && tc && !equal(normalized(m), normalized(t));
      if (mc || tc) differences.push({ kind, id, base: b, mine: m, theirs: t, collision, bothAllowed: collision && !!m && !!t && (kind === 'labels' || (kind === 'spots' && !('acquisition_id' in m && m.acquisition_id) && !('acquisition_id' in t && t.acquisition_id) && !('acquisition_ids' in m && m.acquisition_ids?.length) && !('acquisition_ids' in t && t.acquisition_ids?.length) && 'name' in m && 'name' in t && m.name !== t.name)) });
      const chosen = mc ? m : t;
      if (chosen) (combined[kind] as Element[]).push(chosen);
    }
  }
  return { combined, differences };
}
export type Resolution = 'mine' | 'theirs' | 'both';
export function resolveMerge(merge: Merge, choices: Record<string, Resolution>, newId: () => string = () => crypto.randomUUID()): Elements {
  const out = structuredClone(merge.combined);
  for (const d of merge.differences.filter(d => d.collision)) {
    const choice = choices[`${d.kind}:${d.id}`];
    if (!choice || (choice === 'both' && !d.bothAllowed)) throw new Error(`Choose a resolution for ${d.kind} ${d.id}`);
    const values = (out[d.kind] as Element[]).filter(e => e.id !== d.id);
    const element = choice === 'theirs' ? d.theirs : d.mine;
    if (element) values.push(structuredClone(element));
    if (choice === 'both' && d.theirs) values.push({ ...structuredClone(d.theirs), id: newId() });
    (out[d.kind] as Element[]) = values;
  }
  return out;
}
export function applyChanges(base: Elements, changes: Changes): Elements {
  const out = emptyElements();
  for (const k of KINDS) {
    const removed = new Set([...changes[k].delete, ...(k === 'grains' ? changes.grains.retire : [])]);
    const entries = new Map<string, Element>(base[k].filter(e => !removed.has(e.id)).map(e => [e.id, e]));
    for (const e of changes[k].upsert) entries.set(e.id, e);
    (out[k] as Element[]) = [...entries.values()];
  }
  return out;
}
