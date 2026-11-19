import { expect, it } from 'vitest';
import { emptyImageEdits, imagePointVisible, imageTransform, levelsLut, printableGrains, applyAffine, inverseAffine, readImageEdits, overviewHistogram } from './image-edits';
import { toMosaic, toScreen, matrix, zoomAt, rotateAt, panBy, type View } from './canvas/view';
import { canvasBox } from './grain-selection';
import type { Grain } from './core/types';
import { pixelSignature, comparablePixelKey } from './sync-pixels';

it('bounds published histogram work to four overview tiles and stops stale work', async () => {
  const calls:number[][]=[];
  const source={async pyramidTile(_area:string,level:number,col:number,row:number){calls.push([level,col,row]);return {data:new Uint8Array([10,20,20,30])};}};
  const mosaic={levels:[{level:8,w:10000,h:10000,cols:100,rows:100,scale:256}]} as any;
  const start=performance.now(), hist=await overviewHistogram(source,'a',mosaic);
  expect(calls).toEqual([[8,0,0],[8,99,0],[8,0,99],[8,99,99]]);expect(hist?.[20]).toBe(8);
  console.info(JSON.stringify({overviewTiles:10000,histogramRequests:4,ms:Number((performance.now()-start).toFixed(2))}));
  expect(await overviewHistogram(source,'a',mosaic,()=>true)).toBeNull();expect(calls).toHaveLength(4);
});

it('maps pointer coordinates back to mount µm under rotation, flips and navigation', () => {
  for (const angle of [0, 90, -90, 180, 31.5, -42]) for (const flip_h of [true,false]) for (const flip_v of [true,false]) {
    const e={...emptyImageEdits(),rotate_deg:angle,flip_h,flip_v,crop:{kind:'rect' as const,points_um:[[10,20],[250,180]] as [number,number][]}};
    const u=1.654;
    let v:View={cx:150,cy:90,zoom:2.1,rot:0.23,w:800,h:600,image:imageTransform(e,[400,300],u)};
    const p:[number,number]=[173.125,81.25], px=p.map(n=>n/u) as [number,number];
    for (let step=0;step<3;step++) {
      const screen=toScreen(v,...px), recovered=toMosaic(v,...screen).map(n=>n*u);
      expect(recovered[0]).toBeCloseTo(p[0],10);expect(recovered[1]).toBeCloseTo(p[1],10);
      expect(applyAffine(matrix(v),px)[0]).toBeCloseTo(screen[0],10);expect(applyAffine(matrix(v),px)[1]).toBeCloseTo(screen[1],10);
      const z=zoomAt(v,...screen,1.4);expect(toScreen(z,...px)[0]).toBeCloseTo(screen[0],10);expect(toScreen(z,...px)[1]).toBeCloseTo(screen[1],10);
      const r=rotateAt(z,...screen,0.2);expect(toScreen(r,...px)[0]).toBeCloseTo(screen[0],10);expect(toScreen(r,...px)[1]).toBeCloseTo(screen[1],10);
      v=panBy(r,17,-9);
    }
    const screen=toScreen(v,...px), box=canvasBox(v,u,[screen[0]-5,screen[1]-5],[screen[0]+5,screen[1]+5]);
    expect(box.contains(...p)).toBe(true);expect(box.contains(p[0]+100,p[1]+100)).toBe(false);
  }
});
it('matches crop, polygon boundaries, capsule masks and levels independently of orientation', () => {
  const e={...emptyImageEdits(),crop:{kind:'polygon' as const,points_um:[[0,0],[100,0],[100,100],[0,100]] as [number,number][]},masks:[{points_um:[[20,50],[80,50]] as [number,number][],radius_um:4}]};
  expect(imagePointVisible([0,0],e)).toBe(true);expect(imagePointVisible([50,20],e)).toBe(true);expect(imagePointVisible([50,52],e)).toBe(false);expect(imagePointVisible([110,20],e)).toBe(false);
  e.rotate_deg=90;e.flip_h=true;expect(imagePointVisible([50,20],e)).toBe(true);
  const lut=Uint8Array.from({length:256},(_,i)=>i), before=lut.slice();
  expect([...levelsLut(lut,{...e,levels:{black:20,white:220}})].filter((_,i)=>[0,20,120,220,255].includes(i))).toEqual([0,0,128,255,255]);expect(lut).toEqual(before);
  const m=imageTransform(e,[200,150]);const p:[number,number]=[17,42];expect(applyAffine(inverseAffine(m),applyAffine(m,p))[0]).toBeCloseTo(p[0],10);
});
it('filters 10000 grains locally without changing source geometry', () => {
  const grains=Array.from({length:10000},(_,i)=>({id:`g${i}`,centroid_um:[i%100,Math.floor(i/100)] as [number,number],polygon_um:[[i,0],[i+1,0],[i,1]]})) as Grain[];
  const before=JSON.stringify(grains),e={...emptyImageEdits(),crop:{kind:'rect' as const,points_um:[[0,0],[49,99]] as [number,number][]}};
  const start=performance.now();expect(printableGrains(grains,e)).toHaveLength(5000);const ms=performance.now()-start;
  console.info(JSON.stringify({grains:10000,printable:5000,filterMs:Number(ms.toFixed(2)),addedRequests:0}));
  expect(JSON.stringify(grains)).toBe(before);
  expect(readImageEdits({version:2,areas:{}})).toBeNull();expect(readImageEdits({version:1,areas:{a:{...e,rotate_deg:NaN}}})).toBeNull();
});
it('includes recipes in the existing publication outbox key and preserves legacy keys', () => {
  const area={id:'a',mosaic:{sha256:'pixels',w:100,h:100,um_per_px:1},params:{tone:{gamma:1}}};
  const project={areas:[area]} as any;
  const legacy=pixelSignature(project);expect(comparablePixelKey(legacy)).toBe(legacy);
  const edited={...project,image_edits:{version:1,areas:{a:{...emptyImageEdits(),rotate_deg:90}}}};
  const key=pixelSignature(edited);expect(key).not.toBe(legacy);expect(comparablePixelKey(key)).toBe(key);expect(pixelSignature(JSON.parse(JSON.stringify(edited)))).toBe(key);
});
