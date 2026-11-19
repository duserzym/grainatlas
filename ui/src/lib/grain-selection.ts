import type { Grain } from './core/types';
import { toMosaic, toScreen, type View } from './canvas/view.ts';

type Point = readonly [number, number];
type Bounds = readonly [number, number, number, number];
export interface SelectionBox {
  bounds: Bounds;
  contains(x: number, y: number): boolean;
}

/** Convert once on release. Keep the rotated box, not just its axis-aligned bounds. */
export function canvasBox(view: View, umPerPx: number, start: Point, end: Point): SelectionBox {
  const left = Math.min(start[0], end[0]), right = Math.max(start[0], end[0]);
  const top = Math.min(start[1], end[1]), bottom = Math.max(start[1], end[1]);
  const points = [[left,top],[right,top],[right,bottom],[left,bottom]].map(([x,y]) => toMosaic(view,x,y).map(n => n*umPerPx));
  const eps=1e-4, margin=eps*umPerPx/view.zoom+1e-9;
  return {
    bounds: [Math.min(...points.map(p=>p[0]))-margin,Math.min(...points.map(p=>p[1]))-margin,Math.max(...points.map(p=>p[0]))+margin,Math.max(...points.map(p=>p[1]))+margin],
    contains(px,py) { const [x,y]=toScreen(view,px/umPerPx,py/umPerPx); return x>=left-eps && x<=right+eps && y>=top-eps && y<=bottom+eps; },
  };
}

const grids = new WeakMap<readonly Grain[], GrainGrid>();

/** Grain arrays are replaced on edits. Cache their bounding-box index by identity. */
export function grainGrid(grains: readonly Grain[]): GrainGrid {
  let grid = grids.get(grains);
  if (!grid) { grid = new GrainGrid(grains); grids.set(grains, grid); }
  return grid;
}

class GrainGrid {
  private readonly grains: readonly Grain[];
  private readonly cells = new Map<number, number[]>();
  private readonly size: number;
  private readonly left: number;
  private readonly top: number;
  private readonly columns: number;
  private readonly rows: number;

  constructor(grains: readonly Grain[]) {
    this.grains = grains;
    let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
    for (const g of grains) {
      left = Math.min(left, g.bbox_um[0]); top = Math.min(top, g.bbox_um[1]);
      right = Math.max(right, g.bbox_um[2]); bottom = Math.max(bottom, g.bbox_um[3]);
    }
    this.left = grains.length ? left : 0; this.top = grains.length ? top : 0;
    // Bound the number of cells even for long, thin mounts or very large grains.
    this.size = Math.max(1, Math.max(right - left, bottom - top) / Math.min(128, Math.ceil(Math.sqrt(grains.length))));
    this.columns = grains.length ? Math.floor((right - left) / this.size) + 1 : 1;
    this.rows = grains.length ? Math.floor((bottom - top) / this.size) + 1 : 1;
    grains.forEach((g, index) => {
      const [x0, y0, x1, y1] = this.cellsFor(g.bbox_um);
      for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) {
        const key = y * this.columns + x;
        const cell = this.cells.get(key);
        if (cell) cell.push(index); else this.cells.set(key, [index]);
      }
    });
  }

  private cellsFor([left, top, right, bottom]: Bounds): Bounds {
    return [Math.max(0, Math.floor((left - this.left) / this.size)), Math.max(0, Math.floor((top - this.top) / this.size)),
      Math.min(this.columns - 1, Math.floor((right - this.left) / this.size)), Math.min(this.rows - 1, Math.floor((bottom - this.top) / this.size))];
  }

  query(box: SelectionBox): Grain[] {
    const [x0, y0, x1, y1] = this.cellsFor(box.bounds), candidates = new Set<number>();
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) {
      for (const index of this.cells.get(y * this.columns + x) ?? []) candidates.add(index);
    }
    return [...candidates].sort((a, b) => a - b).map(i => this.grains[i]).filter(g => box.contains(...g.centroid_um));
  }
}
