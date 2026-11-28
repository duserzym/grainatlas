import { contains } from './polygon-edit.ts';
import { heldKind, withStoredLineage, type Elements, type Element, type HeldItem, type ReadElements, type SyncGrain, type SyncSpot } from './sync.ts';

export interface LinkIssue {
  spot: SyncSpot;
  original?: SyncSpot;
  candidates: { grain: SyncGrain; boundary: boolean }[];
  automatic: boolean;
  reason: string;
}

/** Build once per snapshot. Bounds come from the outline, never a cached centroid/bbox. */
function geometryIndex(mine: Elements) {
  const areas = new Map<string, { grain: SyncGrain; bounds: number[] }[]>();
  for (const grain of mine.grains) {
    const points = grain.polygon_um;
    if (grain.retired || !points || points.length < 3 || points.some(p => !p.every(Number.isFinite))) continue;
    const bounds = [Infinity, Infinity, -Infinity, -Infinity];
    for (const [x, y] of points) {
      bounds[0] = Math.min(bounds[0], x); bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x); bounds[3] = Math.max(bounds[3], y);
    }
    const rows = areas.get(grain.area_id) ?? [];
    rows.push({ grain, bounds }); areas.set(grain.area_id, rows);
  }
  for (const rows of areas.values()) rows.sort((a, b) => a.bounds[0] - b.bounds[0]);
  return areas;
}

/** Only present spots participate. A removed spot is never recreated by spatial inference. */
export function linkIssues(mine: Elements, database: ReadElements, uuid: string, held: ReadonlySet<string> = new Set()): LinkIssue[] {
  const index = geometryIndex(mine);
  const originals = new Map(database.spots.map(s => [s.id, s]));
  const grains = new Map(mine.grains.map(g => [g.id, g]));
  const registered = new Map(database.grains.filter(g => g.id).map(g => [g.id, g]));
  const out: LinkIssue[] = [];
  for (const spot of mine.spots) {
    const original = originals.get(spot.id);
    if (original?.project_uuid && original.project_uuid !== uuid) continue;
    const at: [number, number] = [original?.x_um ?? spot.x_um, original?.y_um ?? spot.y_um];
    const area = original?.area_id ?? spot.area_id;
    const linked = spot.grain_id ? grains.get(spot.grain_id) : undefined;
    if (linked && !linked.retired && !linked.artifact && linked.area_id === area && contains(linked.polygon_um, at)) continue;
    const intent = spot.grain_link_review;
    if ((intent?.action === 'keep' || intent?.action === 'detach') && intent.grain_id === spot.grain_id) continue;
    if (spot.detach_grain) continue;
    const candidates: LinkIssue['candidates'] = [];
    let blocked = false;
    for (const { grain, bounds: b } of index.get(area) ?? []) {
      if (b[0] > at[0] + 1e-7) break;
      if (b[2] < at[0] - 1e-7 || b[1] > at[1] + 1e-7 || b[3] < at[1] - 1e-7 || !contains(grain.polygon_um, at)) continue;
      const owner = registered.get(grain.id)?.project_uuid;
      const parents = [...(grain.split_from ? [grain.split_from] : []), ...(grain.merged_from ?? [])];
      const incompatible = parents.length && (!original?.grain_id || !parents.includes(original.grain_id)
        || parents.some(id => { const p = registered.get(id); return !p || p.area_id !== area || (p.project_uuid && p.project_uuid !== uuid); }));
      if (grain.artifact || (owner && owner !== uuid) || incompatible) { blocked = true; continue; }
      candidates.push({ grain, boundary: !contains(grain.polygon_um, at, false) });
    }
    const moved = spot.area_id !== area || spot.x_um !== at[0] || spot.y_um !== at[1];
    // An intentionally unassigned point is not an error. Review it only after a
    // relevant edit/hold, or when containing candidates need a person's choice.
    const candidateEdited = candidates.some(c => {
      const before = registered.get(c.grain.id);
      return !before || JSON.stringify(before.polygon_um) !== JSON.stringify(c.grain.polygon_um);
    });
    if (!spot.grain_id && !original?.grain_id && !moved && intent?.action !== 'review'
      && !held.has(spot.id) && !blocked && candidates.length <= 1
      && !candidates.some(c => c.boundary) && !candidateEdited) continue;
    // A null link replacing a confirmed link may be deliberate. Offer review but never undo it automatically.
    const detached = !!original?.grain_id && spot.grain_id == null;
    const automatic = !moved && !detached && !blocked && candidates.length === 1 && !candidates[0].boundary;
    const reason = moved ? 'The spot differs from its registered position or area. Keep the database version or review the edit.'
      : detached ? 'The registered grain link was cleared here. Choose a grain, keep the original, or confirm detach.'
      : blocked ? 'An artifact, foreign grain or incompatible split/merge history overlaps this spot. Review the outlines.'
      : candidates.some(c => c.boundary) ? 'The original spot centre touches an outline boundary. Choose after reviewing the canvas.'
      : candidates.length > 1 ? 'Several outlines contain the original spot centre. Choose after reviewing the canvas.'
      : candidates.length ? 'One outline contains the original spot centre. Sync can link this same spot to it.'
      : 'No real outline contains the original spot centre. Restore the original or detach deliberately.';
    out.push({ spot, original, candidates: moved ? [] : candidates, automatic, reason });
  }
  return out;
}

