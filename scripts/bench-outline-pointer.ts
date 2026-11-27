// Reproducible synthetic CPU-only comparison. Run: node --experimental-strip-types scripts/bench-outline-pointer.ts
// No DOM/paint, server requests or real data; reports distributions without a timing assertion.
import { performance } from 'node:perf_hooks';
import { BoundsGrid, SnapIndex, SpotIndex } from '../ui/src/lib/canvas-pointer.ts';
import { grainAt, snap, snapTargets, spotDistance } from '../ui/src/lib/spots.ts';
import { spotDiameter } from '../ui/src/lib/spot-size.ts';
import type { Grain, Spot } from '../ui/src/lib/core/types';
const grains:Grain[]=Array.from({length:300},(_,i)=>{
  const x=(i%20)*140+70,y=Math.floor(i/20)*120+60;
  const polygon_um=Array.from({length:64},(_,j)=>[x+50*Math.cos(j*Math.PI/32),y+35*Math.sin(j*Math.PI/32)] as [number,number]);
  return {id:`g-${i}`,number:i+1,polygon_um,centroid_um:[x,y],bbox_um:[x-50,y-35,x+50,y+35],area_um2:5500,major_um:100,minor_um:70,orientation_deg:i%180,eq_diameter_um:84,scale_bar_um:10,mean_intensity:128,manual:false};
});
const spots:Spot[]=Array.from({length:500},(_,i)=>({id:`s-${i}`,area_id:'a',name:`synthetic-${i}`,sample:'synthetic',grain_id:null,grain_number:null,n:i,x_um:i%25*110+30,y_um:Math.floor(i/25)*90+30,x2_um:i%25*110+80,y2_um:Math.floor(i/25)*90+80,diameter_um:20,kind:i%5===0?'line':i%7===0?'raster':'spot',created:'',created_by:'',comment:''}));
const moves=Array.from({length:120},(_,i)=>[20+(i*137)%2760,20+(i*83)%1740]);
const previous=spots.at(-1)!,diameter=20,k=1;
function originalHit(x:number,y:number) {
  let best:Spot|null=null,bs=Infinity;
  for(const s of spots) {
    const d=spotDistance(s,x,y,5*k);let score:number;
    if(d<=0) {const cx=s.kind==='spot'?s.x_um:(s.x_um+(s.x2_um??s.x_um))/2,cy=s.kind==='spot'?s.y_um:(s.y_um+(s.y2_um??s.y_um))/2;score=Math.hypot(x-cx,y-cy)/Math.max(spotDiameter(s)!/2,1e-6)/1e3;}
    else if(d<=8*k)score=1+d/k;else continue;
    if(score<bs){bs=score;best=s;}
  } return best;
}
const start=performance.now(),snapIndex=new SnapIndex(grains,diameter),spotIndex=new SpotIndex(spots),grainIndex=new BoundsGrid(grains,g=>g.bbox_um),buildMs=performance.now()-start;
const before=(x:number,y:number)=>[originalHit(x,y)?.id??grainAt(grains,x,y)?.id??null,snap(x,y,snapTargets(grains,previous,diameter),12*k)];
const after=(x:number,y:number)=>[spotIndex.hit(x,y,k)?.id??grainAt(grainIndex.query([x,y,x,y]),x,y)?.id??null,snapIndex.nearest(x,y,12*k,previous)];
for(const [x,y] of moves) if(JSON.stringify(before(x,y))!==JSON.stringify(after(x,y)))throw new Error(`Query mismatch ${x},${y}`);
let sink=0;
function measure(run:typeof before) {
  const samples:number[]=[];
  for(let warm=0;warm<30;warm++)for(const [x,y] of moves)sink+=run(x,y).length;
  for(let round=0;round<100;round++)for(const [x,y] of moves){const t=performance.now();sink+=run(x,y).length;samples.push(performance.now()-t);}
  samples.sort((a,b)=>a-b);return {n:samples.length,medianMs:samples[Math.floor(samples.length*.5)],p95Ms:samples[Math.floor(samples.length*.95)],maxMs:samples.at(-1),sumMs:samples.reduce((a,b)=>a+b,0)};
}
console.log(JSON.stringify({node:process.version,grains:grains.length,vertices:64,spots:spots.length,moves:moves.length,buildMs,before:measure(before),after:measure(after),note:'CPU-only original query loop versus indexed loop. Excludes DOM, Svelte, contrast/paint; no browser frame claims. Both execute every move, without credit for frame coalescing.',sink},null,2));
