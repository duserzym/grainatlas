import type { Grain, PrintEdits, Project, SheetBand, SheetPlan, SheetRect, SheetStrip, StripEdit, StripKey } from './core/types.ts';

export const LABEL_MM = 4.6;
export const GAP_MM = 2.5;
const EPS = 1e-7;
export const keyId = (key: StripKey) => JSON.stringify([key.area, key.grains_hash, key.split]);
export const bandImageKey = (band: SheetBand) => keyId(band.key) + '/' + band.mosaic_rect_um.join(',');
export const sameKey = (a: StripKey, b: StripKey) => keyId(a) === keyId(b);
export const contains = (a: SheetRect, b: SheetRect) => a[0] <= b[0] + EPS && a[1] <= b[1] + EPS && a[2] >= b[2] - EPS && a[3] >= b[3] - EPS;
export const intersects = (a: SheetRect, b: SheetRect) => Math.max(a[0], b[0]) < Math.min(a[2], b[2]) - EPS && Math.max(a[1], b[1]) < Math.min(a[3], b[3]) - EPS;

/** Exact port of edit.rs strip_block, including the shared-label divider. */
export function stripBlock(s: SheetStrip, origin = s.origin_mm): SheetRect {
  return [origin[0] - (s.labelled ? 0 : GAP_MM), origin[1], origin[0] + s.size_mm[0], origin[1] + s.size_mm[1]];
}
export function bandRect(s: SheetStrip, b: SheetBand, k: number): SheetRect {
  const [x, y] = s.origin_mm;
  return [x + b.offset_mm, y + LABEL_MM, x + b.offset_mm + (b.mosaic_rect_um[2] - b.mosaic_rect_um[0]) / k, y + LABEL_MM + (b.mosaic_rect_um[3] - b.mosaic_rect_um[1]) / k];
}

/** Same ordered candidates and tie rule as edit.rs snap_origin. */
export function snapOrigin(plan: SheetPlan, strip: SheetStrip, sheet: number, origin: [number, number], tolerance = 2): [number, number] {
  const r = stripBlock(strip, origin), b = plan.sheet_bodies[sheet].body_rect_mm;
  const result: [number, number] = [...origin];
  for (const d of [0, 1]) {
    const shifts = [b[d] - r[d], b[d + 2] - r[d + 2]];
    for (const other of plan.strips.filter(s => !s.hidden && s.sheet === sheet && !sameKey(s.key, strip.key))) {
      const rect = stripBlock(other);
      shifts.push(rect[d] - GAP_MM - r[d + 2], rect[d + 2] + GAP_MM - r[d]);
    }
    const shift = shifts.filter(s => Math.abs(s) <= tolerance).sort((a, b) => Math.abs(a) - Math.abs(b))[0];
    if (shift !== undefined) result[d] += shift;
  }
  return result;
}

/** Keep pointer drops in the sheet body; the header is never a drop target. */
export function bodyOrigin(plan: SheetPlan, strip: SheetStrip, origin: [number, number]): [number, number] {
  const b = plan.sheet_bodies[strip.sheet].body_rect_mm;
  const left = b[0] + (strip.labelled ? 0 : GAP_MM);
  return [Math.max(left, Math.min(b[2] - strip.size_mm[0], origin[0])), Math.max(b[1], Math.min(b[3] - strip.size_mm[1], origin[1]))];
}
export function nudgeOrigin(origin: [number, number], direction: [number, number], large = false): [number, number] {
  const step = large ? 5 : 0.5;
  return [origin[0] + direction[0] * step, origin[1] + direction[1] * step];
}

/** MaxRects first place, top then left, with the same gap as packed.rs. */
export function firstFreePlace(body: SheetRect, size: [number, number], taken: SheetRect[]): [number, number] | null {
  let free: SheetRect[] = [[body[0], body[1], body[2] + GAP_MM, body[3] + GAP_MM]];
  for (const block of taken) {
    const r: SheetRect = [block[0], block[1], block[2] + GAP_MM, block[3] + GAP_MM];
    const split: SheetRect[] = [];
    for (const f of free) {
      if (!intersects(f, r)) { split.push(f); continue; }
      if (r[0] > f[0] + EPS) split.push([f[0], f[1], r[0], f[3]]);
      if (r[1] > f[1] + EPS) split.push([f[0], f[1], f[2], r[1]]);
      if (r[2] < f[2] - EPS) split.push([r[2], f[1], f[2], f[3]]);
      if (r[3] < f[3] - EPS) split.push([f[0], r[3], f[2], f[3]]);
    }
    free = split.filter((a, i) => !split.some((b, j) => i !== j && contains(b, a) && (!contains(a, b) || j < i)));
  }
  const place = free.filter(r => r[2] - r[0] + EPS >= size[0] + GAP_MM && r[3] - r[1] + EPS >= size[1] + GAP_MM).sort((a, b) => a[1] - b[1] || a[0] - b[0])[0];
  return place ? [place[0], place[1]] : null;
}

