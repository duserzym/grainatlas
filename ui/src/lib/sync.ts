import type { Grain, Label, Project, Spot } from './core/types.ts';
import { canonicalJson } from './sync-hash.ts';

export type SyncSpot = Pick<Spot, 'id' | 'name' | 'area_id' | 'grain_id' | 'x_um' | 'y_um' | 'x2_um' | 'y2_um' | 'diameter_um' | 'kind' | 'comment'> & Partial<Spot> & { acquisition_ids?: string[]; detach_grain?: boolean };
export type SyncGrain = Pick<Grain, 'id' | 'number' | 'polygon_um' | 'centroid_um' | 'area_um2'> & Partial<Omit<Grain, 'artifact'>> & { artifact?: Grain['artifact'] | null; area_id: string; label?: string; sample?: string | null; split_from?: string; merged_from?: string[]; retired?: boolean; children?: string[] };
/** Canonical reads can contain legacy rows without a client identity. Keep them in
 * the base for context; labels can match existing local identities for diffing. */
export type ReadGrain = Omit<SyncGrain, 'id' | 'split_from'> & { id: string | null; server_id?: string; split_from?: string | string[] | null };
export interface ReadElements { spots: SyncSpot[]; grains: ReadGrain[]; labels: (Label & { retired?: boolean })[] }
export const splitParent = (value: ReadGrain['split_from']): string | undefined => Array.isArray(value) ? value[0] : value || undefined;
export function identifiedGrains(grains: ReadGrain[]): SyncGrain[] {
  return grains.filter(g => g.id !== null && !g.retired).map(g => ({ ...g, id: g.id!, split_from: splitParent(g.split_from) }));
}
/** Legacy labels identify the bundle grain without inventing a database UUID. */
export function mapLegacyGrains(elements: ReadElements, local: ReadElements): ReadElements {
  return { ...elements, grains: elements.grains.map(g => {
    if (g.id !== null) return g;
    const match = local.grains.find(l => l.id !== null && storedElement('grains', l as SyncGrain).label === (g.label ?? `${g.area_id}/${g.number}`));
    return match ? { ...g, id: match.id } : g;
  }) };
}
export function needsFirstSnapshot(server: ReadElements, local: ReadElements): boolean {
  if (server.grains.some(g => g.id === null)) return true;
  return KINDS.some(k => local[k].some(e => e.id !== null && !server[k].some(s => s.id === e.id)));
}
const FIELDS = {
  spots: ['id', 'name', 'area_id', 'grain_id', 'x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um', 'kind', 'comment'],
  grains: ['id', 'area_id', 'number', 'label', 'polygon_um', 'centroid_um', 'area_um2', 'sample', 'artifact', 'split_from', 'merged_from'],
  labels: ['id', 'area_id', 'x_um', 'y_um', 'text'],
} as const;
/** Only database-stored fields participate in equality or ordinary upserts. */
export function storedElement<K extends Kind>(kind: K, element: Elements[K][number]): Elements[K][number] {
  const out = Object.fromEntries(FIELDS[kind].filter(key => key in element).map(key => [key, (element as unknown as Record<string, unknown>)[key]]));
  if (kind === 'grains') {
    // Artifacts are unnumbered (0); each still needs a unique B1 grain label.
    out.artifact ||= '';
    if (out.artifact) out.number ??= 0;
    if (out.label == null && out.number != null) out.label = out.artifact && out.number === 0 ? `artifact/${out.id}` : `${out.area_id}/${out.number}`;
    out.sample ||= null;
    out.split_from = splitParent(out.split_from as ReadGrain['split_from']) ?? null;
    out.merged_from = [...((out.merged_from as string[] | undefined) ?? [])].sort();
  }
  return out as unknown as Elements[K][number];
}
export function storedElements(elements: ReadElements): Elements {
  return { spots: elements.spots.filter(s => s.id != null).map(s => storedElement('spots', s)), grains: identifiedGrains(elements.grains).map(g => storedElement('grains', g)), labels: elements.labels.filter(l => !l.retired).map(l => storedElement('labels', l)) };
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
export function groupedGrains(grains: ReadGrain[], previous: Record<string, Grain[]> = {}): Record<string, Grain[]> {
  const out: Record<string, Grain[]> = {};
  const existing = new Map(Object.values(previous).flat().map(g => [g.id, g]));
  const active = identifiedGrains(grains);
  // A published bundle can already have a UUID and polygon for a legacy database
  // grain. Keep that local identity until the first snapshot adopts it by label.
  for (const legacy of grains.filter(g => g.id === null && !g.retired)) {
    const local = previous[legacy.area_id]?.find(g => (g.label ?? `${legacy.area_id}/${g.number}`) === legacy.label);
    if (local && !active.some(g => g.id === local.id)) active.push({ ...local, area_id: legacy.area_id });
  }
  for (const { area_id, ...grain } of active) {
    const points = grain.polygon_um ?? [];
    const xs = points.map(p => p[0]), ys = points.map(p => p[1]);
    const bbox: Grain['bbox_um'] = points.length ? [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)] : [0, 0, 0, 0];
    const old = existing.get(grain.id);
    const samePolygon = old && canonicalJson(old.polygon_um) === canonicalJson(points);
    (out[area_id] ??= []).push({
      eq_diameter_um: Math.sqrt(4 * grain.area_um2 / Math.PI), major_um: bbox[2] - bbox[0], minor_um: bbox[3] - bbox[1],
      orientation_deg: 0, mean_intensity: 0, manual: false, scale_bar_um: 0,
      ...old, ...grain, polygon_um: points, centroid_um: grain.centroid_um ?? old?.centroid_um ?? [0, 0], area_um2: grain.area_um2 ?? old?.area_um2 ?? 0, number: grain.number ?? old?.number ?? 0, artifact: grain.artifact ?? undefined, bbox_um: samePolygon ? old.bbox_um : bbox,
    });
  }
  return out;
}
export function uiSpots(spots: SyncSpot[], grains: ReadGrain[], previous: Spot[] = [], samples: Record<string, string> = {}): Spot[] {
  const old = new Map(previous.map(s => [s.id, s]));
  return spots.filter(s => s.id != null).map(s => ({
    sample: samples[s.area_id] ?? '', n: 1, created: '', created_by: '', ...old.get(s.id), ...s,
    grain_number: grains.find(g => g.id === s.grain_id)?.number ?? null,
  }));
}
/** JSON float spelling is irrelevant; tolerate database geometry rounding. */
const equal = (a: unknown, b: unknown): boolean => {
  if (typeof a === 'number' && typeof b === 'number') return a === b || Math.abs(a - b) <= 1e-6 * Math.max(1, Math.abs(a), Math.abs(b));
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => equal(v, b[i]));
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    const value = (o: object, k: string) => {
      const v = (o as Record<string, unknown>)[k];
      if (['grain_id', 'x2_um', 'y2_um', 'diameter_um'].includes(k)) return v ?? null;
      if (k === 'comment' || k === 'artifact') return v || '';
      if (k === 'kind') return v || 'spot';
      return v;
    };
    return [...keys].every(k => k === 'sample' || equal(value(a, k), value(b, k)));
  }
  return a === b;
};
function equalElement(kind: Kind, before: Element | undefined, after: Element | undefined): boolean {
  if (!before || !after) return before === after;
  const candidate = { ...after } as unknown as Record<string, unknown>;
  const keep = kind === 'spots' ? ['x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um'] : kind === 'grains' ? ['polygon_um', 'centroid_um', 'area_um2', 'number'] : [];
  for (const key of keep) if (candidate[key] == null) candidate[key] = (before as unknown as Record<string, unknown>)[key];
  if (kind === 'grains' && candidate.label == null) candidate.label = (before as SyncGrain).label;
  if (kind === 'spots' && !('comment' in candidate)) candidate.comment = (before as SyncSpot).comment;
  return equal(before, candidate);
}
export function diffElements(base: ReadElements, mine: ReadElements): Changes {
  const beforeElements = storedElements(mapLegacyGrains(base, mine)), afterElements = storedElements(mine);
  function diff<K extends Kind>(kind: K): { upsert: Elements[K]; delete: string[] } {
    const a = beforeElements[kind], b = afterElements[kind];
    const before = new Map(a.map(e => [e.id, e]));
    const after = new Map(b.map(e => [e.id, e]));
    return { upsert: b.filter(e => !equalElement(kind, before.get(e.id), e)) as Elements[K], delete: a.filter(e => !after.has(e.id)).map(e => e.id) };
  }
  const grains = diff('grains');
  const spots = diff('spots');
  // Measurement links are protected even when an old draft predates the UI guard.
  spots.delete = spots.delete.filter(id => !base.spots.some(s => s.id === id && s.acquisition_ids?.length));
  // Every grain removal retires the server grain, even without split/merge lineage.
  return { spots, grains: { upsert: grains.upsert, retire: grains.delete, delete: [] }, labels: diff('labels') };
}
/** Current database identities supersede bundle rows, including retirement.
 * Legacy rows have no identity or geometry: keep the bundle until adoption. */
