import type { Grain } from './core/types';
import { polygonGrain, type Point } from './polygon-edit.ts';

/** Preserve identity/metadata; only the explicitly edited geometry and derived measurements change. */
export function editedOutline(base:Grain, points:Point[]):Grain {
  const geometry=polygonGrain(points,base); // validates area and self intersections
  if(points.length<3 || points.some((p,i)=>!p.every(Number.isFinite) || Math.hypot(p[0]-points[(i+1)%points.length][0],p[1]-points[(i+1)%points.length][1])<1e-7)) throw new Error('Keep distinct outline corners.');
  for(let i=0;i<points.length;i++) {
    const a=points[(i+points.length-1)%points.length],b=points[i],c=points[(i+1)%points.length];
    const u:[number,number]=[b[0]-a[0],b[1]-a[1]],v:[number,number]=[c[0]-b[0],c[1]-b[1]];
    if(Math.abs(u[0]*v[1]-u[1]*v[0])<1e-7 && u[0]*v[0]+u[1]*v[1]<0) throw new Error('The outline doubles back on itself.');
  }
  // Retain collinear corners added deliberately; the measurement helper simplifies its ring.
  return {...geometry,polygon_um:points.map(p=>[...p] as Point)};
}
/** Nearest vertex first, then nearest edge projection. Stable ties follow ring order. */
export function outlineHandle(points:Point[],at:Point,radius:number):{kind:'vertex'|'edge';index:number;point:Point}|null {
  let best=radius, hit:ReturnType<typeof outlineHandle>=null;
  for(let i=0;i<points.length;i++) {
    const distance=Math.hypot(points[i][0]-at[0],points[i][1]-at[1]);
    if(distance<best) {best=distance;hit={kind:'vertex',index:i,point:points[i]};}
  }
  if(hit) return hit;
  for(let i=0;i<points.length;i++) {
    const a=points[i],b=points[(i+1)%points.length],dx=b[0]-a[0],dy=b[1]-a[1];
    const t=Math.max(0,Math.min(1,((at[0]-a[0])*dx+(at[1]-a[1])*dy)/(dx*dx+dy*dy||1)));
    const point:Point=[a[0]+t*dx,a[1]+t*dy],distance=Math.hypot(point[0]-at[0],point[1]-at[1]);
    if(distance<best) {best=distance;hit={kind:'edge',index:i,point};}
  }
  return hit;
}