export function targetGrains(plan: SheetPlan, area: string, grains: Record<string, Grain[]>): Grain[] {
  const ids = new Set(plan.targets[area] ?? []);
  return (grains[area] ?? []).filter(g => ids.has(g.id));
}
/** Nearest legal edge; union intervals implicitly by checking all candidates. */
export function snapGap(value: number, axis: 0 | 1, span: [number, number], grains: Grain[], min: number, max: number): number | null {
  const boxes = grains.map(g => g.bbox_um).filter(b => b[1 - axis] < span[1] - EPS && b[3 - axis] > span[0] + EPS);
  const candidates = [Math.max(min, Math.min(max, value)), min, max, ...boxes.flatMap(b => [b[axis], b[axis + 2]])];
  return candidates.filter(v => v >= min && v <= max && !boxes.some(b => v > b[axis] + EPS && v < b[axis + 2] - EPS)).sort((a, b) => Math.abs(a - value) - Math.abs(b - value) || a - b)[0] ?? null;
}
export function cropRect(rect: SheetRect, edge: number, value: number, bounds: SheetRect, grains: Grain[]): SheetRect {
  const axis = edge % 2 as 0 | 1;
  const span: [number, number] = axis === 0 ? [rect[1], rect[3]] : [rect[0], rect[2]];
  const min = edge < 2 ? bounds[axis] : rect[axis] + EPS;
  const max = edge < 2 ? rect[axis + 2] - EPS : bounds[axis + 2];
  const at = snapGap(value, axis, span, grains, min, max);
  const next: SheetRect = [...rect];
  if (at !== null) next[edge] = at;
  return next;
}
export function grainChanges(before: SheetRect, after: SheetRect, grains: Grain[]) {
  return { leaving: grains.filter(g => contains(before, g.bbox_um) && !contains(after, g.bbox_um)), entering: grains.filter(g => !contains(before, g.bbox_um) && contains(after, g.bbox_um)) };
}

const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const number = (v: unknown) => typeof v === 'number' && Number.isFinite(v);
const index = (v: unknown) => number(v) && Number.isSafeInteger(v) && (v as number) >= 0;
const tuple = (v: unknown, n: number) => Array.isArray(v) && v.length === n && v.every(number);
const stripKey = (v: unknown) => object(v) && typeof v.area === 'string' && typeof v.grains_hash === 'string' && index(v.split) && (v.split as number) <= 0xffffffff;
function readableEdit(v: unknown): boolean {
  if (!object(v)) return false;
  switch (v.op) {
    case 'move': return stripKey(v.strip) && index(v.sheet) && (v.origin_mm == null || tuple(v.origin_mm, 2));
    case 'add_sheet': return index(v.at);
    case 'remove_sheet': return index(v.sheet);
    case 'move_sheet': return index(v.from) && index(v.to);
    case 'crop': return stripKey(v.band) && tuple(v.mosaic_rect_um, 4);
    case 'split': return stripKey(v.band) && (v.axis === 'x' || v.axis === 'y') && number(v.at_um);
    case 'join': return stripKey(v.band_a) && stripKey(v.band_b);
    case 'separate': case 'hide': case 'show': return stripKey(v.strip);
    case 'label': return stripKey(v.strip) && typeof v.text === 'string';
    case 'repack': return true;
    default: return false;
  }
}

