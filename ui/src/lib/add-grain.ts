import type { Grain, SegmentParams } from './core/types';
import { contains, polygonGrain, type Point } from './polygon-edit';
import { editedOutline } from './outline-vertices';
import { formatName, newId } from './spots';

export type AddMode = 'click' | 'draw' | 'box';
export type AddSource = { kind: 'click'; point: Point } | { kind: 'draw'; points: Point[]; free: boolean } | { kind: 'box'; points: Point[] };
export interface AddPreview {
  revision: number;
  status: 'idle' | 'working' | 'ready';
  source: AddSource | null;
  grain: Grain | null;
  error: string;
}
export type PreviewEvent = { type: 'cancel' } | { type: 'begin'; source: AddSource }
  | { type: 'result'; revision: number; grain: Grain | null; error?: string }
  | { type: 'adjust'; grain: Grain };
export const emptyAddPreview = (): AddPreview => ({ revision: 0, status: 'idle', source: null, grain: null, error: '' });
/** Only the latest live request may supply a preview. No event edits the project. */
export function previewTransition(state: AddPreview, event: PreviewEvent): AddPreview {
  if (event.type === 'cancel') return { ...emptyAddPreview(), revision: state.revision + 1 };
  if (event.type === 'begin') return { revision: state.revision + 1, source: event.source, status: 'working', grain: null, error: '' };
  if (event.type === 'adjust') return state.status === 'ready' ? { ...state, grain: event.grain } : state;
  if (state.status !== 'working' || event.revision !== state.revision) return state;
  return { ...state, status: event.grain ? 'ready' : 'idle', grain: event.grain, error: event.error ?? '' };
}

export function nextGrainName(grains: Grain[], sample: string, pattern: string) {
  let number = grains.reduce((n, g) => Math.max(n, Number.isFinite(g.number) ? g.number : 0), 0) + 1;
  const used = new Set(grains.map(g => g.label).filter(Boolean));
  let label = formatName(pattern, sample, number, number).trim() || `Grain ${number}`;
  // A constant pattern must also produce a distinct label, without an unbounded search.
  if (used.has(label)) {
    const base = label;
    do { label = `${base} (${number++})`; } while (used.has(label));
    number--;
  }
  return { number, label, sample };
}
export function namedNewGrain(geometry: Grain, grains: Grain[], sample: string, pattern: string): Grain {
  return { ...geometry, ...nextGrainName(grains, sample, pattern), id: newId(), manual: true, drawn: true, artifact: undefined, added: undefined };
}
export function adjustGrainPreview(grain: Grain, points: Point[]) { return editedOutline(grain, points); }

const cross = (a: Point, b: Point, c: Point) => (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);
function intersects(a: Point, b: Point, c: Point, d: Point) {
  if (Math.max(a[0],b[0]) < Math.min(c[0],d[0]) || Math.max(c[0],d[0]) < Math.min(a[0],b[0])
    || Math.max(a[1],b[1]) < Math.min(c[1],d[1]) || Math.max(c[1],d[1]) < Math.min(a[1],b[1])) return false;
  return cross(a,b,c)*cross(a,b,d) <= 0 && cross(c,d,a)*cross(c,d,b) <= 0;
}
/** Include containment and crossing edges, even when neither centroid is enclosed. */
export function overlappingGrains(preview: Grain, grains: Grain[]): Grain[] {
  const p = preview.polygon_um, b = preview.bbox_um;
  if (p.length < 3) return [];
  return grains.filter(g => {
    const q=g.polygon_um, c=g.bbox_um;
    if (q.length < 3 || b[0]>c[2] || b[2]<c[0] || b[1]>c[3] || b[3]<c[1]) return false;
    return contains(p,q[0]) || contains(q,p[0]) || p.some((a,i)=>q.some((c,j)=>intersects(a,p[(i+1)%p.length],c,q[(j+1)%q.length])));
  });
}

export interface PreviewCore {
  grainAt(area: string, x: number, y: number, params: SegmentParams, pad: {margin_um: number; smooth: boolean}): Promise<Grain | null>;
  snapOutline(area: string, points: Point[], width: number, contrast: number): Promise<Point[]>;
  outlineGrain(area: string, points: Point[], base: Grain | null, op: 'new', pad: {margin_um: number; smooth: boolean}): Promise<Grain | null>;
  refineGrain(area: string, grain: Grain): Promise<Grain>;
}
/** Reuse pixel detection, Outline and GrabCut; return only detached geometry. */
export async function detectGrainPreview(core: PreviewCore | null, area: string, source: AddSource, params: SegmentParams | null,
  sensitivity: number, pixels: boolean, pad: {margin_um: number; smooth: boolean}, width: number): Promise<Grain | null> {
  if (source.kind === 'draw') {
    let points = source.points;
    if (core && pixels && !source.free) {
      const snapped = await core.snapOutline(area, points, width, .08);
      if (snapped.length >= 3) points = snapped;
    }
    return core && pixels ? core.outlineGrain(area, points, null, 'new', source.free ? {margin_um: 0, smooth: false} : pad) : polygonGrain(points);
  }
  if (!pixels || !core) throw new Error('Image pixels are unavailable. Use Draw to add a grain.');
  if (source.kind === 'box') {
    const box = polygonGrain(source.points);
    const result = await core.refineGrain(area, box);
    // Refinement may expand its crop. Keep the promised box boundary.
    const [x0,y0,x1,y1] = box.bbox_um;
    let points = result.polygon_um;
    // Clip edges at their intersections; clamping vertices would distort angled edges.
    for (const [axis, edge, sign] of [[0,x0,1],[0,x1,-1],[1,y0,1],[1,y1,-1]]) {
      const out: Point[] = [];
      for (let i=0; i<points.length; i++) {
        const a=points[i], b=points[(i+1)%points.length];
        const ai=(a[axis]-edge)*sign>=0, bi=(b[axis]-edge)*sign>=0;
        if (ai) out.push(a);
        if (ai !== bi) { const t=(edge-a[axis])/(b[axis]-a[axis]); out.push([a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])]); }
      }
      points=out;
    }
    return polygonGrain(points, result);
  }
  if (!params) throw new Error('Detection is still loading. Try again or use Draw.');
  return core.grainAt(area, ...source.point, { ...params, click_sensitivity: Math.max(.01, Math.min(.99, sensitivity / 100)) }, pad);
}

export interface AddGrainStore {
  areaId: string | null; readOnly: boolean; busy: unknown; grains: Record<string, Grain[]>;
  checkpoint(): void; persist(): void;
}
/** Append only: do not use replacement helpers that merge grains or relink registered spots. */
export function commitGrainPreview(store: AddGrainStore, area: string, grain: Grain): boolean {
  if (store.readOnly || store.busy || store.areaId !== area || (store.grains[area] ?? []).some(g=>g.id===grain.id)) return false;
  editedOutline(grain, grain.polygon_um); // Reject invalid adjusted geometry before checkpointing.
  store.checkpoint();
  store.grains = { ...store.grains, [area]: [...(store.grains[area] ?? []), grain] };
  store.persist();
  return true;
}
