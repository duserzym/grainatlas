import type { Grain, Spot } from './core/types';
import { formatName } from './spots.ts';

export type BatchOrder = 'table' | 'placement' | 'rows';
export interface BatchOptions { sample: string; pattern: string; renumber: boolean; start: number; order: BatchOrder }
export interface BatchRow { old: Spot; next: Spot; clash: boolean }

function median(ns: number[]): number {
  const sorted = [...ns].sort((a, b) => a - b), m = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[m] : (sorted[m - 1] + sorted[m]) / 2;
}

export function orderSpots(selected: Spot[], order: BatchOrder, tableIds: string[], grains: Grain[]): Spot[] {
  if (order === 'placement') return [...selected].sort((a, b) => a.created.localeCompare(b.created));
  if (order === 'table') {
    const ranks = new Map(tableIds.map((id, i) => [id, i]));
    return [...selected].sort((a, b) => (ranks.get(a.id) ?? Infinity) - (ranks.get(b.id) ?? Infinity));
  }
  if (!selected.length) return [];
  const under = grains.filter((g) => selected.some((s) => s.grain_id === g.id));
  const tol = under.length ? 0.6 * median(under.map((g) => g.eq_diameter_um)) : 3 * median(selected.map((s) => s.diameter_um));
  const rows: Spot[][] = [];
  let mean = 0;
  for (const s of [...selected].sort((a, b) => a.y_um - b.y_um)) {
    if (!rows.length || s.y_um - mean > tol) { rows.push([s]); mean = s.y_um; }
    else {
      const row = rows[rows.length - 1];
      row.push(s);
      mean += (s.y_um - mean) / row.length;
    }
  }
  return rows.flatMap((row) => row.sort((a, b) => a.x_um - b.x_um));
}

/** Plans immutable replacements. Only sample, n and name may change. */
export function planBatch(project: Spot[], selected: Spot[], options: BatchOptions, tableIds: string[], grains: Grain[]): { rows: BatchRow[]; clashes: number } {
  const ordered = orderSpots(selected, options.order, tableIds, grains);
  const ids = new Set(selected.map((s) => s.id));
  const counts = new Map<string, number>();
  for (const s of project) if (!ids.has(s.id)) counts.set(s.name, (counts.get(s.name) ?? 0) + 1);
  const rows = ordered.map((s, i) => {
    const sample = options.sample.trim() || s.sample;
    const n = options.renumber ? options.start + i : s.n;
    const next = { ...s, sample, n, name: formatName(options.pattern, sample, s.grain_number, n) };
    counts.set(next.name, (counts.get(next.name) ?? 0) + 1);
    return { old: s, next, clash: false };
  });
  for (const row of rows) row.clash = counts.get(row.next.name)! > 1;
  return { rows, clashes: new Set(rows.filter((r) => r.clash).map((r) => r.next.name)).size };
}
