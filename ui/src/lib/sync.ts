import type { Grain, Label, Project, Spot } from './core/types.ts';
import { canonicalJson } from './sync-hash.ts';

export type SyncSpot = Pick<Spot, 'id' | 'name' | 'area_id' | 'grain_id' | 'x_um' | 'y_um' | 'x2_um' | 'y2_um' | 'diameter_um' | 'kind' | 'comment'> & Partial<Spot> & { acquisition_ids?: string[]; detach_grain?: boolean };
export type SyncGrain = Pick<Grain, 'id' | 'number' | 'polygon_um' | 'centroid_um' | 'area_um2'> & Partial<Omit<Grain, 'artifact'>> & { artifact?: Grain['artifact'] | null; area_id: string; label?: string; sample?: string | null; split_from?: string; merged_from?: string[]; retired?: boolean; children?: string[] };
/** Canonical reads can contain legacy rows without a client identity. Keep them in
 * the base for context; labels can match existing local identities for diffing. */
export type ReadGrain = Omit<SyncGrain, 'id' | 'split_from'> & { id: string | null; server_id?: string; split_from?: string | string[] | null };
export interface Ownership { project_uuid?: string | null }
export interface ReadElements { spots: (SyncSpot & Ownership)[]; grains: (ReadGrain & Ownership)[]; labels: (Label & Ownership & { retired?: boolean })[] }
/** The database stores '' (older rows null) for an ordinary grain; in the client an
 * ordinary grain has no artifact field at all, and the core accepts only a reason or none. */
const blankArtifact = (grain: object): boolean => 'artifact' in grain && !(grain as { artifact?: unknown }).artifact;
export function withoutBlankArtifact<T extends object>(grain: T): T {
  if (!blankArtifact(grain)) return grain;
  const { artifact: _blank, ...rest } = grain as T & { artifact?: unknown };
  return rest as T;
}
export function withoutBlankArtifacts<T extends { grains: object[] }>(elements: T): T {
  return elements.grains.some(blankArtifact) ? { ...elements, grains: elements.grains.map(withoutBlankArtifact) } : elements;
}
export function grainMapWithoutBlankArtifacts<G extends object>(grains: Record<string, G[]>): Record<string, G[]> {
  return Object.values(grains).some(list => list.some(blankArtifact))
    ? Object.fromEntries(Object.entries(grains).map(([area, list]) => [area, list.map(withoutBlankArtifact)])) : grains;
}
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
  if (server.grains.some(g => g.id === null && local.grains.some(l => l.id !== null && storedElement('grains', l as SyncGrain).label === g.label))) return true;
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
    // The database stores '' for an ordinary grain; in the client that is no artifact at all.
    if (!out.artifact) delete out.artifact;
    if (out.artifact) out.number ??= 0;
    if (out.label == null && out.number != null) out.label = out.artifact && out.number === 0 ? `artifact/${out.id}` : `${out.area_id}/${out.number}`;
    out.sample ||= null;
    out.split_from = splitParent(out.split_from as ReadGrain['split_from']) ?? null;
    out.merged_from = [...((out.merged_from as string[] | undefined) ?? [])].sort();
  }
  return out as unknown as Elements[K][number];
}
export function storedElements(elements: ReadElements): Elements {
  return { spots: elements.spots.filter(s => s.id != null).map(s => storedElement('spots', s)), grains: identifiedGrains(elements.grains).map(g => storedElement('grains', g)), labels: elements.labels.filter(l => l.id != null && !l.retired).map(l => storedElement('labels', l)) };
}
/** Wire and base snapshots use flat grain lists, each with area_id. */
export interface Elements { spots: SyncSpot[]; grains: SyncGrain[]; labels: Label[] }
/** Local-only parents have no database lineage. Keep the project's history intact. */
export function withKnownLineage(elements: Elements, known: Set<string>): Elements {
  return { ...elements, grains: elements.grains.map(grain => {
    const out = { ...grain };
    if (!out.split_from || !known.has(out.split_from)) delete out.split_from;
    if (out.merged_from) {
      out.merged_from = out.merged_from.filter(id => known.has(id));
      if (!out.merged_from.length) delete out.merged_from;
    }
    return out;
  }) };
}
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
  return { spots: ui?.spots ?? [], labels: ui?.labels ?? [], grains: Object.entries(ui?.grains ?? {}).flatMap(([area_id, grains]) => grains.map(g => ({ ...withoutBlankArtifact(g), area_id, sample: g.sample !== undefined ? g.sample : project.areas?.find(a => a.id === area_id)?.sample ?? null }))) };
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
    (out[area_id] ??= []).push(withoutBlankArtifact({
      eq_diameter_um: Math.sqrt(4 * grain.area_um2 / Math.PI), major_um: bbox[2] - bbox[0], minor_um: bbox[3] - bbox[1],
      orientation_deg: 0, mean_intensity: 0, manual: false, scale_bar_um: 0,
      ...old, ...grain, polygon_um: points, centroid_um: grain.centroid_um ?? old?.centroid_um ?? [0, 0], area_um2: grain.area_um2 ?? old?.area_um2 ?? 0, number: grain.number ?? old?.number ?? 0, artifact: grain.artifact || '', bbox_um: samePolygon ? old.bbox_um : bbox,
    } as Grain));
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
/** Diff only one immutable collection, preserving legacy identities and protections. */
export function diffElementKind<K extends Kind>(kind: K, base: ReadElements[K], mine: ReadElements[K]): Changes[K] {
  const beforeInput = { ...emptyElements(), [kind]: base } as ReadElements;
  const afterInput = { ...emptyElements(), [kind]: mine } as ReadElements;
  const a = storedElements(mapLegacyGrains(beforeInput, afterInput))[kind];
  const b = storedElements(afterInput)[kind];
  const before = new Map<string, Element>(a.map(e => [e.id, e]));
  const after = new Set(b.map(e => e.id));
  const upsert = b.filter(e => !equalElement(kind, before.get(e.id), e));
  let removed = a.filter(e => !after.has(e.id)).map(e => e.id);
  if (kind === 'spots') removed = removed.filter(id => !(base as ReadElements['spots']).some(s => s.id === id && s.acquisition_ids?.length));
  return (kind === 'grains' ? { upsert, retire: removed, delete: [] } : { upsert, delete: removed }) as Changes[K];
}
export function diffElements(base: ReadElements, mine: ReadElements): Changes {
  return { spots: diffElementKind('spots', base.spots, mine.spots), grains: diffElementKind('grains', base.grains, mine.grains), labels: diffElementKind('labels', base.labels, mine.labels) };
}
/** Reuse a kind's result when immutable inputs survive another kind's edit. */
export function elementDiffer() {
  const cache: Partial<{ [K in Kind]: { base: ReadElements[K]; mine: ReadElements[K]; result: Changes[K] } }> = {};
  return <K extends Kind>(kind: K, base: ReadElements[K], mine: ReadElements[K]): Changes[K] => {
    const previous = cache[kind];
    if (previous?.base === base && previous.mine === mine) return previous.result as Changes[K];
    const result = diffElementKind(kind, base, mine);
    Object.assign(cache, { [kind]: { base, mine, result } });
    return result;
  };
}
/** The elements endpoint can include unowned context. Only this project's
 * identified rows can be removed; legacy id-null rows wait for adoption. */
