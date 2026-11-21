/** Read an existing spot's size without coercion, repair or a default. */
export function spotDiameter(s: { diameter_um?: unknown }): number | null {
  const d = s.diameter_um;
  return typeof d === 'number' && Number.isFinite(d) && d > 0 ? d : null;
}

type SizedSpot = { id: string; area_id: string; diameter_um?: unknown; created?: string };
/** Most common valid area size, then project size, then Settings. Ties use the
 * newest ISO creation time (missing is oldest), then later array position. */
export function nextSpotDiameter(spots: readonly SizedSpot[], area: string | null, fallback: number): number {
  type Frequency = { count: number; created: string; index: number };
  const here = new Map<number, Frequency>(), project = new Map<number, Frequency>();
  const newer = (a: Frequency, b: Frequency) => a.created > b.created || (a.created === b.created && a.index > b.index);
  function add(counts: Map<number, Frequency>, diameter: number, created: string, index: number) {
    const previous = counts.get(diameter);
    const latest = { count: (previous?.count ?? 0) + 1, created, index };
    if (previous && newer(previous, latest)) {
      latest.created = previous.created;
      latest.index = previous.index;
    }
    counts.set(diameter, latest);
  }
  function mostCommon(counts: Map<number, Frequency>): number | null {
    let winner: { diameter: number; frequency: Frequency } | null = null;
    for (const [diameter, frequency] of counts) {
      if (!winner || frequency.count > winner.frequency.count || (frequency.count === winner.frequency.count && newer(frequency, winner.frequency))) {
        winner = { diameter, frequency };
      }
    }
    return winner?.diameter ?? null;
  }
  spots.forEach((s, index) => {
    const diameter = spotDiameter(s);
    if (diameter === null) return;
    add(project, diameter, s.created ?? '', index);
    if (s.area_id === area) add(here, diameter, s.created ?? '', index);
  });
  return mostCommon(here) ?? mostCommon(project) ?? spotDiameter({ diameter_um: fallback }) ?? 25;
}

export function selectionSize(spots: readonly SizedSpot[], next: number) {
  if (!spots.length) return { value: next as number | null, placeholder: '', range: '', caption: 'next spot' };
  const ds = spots.map(spotDiameter), valid = ds.filter((d): d is number => d !== null);
  const common = ds.every(d => d === ds[0]);
  const lo = valid.length ? valid.reduce((a,b) => Math.min(a,b), Infinity) : null, hi = valid.length ? valid.reduce((a,b) => Math.max(a,b), -Infinity) : null;
  return { value: common ? ds[0] : null, placeholder: common ? (ds[0] === null ? 'not set' : '') : 'mixed',
    range: lo === null ? 'not set' : `⌀ ${lo}${lo === hi ? '' : `–${hi}`} µm${valid.length < ds.length ? '; not set' : ''}`,
    caption: `${spots.length} selected` };
}

/** Both the tile viewer and the µm overlay receive this current area's calibration. */
export function areaCalibration(area: { pixel_size_nm?: number } | null, mosaic: { um_per_px?: number } | null): number | null {
  return spotDiameter({ diameter_um: mosaic?.um_per_px }) ?? spotDiameter({ diameter_um: area?.pixel_size_nm == null ? null : area.pixel_size_nm / 1000 });
}

/** Resolve pixels and calibration only within the requested area. */
export function canvasArea<M extends { um_per_px: number }, A extends { id: string; pixel_size_nm?: number; mosaic?: M | null }>(
  areaId: string | null, areas: readonly A[], stitched: Readonly<Record<string, { mosaic: M } | undefined>>,
) {
  const area = areas.find(a => a.id === areaId) ?? null;
  const mosaic = (areaId ? stitched[areaId]?.mosaic : null) ?? area?.mosaic ?? null;
  return { area, mosaic, umPerPx: areaCalibration(area, mosaic) };
}

/** Selection takes priority over hover; neither may focus a spot in another area. */
export function focusSpot<T extends { id: string }>(areaSpots: readonly T[], selection: readonly string[], selectedGrains: readonly string[], infoId: string | null): T | null {
  const selected = areaSpots.find(s => selection.includes(s.id));
  const id = selected?.id ?? (selectedGrains.length ? null : infoId);
  return areaSpots.find(s => s.id === id) ?? null;
}

export interface SizeRepair { id: string; area_id: string; current: unknown; proposed: number }
function commonSize(spots: readonly SizedSpot[]): number | null {
  const counts = new Map<number, number>();
  for (const s of spots) { const d = spotDiameter(s); if (d !== null) counts.set(d, (counts.get(d) ?? 0) + 1); }
  // Deterministic tie: the smaller size. This is a proposal, never an automatic edit.
  return [...counts].sort(([a, n], [b, m]) => m - n || a - b)[0]?.[0] ?? null;
}
export function planSizeRepairs(spots: readonly SizedSpot[], fallback: number): SizeRepair[] {
  const project = commonSize(spots) ?? spotDiameter({ diameter_um: fallback }) ?? 25;
  const byArea = new Map<string, SizedSpot[]>();
  for (const s of spots) { const rows = byArea.get(s.area_id) ?? []; rows.push(s); byArea.set(s.area_id, rows); }
  const proposed = new Map([...byArea].map(([area, rows]) => [area, commonSize(rows) ?? project]));
  return spots.filter(s => spotDiameter(s) === null).map(s => ({ id: s.id, area_id: s.area_id, current: s.diameter_um, proposed: proposed.get(s.area_id)! }));
}
/** Revalidate a preview: a concurrently edited or repaired spot must not be overwritten. */
export function applySizeRepairs<T extends SizedSpot>(spots: T[], plan: readonly SizeRepair[]): T[] {
  const byId = new Map(plan.map(r => [r.id, r]));
  return spots.map(s => {
    const r = byId.get(s.id);
    return r && r.area_id === s.area_id && Object.is(r.current, s.diameter_um) && spotDiameter(s) === null && spotDiameter({ diameter_um: r.proposed }) !== null
      ? { ...s, diameter_um: r.proposed } : s;
  });
}