export function viewElements(bundle: Elements, server: ReadElements): Elements {
  const out = emptyElements();
  const legacy = server.grains.some(g => g.id === null);
  for (const kind of KINDS) {
    const entries = new Map<string, Element>(bundle[kind].map(e => [e.id, e]));
    for (const row of server[kind]) {
      if (row.id === null) continue;
      if ('retired' in row && row.retired) { entries.delete(row.id); continue; }
      const previous = entries.get(row.id);
      const values = { ...row } as unknown as Record<string, unknown>;
      // Null geometry is missing data, never an instruction to erase pixels'
      // matching geometry. Legacy publishing also leaves spot links empty.
      for (const key of ['polygon_um', 'centroid_um', 'area_um2', 'number', 'x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um', ...(legacy ? ['grain_id'] : [])]) {
        if (values[key] == null && previous && key in previous) delete values[key];
      }
      entries.set(row.id, { ...previous, ...values } as unknown as Element);
    }
    (out[kind] as Element[]) = [...entries.values()];
  }
  return out;
}
export function validateLineage(base: ReadElements, mine: Elements, changes: Changes): void {
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
export function syncState(input: { registered?: boolean; reachable: boolean | null; baseRevision: number | null; serverRevision: number | null; localChanges: boolean | Counts; pixelsChanged: boolean; serverChanged?: boolean; firstSync?: boolean }): SyncState {
  if (input.reachable === false) return 'offline';
  if (input.registered === undefined || input.reachable === null) return 'checking';
  if (!input.registered) return 'unregistered';
  if (input.firstSync) return 'local-changes';
  const local = typeof input.localChanges === 'boolean' ? input.localChanges : hasChanges(input.localChanges);
  const newer = input.serverChanged ?? (input.serverRevision !== null && input.baseRevision !== null && input.serverRevision > input.baseRevision);
  return newer ? local ? 'both-changed' : 'server-newer' : local ? 'local-changes' : 'in-sync';
}
export function chipText(state: SyncState, counts: Counts, version: number | null, revision: number | null, pixelsChanged = false, firstSync = false): string {
  const text: Record<SyncState, string> = {
    checking: 'Checking database…', unregistered: 'Not in the database', 'in-sync': `In sync · ${version == null ? '' : `v${version} `}r${revision ?? '?'}`,
    'local-changes': firstSync ? `Not yet synced · ${(['grains', 'spots', 'labels'] as const).filter(k => counts[k]).map(k => `${counts[k]} ${counts[k] === 1 ? k.slice(0, -1) : k}`).join(', ')}` : `Local changes · ${describeChanges(counts)}`, 'server-newer': 'Database is newer', 'both-changed': 'Both changed', offline: 'Offline, changes kept here',
  };
  return text[state] + (pixelsChanged ? ' · New images not published' : '');
}

export type Element = SyncSpot | SyncGrain | Label;
export interface Difference { kind: Kind; id: string; base?: Element; mine?: Element; theirs?: Element; collision: boolean; bothAllowed: boolean }
export interface Merge { combined: Elements; differences: Difference[] }
/** Every changed id is listed, including deletions and identical changes on both sides.
 * "Both" duplicates a spot or annotation with a fresh id; grains require a geometry decision. */
export function mergeElements(base: ReadElements, mine: ReadElements, theirs: ReadElements): Merge {
  base = mapLegacyGrains(base, mine);
  theirs = mapLegacyGrains(theirs, mine);
  const combined = emptyElements();
  const differences: Difference[] = [];
  for (const kind of KINDS) {
    const maps = [base, mine, theirs].map(s => {
      const active: Elements = { spots: s.spots, grains: identifiedGrains(s.grains), labels: s.labels.filter(l => !l.retired) };
      return new Map<string, Element>(active[kind].filter(e => e.id != null).map(e => [e.id, e]));
    });
    for (const id of new Set(maps.flatMap(m => [...m.keys()]))) {
      const [b, m, t] = maps.map(map => map.get(id));
      const normalized = (e: Element | undefined) => e ? storedElement(kind, e as Elements[typeof kind][number]) : undefined;
      const mc = !equalElement(kind, normalized(b), normalized(m)), tc = !equalElement(kind, normalized(b), normalized(t));
      const collision = mc && tc && !equalElement(kind, normalized(m), normalized(t));
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
export function applyChanges(base: ReadElements, changes: Changes): ReadElements {
  const out: ReadElements = emptyElements();
  for (const k of KINDS) {
    const removed = new Set([...changes[k].delete, ...(k === 'grains' ? changes.grains.retire : [])]);
    const entries = new Map<string | null, Element | ReadGrain>(base[k].filter(e => !removed.has(e.id ?? '')).map(e => [e.id ?? ('server_id' in e ? e.server_id! : null), e]));
    for (const e of changes[k].upsert) entries.set(e.id, e);
    (out[k] as (Element | ReadGrain)[]) = [...entries.values()];
  }
  return out;
}

/** What one sync wrote, from SyncOut.applied (not totals: a snapshot reports only real writes). */
export function appliedCounts(a: { spots: { created: number; updated: number; deleted: number }; grains: { created: number; adopted: number; updated: number; retired: number }; labels: { created: number; updated: number; retired: number } }) {
  return { spots: a.spots.created + a.spots.updated + a.spots.deleted, grains: a.grains.created + a.grains.adopted + a.grains.updated + a.grains.retired, labels: a.labels.created + a.labels.updated + a.labels.retired };
}