export function assignLink(spot: SyncSpot, grain: SyncGrain, reason: string): SyncSpot {
  return { ...spot, grain_id: grain.id, grain_number: grain.number, detach_grain: undefined,
    grain_link_review: { action: 'assign', grain_id: grain.id, from: spot.grain_id, reason } };
}

export function repairLinks(mine: Elements, database: ReadElements, uuid: string, excluded: ReadonlySet<string> = new Set(), held: ReadonlySet<string> = new Set()): { elements: Elements; repaired: LinkIssue[]; unresolved: LinkIssue[] } {
  const issues = linkIssues(mine, database, uuid, held).filter(i => !excluded.has(i.spot.id));
  const repaired = issues.filter(i => i.automatic);
  const bySpot = new Map(repaired.map(i => [i.spot.id, i]));
  return { unresolved: issues.filter(i => !i.automatic), elements: { ...mine, spots: mine.spots.map(s => {
    const issue = bySpot.get(s.id);
    if (issue) return assignLink(s, issue.candidates[0].grain, 'Unique same-area outline at the original spot centre; prepared during Sync.');
    return s;
  }) }, repaired };
}

/** Retain unresolved edit evidence across a successful acknowledgement of other work. */
export function rememberLinkIssues(mine: Elements, issues: LinkIssue[]): Elements {
  const unresolved = new Map(issues.map(i => [i.spot.id, i]));
  return { ...mine, spots: mine.spots.map(s => {
    const pending = unresolved.get(s.id);
    // Local metadata never changes a registered link or enters the wire.
    return pending && s.grain_link_review?.action !== 'review' ? { ...s,
      grain_link_review: { action: 'review', grain_id: s.grain_id ?? null, from: pending.original?.grain_id, reason: pending.reason } } : s;
  }) };
}

/** Revert one rejected operation, restoring only the dependencies needed to keep it consistent.
 * Existing measured spots are restored by identity; new local spots remain as unassigned spots. */
export function keepHeldVersion(mine: Elements, database: ReadElements, item: HeldItem, uuid: string): Elements {
  const kind = heldKind(item.kind);
  if (!kind) throw new Error('Open the item owner to review this operation.');
  const out: Elements = { spots: [...mine.spots], grains: [...mine.grains], labels: [...mine.labels] };
  const remote = database[kind].find(e => e.id === item.id);
  if (remote && remote.project_uuid !== uuid) throw new Error('Open the owning project to review this registered identity.');
  const active = remote && !('retired' in remote && remote.retired) ? remote : undefined;
  const localSpot = kind === 'spots' ? mine.spots.find(s => s.id === item.id) : undefined;
  if (!active && localSpot && (localSpot.acquisition_id || localSpot.acquisition_ids?.length)) {
    throw new Error('The measured spot is missing from this database copy. Open its owning project to review its identity.');
  }
  (out[kind] as Element[]) = [...(out[kind] as Element[]).filter(e => e.id !== item.id), ...(active ? [active as Element] : [])];
  const restored = new Set<string>();
  const restoreGrain = (id: string | null | undefined, spot: SyncSpot | undefined) => {
    if (!id) return;
    const g = database.grains.find(g => g.id === id && !g.retired);
    if (!g || g.project_uuid !== uuid) throw new Error('The original grain is unavailable here. Open its owning project to restore the link.');
    restored.add(id);
    const local = out.grains.find(g => g.id === id);
    if (local && !local.artifact && spot && local.area_id === spot.area_id && contains(local.polygon_um, [spot.x_um, spot.y_um])) return;
    out.grains = [...out.grains.filter(g => g.id !== id), g as SyncGrain];
  };
  if (kind === 'spots' && active) {
    const s = active as SyncSpot;
    restoreGrain(s.grain_id, s);
    const local = mine.spots.find(s => s.id === item.id);
    out.spots = out.spots.map(s => s.id === item.id ? { ...local, ...s, detach_grain: undefined,
      grain_link_review: { action: 'keep', grain_id: s.grain_id, reason: 'Kept the database version of this rejected operation.' } } : s);
  }
  if (kind === 'grains' && !active) {
    out.spots = out.spots.map(s => {
      if (s.grain_id !== item.id) return s;
      const original = database.spots.find(r => r.id === s.id);
      if (original && original.project_uuid !== uuid) throw new Error('A dependent spot belongs to another project. Open its owner.');
      restoreGrain(original?.grain_id, original);
      return { ...s, grain_id: original?.grain_id ?? null, grain_number: null, detach_grain: undefined,
        grain_link_review: { action: 'keep', grain_id: original?.grain_id ?? null, reason: 'Restored the link required by the kept database version.' } };
    });
  }
  // A restored parent must not be retired again by a remaining local lineage instruction.
  if (kind === 'grains' && active) restored.add(item.id);
  out.grains = out.grains.map(g => ({ ...g,
    ...(g.split_from && restored.has(g.split_from) ? { split_from: undefined } : {}),
    ...(g.merged_from ? { merged_from: g.merged_from.filter(id => !restored.has(id)) } : {}),
  }));
  return withStoredLineage(out, database);
}