export function projectElements(server: ReadElements, uuid: string): ReadElements {
  return Object.fromEntries(KINDS.map(k => [k, server[k].filter(e => e.project_uuid === uuid)])) as unknown as ReadElements;
}
/** Before the first acknowledgement, preserve independent edits to stored fields.
 * Missing server values are context gaps, and database-only rows stay with the
 * existing additions path. Legacy grains still wait for adoption by label. */
export function firstSyncMerge(base: ReadElements | null | undefined, mine: Elements, theirs: ReadElements, uuid: string): {
  elements: Elements; conflicts: { kind: Kind; id: string; field: string }[];
} {
  const same = (a: unknown, b: unknown): boolean => {
    if (typeof a === 'number' && typeof b === 'number') return a === b || Math.abs(a - b) <= 0.001 + Number.EPSILON * Math.max(1, Math.abs(a), Math.abs(b));
    if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => same(v, b[i]));
    return canonicalJson(a) === canonicalJson(b);
  };
  const elements = emptyElements();
  const conflicts: { kind: Kind; id: string; field: string }[] = [];
  const owned = projectElements(theirs, uuid);
  for (const kind of KINDS) {
    const before = new Map((base ?? mine)[kind].map(e => [e.id, e]));
    const server = new Map(owned[kind].filter(e => e.id !== null).map(e => [e.id, e]));
    (elements[kind] as Element[]) = mine[kind].map(local => {
      const remote = server.get(local.id);
      if (!remote) return local;
      const stored = (e: Element) => storedElement(kind, e) as unknown as Record<string, unknown>;
      const b = before.get(local.id), m = stored(local), t = stored(remote as Element);
      const original = b ? stored(b as Element) : {};
      const out = { ...local } as unknown as Record<string, unknown>;
      for (const field of FIELDS[kind]) {
        if ((remote as unknown as Record<string, unknown>)[field] == null || same(m[field], t[field])) continue;
        if (same(m[field], original[field])) out[field] = t[field];
        else if (!same(t[field], original[field])) conflicts.push({ kind, id: local.id, field });
      }
      return out as unknown as Element;
    });
  }
  return { elements, conflicts };
}
export function syncChanges(mode: 'snapshot' | 'delta', base: ReadElements, mine: Elements, server: ReadElements, uuid: string, seen: ReadElements = base): Changes {
  const changes = diffElements(mode === 'snapshot' ? emptyElements() : base, mine);
  const owned = projectElements(server, uuid);
  const lastSeen = Object.fromEntries(KINDS.map(k => [k, seen[k].filter(e => owned[k].some(s => s.id === e.id))])) as unknown as ReadElements;
  const removals = diffElements(lastSeen, mine);
  changes.spots.delete = removals.spots.delete.filter(id => !server.spots.some(s => s.id === id && s.acquisition_ids?.length));
  changes.grains.retire = removals.grains.retire;
  changes.grains.delete = [];
  changes.labels.delete = removals.labels.delete;
  return changes;
}
/** Missing own rows that were never shown are database additions, not removals. */
export function databaseAdditions(server: ReadElements, seen: ReadElements, mine: Elements, uuid: string): Elements {
  const active = storedElements(projectElements(server, uuid));
  return Object.fromEntries(KINDS.map(k => {
    const known = new Set([...seen[k], ...mine[k]].map(e => e.id));
    return [k, active[k].filter(e => !known.has(e.id))];
  })) as unknown as Elements;
}
export function keepAdditions(mine: Elements, additions: Elements): Elements {
  return Object.fromEntries(KINDS.map(k => [k, [...mine[k], ...additions[k].filter(e => !mine[k].some(m => m.id === e.id))]])) as unknown as Elements;
}
/** A 200 is provisional until both the response and canonical state agree with
 * the submitted intent. Unrelated mount omissions are only notices. */
