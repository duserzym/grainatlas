// Pointer queries keep source order: strict distance comparisons make the first tie win.
import type { Grain, Spot } from './core/types';
import { snap, snapTargets, spotDistance, type SnapTarget } from './spots.ts';
import { spotDiameter } from './spot-size.ts';
type Bounds = readonly [number, number, number, number];

/** Immutable area index. Rebuild on geometry/list replacement, never on pointer movement. */
export class BoundsGrid<T> {
  private cells = new Map<string, number[]>();
  private broad: number[] = [];
  private items: readonly T[];
  private bounds: (item:T)=>Bounds;
  private size:number;
  constructor(items: readonly T[], bounds: (item: T) => Bounds, size = 128) {
    this.items=items;this.bounds=bounds;this.size=size;
    items.forEach((item, i) => {
      const [a,b,c,d] = this.range(bounds(item));
      if (![a,b,c,d].every(Number.isFinite) || (c-a+1)*(d-b+1)>4096) { this.broad.push(i); return; }
      for (let y=b;y<=d;y++) for (let x=a;x<=c;x++) {
        const key=`${x},${y}`, cell=this.cells.get(key);
        if (cell) cell.push(i); else this.cells.set(key,[i]);
      }
    });
  }
  private range([a,b,c,d]: Bounds) { return [Math.floor(a/this.size),Math.floor(b/this.size),Math.floor(c/this.size),Math.floor(d/this.size)]; }
  query(box: Bounds): T[] {
    const [a,b,c,d]=this.range(box);
    // Zoomed far out: a bounded full scan is cheaper than walking millions of empty cells.
    if (![a,b,c,d].every(Number.isFinite) || (c-a+1)*(d-b+1)>4096) return this.items.filter(item=>this.overlaps(this.bounds(item),box));
    const ids=new Set(this.broad);
    for(let y=b;y<=d;y++) for(let x=a;x<=c;x++) for(const i of this.cells.get(`${x},${y}`)??[]) ids.add(i);
    return [...ids].sort((a,b)=>a-b).map(i=>this.items[i]).filter(item=>this.overlaps(this.bounds(item),box));
  }
  private overlaps(a: Bounds,b: Bounds) { return a[0]<=b[2] && a[2]>=b[0] && a[1]<=b[3] && a[3]>=b[1]; }
}
export class SnapIndex {
  private grid: BoundsGrid<SnapTarget>;
  constructor(grains: Grain[], diameter: number) {
    this.grid=new BoundsGrid(snapTargets(grains,null,diameter),t=>[t.x,t.y,t.x,t.y]);
  }
  nearest(x:number,y:number,radius:number,previous:Spot|null):SnapTarget|null {
    const candidates=this.grid.query([x-radius,y-radius,x+radius,y+radius]);
    // Previous spot was last in the original scan, including exact ties with grain targets.
    if(previous) candidates.push({x:previous.x_um,y:previous.y_um,what:`previous spot ${previous.name}`});
    return snap(x,y,candidates,radius);
  }
}
export function spotBounds(s:Spot):Bounds {
  const radius=(spotDiameter(s)??0)/2;
  if(spotDiameter(s)===null) return [s.x_um,s.y_um,s.x_um,s.y_um];
  const x2=s.kind==='spot'?s.x_um:s.x2_um??s.x_um, y2=s.kind==='spot'?s.y_um:s.y2_um??s.y_um;
  const r=s.kind==='raster' && s.x2_um!=null && s.y2_um!=null?0:radius;
  return [Math.min(s.x_um,x2)-r,Math.min(s.y_um,y2)-r,Math.max(s.x_um,x2)+r,Math.max(s.y_um,y2)+r];
}
/** Original scoring and slack, shared by indexed queries and brute-force regression checks. */
export function nearestSpot(spots:readonly Spot[],x:number,y:number,k:number):Spot|null {
  let best:Spot|null=null, score=Infinity;
  for(const s of spots) {
    const d=spotDistance(s,x,y,5*k), radius=spotDiameter(s)===null?5*k:spotDiameter(s)!/2;
    let next:number;
    if(d<=0) {
      const cx=s.kind==='spot'?s.x_um:(s.x_um+(s.x2_um??s.x_um))/2;
      const cy=s.kind==='spot'?s.y_um:(s.y_um+(s.y2_um??s.y_um))/2;
      next=Math.hypot(x-cx,y-cy)/Math.max(radius,1e-6)/1e3;
    } else if(d<=8*k) next=1+d/k; else continue;
    if(next<score) { score=next;best=s; }
  }
  return best;
}
export class SpotIndex {
  private grid:BoundsGrid<Spot>;
  constructor(spots:Spot[]) { this.grid=new BoundsGrid(spots,spotBounds); }
  hit(x:number,y:number,k:number):Spot|null {
    // Missing sizes are points in the index and have a 5 px marker plus 8 px slack.
    const r=13*k;
    return nearestSpot(this.grid.query([x-r,y-r,x+r,y+r]),x,y,k);
  }
}
/** Latest hover/live move per frame. Flush before press/release, discard on cancel/context change. */
export function latestFrame<T>(run:(value:T)=>void, request:(fn:()=>void)=>number=requestAnimationFrame, cancel:(id:number)=>void=cancelAnimationFrame) {
  let frame:number|null=null, pending:{value:T}|null=null;
  const clear=()=>{ if(frame!==null) cancel(frame);frame=null; };
  const flush=()=>{ clear();const next=pending;pending=null;if(next) run(next.value); };
  return { push(value:T) { pending={value};if(frame===null) frame=request(flush); }, flush, clear() {clear();pending=null;} };
}
