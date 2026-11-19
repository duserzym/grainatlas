import type { PiecePlacement, SheetPlan, SheetRect, SheetStrip, StripKey } from './core/types.ts';
import { keyId } from './strip-edit.ts';
export type Point = [number, number];
export type Matrix = [number, number, number, number, number, number];
/** Labels identify area pieces across all pages, in the plan's stable order. */
export function pieceLabels(strips: SheetStrip[]): Map<string, string> {
  const areas = new Map<string, SheetStrip[]>(), labels = new Map<string, string>();
  for (const s of strips) if (!s.hidden) {
    const pieces = areas.get(s.key.area) ?? [];
    pieces.push(s); areas.set(s.key.area, pieces);
  }
  for (const pieces of areas.values()) pieces.forEach((s, i) => {
    labels.set(keyId(s.key), `${s.sample} · ${s.label}${pieces.length > 1 ? ` · piece ${i + 1} of ${pieces.length}` : ''}`);
  });
  return labels;
}
/** Round the field display only. Small scales use scientific notation to stay nonzero. */
export function formatPieceScale(value: number): string {
  return value >= 0.01 ? String(Number(value.toFixed(2))) : value.toExponential(2).replace(/\.?0+e/, 'e');
}
export const point = (m: Matrix, p: Point): Point => [m[0]*p[0]+m[2]*p[1]+m[4], m[1]*p[0]+m[3]*p[1]+m[5]];
export function bounds(m: Matrix, r: SheetRect): SheetRect {
  const ps = [[r[0],r[1]],[r[2],r[1]],[r[2],r[3]],[r[0],r[3]]].map(p => point(m,p as Point));
  return [Math.min(...ps.map(p=>p[0])),Math.min(...ps.map(p=>p[1])),Math.max(...ps.map(p=>p[0])),Math.max(...ps.map(p=>p[1]))];
}
export function pieceMatrix(s: SheetStrip): Matrix {
  const { scale = 1, rotation_deg = 0 } = s.transform ?? {};
  const a = rotation_deg*Math.PI/180, c = Math.cos(a)*scale, sn = Math.sin(a)*scale;
  const m: Matrix = [c,sn,-sn,c,0,0];
  m[4] = s.origin_mm[0]-Math.min(0,c*s.size_mm[0])-Math.min(0,-sn*s.size_mm[1]);
  m[5] = s.origin_mm[1]-Math.min(0,sn*s.size_mm[0])-Math.min(0,c*s.size_mm[1]); return m;
}
export function pieceRect(s: SheetStrip): SheetRect {
  const {scale=1,rotation_deg=0}=s.transform??{}, a=rotation_deg*Math.PI/180;
  const w=(Math.abs(Math.cos(a))*s.size_mm[0]+Math.abs(Math.sin(a))*s.size_mm[1])*scale;
  const h=(Math.abs(Math.sin(a))*s.size_mm[0]+Math.abs(Math.cos(a))*s.size_mm[1])*scale;
  return [...s.origin_mm,s.origin_mm[0]+w,s.origin_mm[1]+h];
}
export function groupRect(strips: SheetStrip[]): SheetRect {
  if(!strips.length)return [0,0,0,0];
  const result:SheetRect=[Infinity,Infinity,-Infinity,-Infinity];
  for(const s of strips){const r=pieceRect(s);for(const d of [0,1]){result[d]=Math.min(result[d],r[d]);result[d+2]=Math.max(result[d+2],r[d+2]);}}
  return result;
}
export function placement(s: SheetStrip): PiecePlacement { return { strip:s.key, sheet:s.sheet, origin_mm:[...s.origin_mm], scale:s.transform?.scale ?? 1, rotation_deg:s.transform?.rotation_deg ?? 0 }; }
export function placed(s: SheetStrip, p: PiecePlacement): SheetStrip { return { ...s, origin_mm:p.origin_mm, sheet:p.sheet, transform:{scale:p.scale,rotation_deg:p.rotation_deg} }; }
export function groupScale(strips: SheetStrip[], factor: number, anchor: Point = groupRect(strips).slice(0,2) as Point): PiecePlacement[] {
  return strips.map(s => ({ ...placement(s), scale:(s.transform?.scale ?? 1)*factor, origin_mm:[anchor[0]+(s.origin_mm[0]-anchor[0])*factor,anchor[1]+(s.origin_mm[1]-anchor[1])*factor] }));
}
export function groupMove(strips: SheetStrip[], delta: Point): PiecePlacement[] { return strips.map(s=>({...placement(s),origin_mm:[s.origin_mm[0]+delta[0],s.origin_mm[1]+delta[1]]})); }
export function nudge(strips: SheetStrip[], direction: Point, shift = false): PiecePlacement[] { return groupMove(strips,direction.map(v=>v*(shift?10:1)) as Point); }
export function clampGroup(strips: SheetStrip[], body: SheetRect): Point {
  const r=groupRect(strips);
  return [Math.max(body[0]-r[0],Math.min(0,body[2]-r[2])),Math.max(body[1]-r[1],Math.min(0,body[3]-r[3]))];
}
export function scaleBar(k: number, width = 20): number {
  const max=k*width, p=10**Math.floor(Math.log10(max));
  return [5,2,1].map(v=>v*p).find(v=>v<=max*(1+1e-12)) ?? p;
}
export interface Guide { axis: 0 | 1; at: number }
/** Edges and centres, margins and the packer's 2.5 mm clearance. Stable nearest wins. */
export function snapGroup(plan: SheetPlan, strips: SheetStrip[], tolerance = 2): { delta: Point; guides: Guide[] } {
  const r=groupRect(strips), body=plan.sheet_bodies[strips[0].sheet].body_rect_mm;
  const delta: Point=[0,0], guides: Guide[]=[];
  const ids=new Set(strips.map(s=>keyId(s.key)));
  for (const d of [0,1] as const) {
    const from=[r[d],(r[d]+r[d+2])/2,r[d+2]], targets=[body[d],(body[d]+body[d+2])/2,body[d+2]];
    for (const s of plan.strips.filter(s=>!s.hidden && s.sheet===strips[0].sheet && !ids.has(keyId(s.key)))) {
      const b=pieceRect(s); targets.push(b[d],(b[d]+b[d+2])/2,b[d+2],b[d]-2.5,b[d+2]+2.5);
    }
    let best:{shift:number;at:number}|undefined;
    for(const at of targets)for(const v of from){const shift=at-v;if(Math.abs(shift)<=tolerance&&(!best||Math.abs(shift)<Math.abs(best.shift)))best={shift,at};}
    if(best){delta[d]=best.shift;guides.push({axis:d,at:best.at});}
  }
  return {delta,guides};
}
export function align(strips: SheetStrip[], axis: 0 | 1, fraction: number): PiecePlacement[] {
  const group=groupRect(strips), to=group[axis]+(group[axis+2]-group[axis])*fraction;
  return strips.map(s=>{const r=pieceRect(s),p=placement(s);p.origin_mm[axis]+=to-r[axis]-(r[axis+2]-r[axis])*fraction;return p;});
}
export function distribute(strips: SheetStrip[]): PiecePlacement[] {
  const r=groupRect(strips), axis:0|1=(r[2]-r[0]>=r[3]-r[1]?0:1);
  const sorted=[...strips].sort((a,b)=>a.origin_mm[axis]-b.origin_mm[axis]);
  const total=sorted.reduce((n,s)=>{const b=pieceRect(s);return n+b[axis+2]-b[axis];},0);
  const gap=(r[axis+2]-r[axis]-total)/(sorted.length-1);
  let at=r[axis];
  return sorted.map(s=>{const p=placement(s),b=pieceRect(s);p.origin_mm[axis]=at;at+=b[axis+2]-b[axis]+gap;return p;});
}
export function chosen(plan:SheetPlan,keys:StripKey[]):SheetStrip[]{const ids=new Set(keys.map(keyId));return plan.strips.filter(s=>!s.hidden&&ids.has(keyId(s.key)));}