/** Guard raw project JSON before using it for controls or replay. Never change the stored value. */
export function readPrintEdits(v: unknown): PrintEdits | null {
  if (!object(v) || !object(v.params) || !Array.isArray(v.edits) || !v.edits.every(readableEdit)) return null;
  if (v.um_per_mm !== undefined && !number(v.um_per_mm)) return null;
  const p = v.params;
  const fields = (o: Record<string, unknown>, names: string[], valid: (v: unknown) => boolean) => names.every(k => o[k] === undefined || valid(o[k]));
  if (!fields(p, ['paper', 'title', 'lab', 'project', 'registration_file', 'printed'], v => typeof v === 'string')
    || !fields(p, ['landscape'], v => typeof v === 'boolean')
    || !fields(p, ['margin_mm', 'target_grain_mm', 'pad_um', 'gap_break_um', 'context_um', 'overlap_um', 'max_repeated_fraction', 'min_grain_um', 'max_grain_um'], number)
    || !fields(p, ['min_group', 'min_cluster_grains', 'min_cut_grains'], index)
    || !fields(p, ['packing'], v => v === 'lines' || v === 'packed')
    || !fields(p, ['um_per_mm'], v => number(v) || typeof v === 'string' && (/^(auto)?$/i.test(v.trim()) || /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i.test(v.trim()) && number(Number(v))))
    || !fields(p, ['spot_sizes_um'], v => Array.isArray(v) && v.every(number))) return null;
  if (p.enhance !== undefined && (!object(p.enhance)
    || !fields(p.enhance, ['denoise', 'sharpen', 'strip'], v => typeof v === 'boolean')
    || !fields(p.enhance, ['denoise_h', 'sharpen_radius', 'sharpen_amount', 'strip_margin_um'], number)
    || !fields(p.enhance, ['strip_grey'], v => index(v) && (v as number) <= 255))) return null;
  return { ...v, um_per_mm: v.um_per_mm ?? 0 } as unknown as PrintEdits;
}

/** Store only on the open project; the usual Save writes the record on both hosts. */
export function storePrintEdits(app: { project: Project | null; readOnly: boolean; dirty: boolean }, plan: SheetPlan, edits: StripEdit[]) {
  if (!app.project || app.readOnly) return;
  if (edits.length) app.project.print_sheets = structuredClone({ params: plan.params, um_per_mm: plan.um_per_mm, edits });
  else delete app.project.print_sheets;
  app.dirty = true;
}

/** Core recomputes refresh the record; print edits are held by the UI until Save. */
export function keepPrintEdits(record: Project, current: Project | null): Project {
  if (current?.uuid === record.uuid) {
    if (current.print_sheets !== undefined) record.print_sheets = current.print_sheets;
    else delete record.print_sheets;
  }
  return record;
}

/** One active edit list for preview, undo/redo and project saves. */
export class StripHistory {
  edits: StripEdit[] = [];
  future: StripEdit[][] = [];
  past: StripEdit[][] = [];
  get count() { return this.edits.length; }
  load(edits: StripEdit[]) {
    if (JSON.stringify(this.edits) === JSON.stringify(edits)) return;
    this.edits = structuredClone(edits); this.past = []; this.future = [];
  }
  push(edit: StripEdit) { this.past.push(this.edits); this.edits = [...this.edits, edit]; this.future = []; }
  undo() { const next = this.past.pop(); if (next) { this.future.push(this.edits); this.edits = next; } }
  redo() { const next = this.future.pop(); if (next) { this.past.push(this.edits); this.edits = next; } }
  /** Remove refused operations by identity, even if more input arrived during rendering. */
  reject(rendered: StripEdit[], dropped: number[]) {
    const refused = new Set(dropped.map(i => rendered[i]).filter(e => e !== undefined));
    if (!refused.size) return;
    const clean = (edits: StripEdit[]) => edits.filter(e => !refused.has(e));
    const same = (a: StripEdit[], b: StripEdit[]) => a.length === b.length && a.every((e, i) => e === b[i]);
    this.edits = clean(this.edits);
    // Each stack ends at the state nearest the current list. Drop no-op undo/redo steps.
    const prune = (stack: StripEdit[][]) => {
      const states = stack.map(clean);
      return states.filter((state, i) => !same(state, states[i + 1] ?? this.edits));
    };
    this.past = prune(this.past);
    this.future = prune(this.future);
  }
  rebase(dropped: number[]) { this.edits = this.edits.filter((_, i) => !dropped.includes(i)); this.past = []; this.future = []; }
  automatic() { this.past.push(this.edits); this.edits = []; this.future = []; }
}

/** Recover direct sibling keys using the core's monotonically increasing counters. */
export function splitPairs(base: SheetPlan, edits: StripEdit[], dropped: number[] = []): [StripKey, StripKey][] {
  const counters = new Map<string, number>();
  const family = (k: StripKey) => JSON.stringify([k.area, k.grains_hash]);
  for (const s of base.strips) for (const b of s.bands) counters.set(family(b.key), Math.max(counters.get(family(b.key)) ?? 0, b.key.split));
  let pairs: [StripKey, StripKey][] = [];
  edits.forEach((e, i) => {
    if (dropped.includes(i)) return;
    if (e.op === 'split') {
      const n = counters.get(family(e.band)) ?? 0;
      pairs.push([{ ...e.band, split: n + 1 }, { ...e.band, split: n + 2 }]);
      counters.set(family(e.band), n + 2);
    } else if (e.op === 'join') pairs = pairs.filter(p => !p.some(k => sameKey(k, e.band_a)));
  });
  return pairs;
}