export function syncIntentFailure(changes: Changes, mine: Elements, server: ReadElements, uuid: string,
  reply: { revision: number; previous_revision: number; not_in_snapshot: Record<Kind, string[]> }, accepted?: ReadElements): string | null {
  const current = projectElements(server, uuid);
  const intended = diffElements(current, mine);
  const sent = (k: Kind) => new Set([...changes[k].upsert.map(e => e.id), ...changes[k].delete, ...(k === 'grains' ? changes.grains.retire : [])]);
  const intendedIds = (k: Kind) => [...intended[k].upsert.map(e => e.id), ...intended[k].delete, ...(k === 'grains' ? intended.grains.retire : [])];
  for (const k of KINDS) {
    const own = new Set(current[k].map(e => e.id));
    const changed = new Set(intendedIds(k));
    const kept = reply.not_in_snapshot[k].filter(id => own.has(id) && sent(k).has(id) && changed.has(id));
    if (kept.length) return `The database kept ${kept.length} ${kept.length === 1 ? k.slice(0, -1) : k} this sync ${kept.every(id => !mine[k].some(e => e.id === id)) ? 'removed' : 'changed'}`;
  }
  const changed = KINDS.some(k => intendedIds(k).some(id => sent(k).has(id)));
  if (changed && reply.revision <= reply.previous_revision) return 'The database revision did not advance; changes kept here';
  if (accepted) {
    const remaining = changeCounts(diffElements(projectElements(accepted, uuid), mine));
    for (const k of KINDS) if (sent(k).size && remaining[k]) return `The database did not save all ${k}; changes kept here`;
  }
  return null;
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

/** One item a partial sync held back (UPbDB sync contract 2026-11, section 2). */
export interface HeldItem {
  kind: string; id: string; op?: 'upsert' | 'delete'; label?: string; code?: string; reason?: string;
  related?: { kind: string; id: string; label?: string }[];
}
export function heldKind(kind: string): Kind | null {
  const k = kind.replace(/s$/, '');
  return k === 'spot' ? 'spots' : k === 'grain' ? 'grains' : k === 'label' || k === 'annotation' ? 'labels' : null;
}
/** What a partial reply applied. Held upserts keep the database version (new ones stay out),
 * held deletes keep the element: the next base is exactly what the database now has, so the
 * held items stay local changes and the next sync sends them again with any new edits. */
export function appliedPart(changes: Changes, mine: Elements, current: ReadElements, held: HeldItem[], uuid: string): { changes: Changes; mine: Elements } {
  const out = { changes: structuredClone(changes), mine: { ...mine } as Elements };
  const owned = storedElements(projectElements(current, uuid));
  for (const item of held) {
    const kind = heldKind(item.kind);
    if (!kind) continue;
    const op = item.op ?? (changes[kind].upsert.some(e => e.id === item.id) ? 'upsert' : 'delete');
    const c = out.changes[kind] as { upsert: Element[]; delete: string[]; retire?: string[] };
    if (op === 'upsert') c.upsert = c.upsert.filter(e => e.id !== item.id);
    else { c.delete = c.delete.filter(id => id !== item.id); if (c.retire) c.retire = c.retire.filter(id => id !== item.id); }
    const stored = (owned[kind] as Element[]).find(e => e.id === item.id);
    (out.mine[kind] as Element[]) = [...(out.mine[kind] as Element[]).filter(e => e.id !== item.id), ...(stored ? [stored] : [])];
  }
  return out;
}
/** A person-readable name: spot name, grain label, annotation text; 8 id characters only as a last resort. */
export function elementLabel(kind: Kind, id: string, ...sources: (Partial<Record<Kind, readonly (Element | ReadGrain)[]>> | null | undefined)[]): string {
  for (const source of sources) {
    const element = source?.[kind]?.find(e => e.id === id || ('server_id' in e && e.server_id === id));
    if (!element) continue;
    if (kind === 'spots' && 'name' in element && element.name) return element.name;
    if (kind === 'labels' && 'text' in element && element.text) return element.text.slice(0, 40);
    if (kind === 'grains') {
      const g = element as ReadGrain;
      if (g.label) return g.label;
      if (g.number != null) return `${g.area_id}/${g.number}`;
    }
  }
  return id.slice(0, 8);
}
/** "a, b, c and 4 more": duplicates removed, at most three names. */
export function nameList(names: string[], max = 3): string {
  const unique = [...new Set(names.filter(Boolean))];
  const shown = unique.slice(0, max);
  const more = unique.length - shown.length;
  if (more) return `${shown.join(', ')} and ${more} more`;
  return shown.length > 1 ? `${shown.slice(0, -1).join(', ')} and ${shown.at(-1)}` : shown[0] ?? '';
}
/** Short readable lines for one side of a difference (base to after); JSON stays under Details. */
export function describeChange(kind: Kind, before: Element | ReadGrain | undefined, after: Element | ReadGrain | undefined, grainName: (id: string | null | undefined) => string = id => id ? id.slice(0, 8) : 'none'): string[] {
  if (!before && !after) return [];
  if (!after) return [kind === 'grains' ? 'retired' : 'deleted'];
  if (!before) return ['added'];
  const b = before as unknown as Record<string, unknown>, a = after as unknown as Record<string, unknown>;
  const lines: string[] = [];
  const num = (v: unknown) => typeof v === 'number' ? v : null;
  const point = (o: Record<string, unknown>): [number, number] | null => kind === 'grains'
    ? Array.isArray(o.centroid_um) ? [Number(o.centroid_um[0]), Number(o.centroid_um[1])] : null
    : num(o.x_um) !== null && num(o.y_um) !== null ? [o.x_um as number, o.y_um as number] : null;
  const nameKey = kind === 'spots' ? 'name' : kind === 'grains' ? 'label' : 'text';
  if (a[nameKey] != null && b[nameKey] != null && a[nameKey] !== b[nameKey]) lines.push(`renamed ${b[nameKey]} to ${a[nameKey]}`);
  if ('retired' in b || 'retired' in a) { if (!b.retired && a.retired) lines.push('retired'); else if (b.retired && !a.retired) lines.push('restored'); }
  const p = point(b), q = point(a);
  if (p && q) { const d = Math.hypot(q[0] - p[0], q[1] - p[1]); if (d >= 0.05) lines.push(`moved ${d.toFixed(1)} µm`); }
  if (kind === 'grains') {
    const pb = Array.isArray(b.polygon_um) ? b.polygon_um as number[][] : null, pa = Array.isArray(a.polygon_um) ? a.polygon_um as number[][] : null;
    if (pb && pa && !equal(pb, pa)) lines.push(`outline changed (${pb.length} to ${pa.length} vertices)`);
    if ((b.artifact || '') !== (a.artifact || '')) lines.push(a.artifact ? `marked ${a.artifact}` : 'artifact mark removed');
  }
  if (kind === 'spots') {
    if ((b.grain_id ?? null) !== (a.grain_id ?? null) && 'grain_id' in a) lines.push(`grain changed (${grainName(b.grain_id as string | null)} to ${grainName(a.grain_id as string | null)})`);
    if (num(b.diameter_um) !== null && num(a.diameter_um) !== null && !equal(b.diameter_um, a.diameter_um)) lines.push(`size changed (${b.diameter_um} to ${a.diameter_um} µm)`);
    if ((b.comment || '') !== (a.comment || '') && 'comment' in a) lines.push('comment changed');
  }
  if (lines.length) return lines;
  const same = equalElement(kind, storedElement(kind, before as never) as Element, storedElement(kind, after as never) as Element);
  return same ? [] : ['other fields changed'];
}
