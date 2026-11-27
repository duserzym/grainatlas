import { describe, it, expect } from 'vitest';
import { emptyAddPreview, previewTransition, nextGrainName, overlappingGrains, namedNewGrain, detectGrainPreview, commitGrainPreview, adjustGrainPreview, type AddSource, type PreviewCore } from './add-grain';
import { polygonGrain } from './polygon-edit';
import type { Grain, SegmentParams } from './core/types';
const rect = (x=0,y=0,w=10,h=10) => polygonGrain([[x,y],[x+w,y],[x+w,y+h],[x,y+h]]);
const click: AddSource = {kind:'click',point:[5,5]};
const params = {threshold:null} as SegmentParams;
const pad = {margin_um:0,smooth:false};
const copy = <T>(v:T):T => structuredClone(v);

describe('Add grain preview lifecycle',()=>{
  it('isolates requests, invalidates cancellation and ignores stale completion',()=>{
    const idle=emptyAddPreview(), a=previewTransition(idle,{type:'begin',source:click});
    const b=previewTransition(a,{type:'begin',source:click});
    expect(previewTransition(b,{type:'result',revision:a.revision,grain:rect()})).toBe(b);
    const cancelled=previewTransition(b,{type:'cancel'});
    expect(previewTransition(cancelled,{type:'result',revision:b.revision,grain:rect()})).toBe(cancelled);
    expect(cancelled.status).toBe('idle'); expect(cancelled.grain).toBeNull(); expect(idle.revision).toBe(0);
  });
  it('keeps vertex edits detached and validates crossings before adjustment',()=>{
    const grain=rect(), before=copy(grain), begin=previewTransition(emptyAddPreview(),{type:'begin',source:click});
    const ready=previewTransition(begin,{type:'result',revision:begin.revision,grain});
    const adjusted=adjustGrainPreview(grain,[[-2,-2],[10,0],[10,10],[0,10]]);
    const next=previewTransition(ready,{type:'adjust',grain:adjusted});
    expect(next.grain?.polygon_um[0]).toEqual([-2,-2]); expect(grain).toEqual(before);
    expect(()=>adjustGrainPreview(grain,[[0,0],[10,10],[0,10],[10,0]])).toThrow();
    expect(previewTransition(begin,{type:'adjust',grain:adjusted})).toBe(begin);
  });
  it('keeps a failed attempt retryable and clears error on a new attempt',()=>{
    const a=previewTransition(emptyAddPreview(),{type:'begin',source:click});
    const b=previewTransition(a,{type:'result',revision:a.revision,grain:null,error:'background'});
    expect(b.status).toBe('idle'); expect(b.error).toBe('background');
    expect(previewTransition(b,{type:'begin',source:click}).error).toBe('');
  });
});

describe('next grain defaults',()=>{
  it('uses the current sample and pattern with the next area grain number',()=>{
    const existing={...rect(),number:12,label:'keep'};
    expect(nextGrainName([existing], 'Area A', '{sample}_{grain}_{n:03}')).toEqual({number:13,label:'Area A_13_013',sample:'Area A'});
    expect(nextGrainName([], 'Area B', '{sample}_{n}')).toEqual({number:1,label:'Area B_1',sample:'Area B'});
  });
  it('keeps names unique even with a constant pattern and includes artifact numbers',()=>{
    const g={...rect(),number:4,label:'fixed',artifact:'manual' as const};
    expect(nextGrainName([g], 'A', 'fixed')).toEqual({number:5,label:'fixed (5)',sample:'A'});
    const newGrain=namedNewGrain(g,[g],'A','{sample}_{n}');
    expect(newGrain.id).not.toBe(g.id); expect(newGrain.number).toBe(5); expect(newGrain.artifact).toBeUndefined();
    expect(g.label).toBe('fixed'); expect(newGrain.drawn).toBe(true);
  });
});

describe('overlap warnings',()=>{
  it('finds crossing edges without enclosed centroids, containment and touch',()=>{
    const horizontal=rect(-10,-1,20,2), vertical=rect(-1,-10,2,20), inside=rect(0,0,1,1), touching=rect(10,0,2,2), distant=rect(11,3);
    expect(overlappingGrains(horizontal,[vertical,inside,touching,distant])).toEqual([vertical,inside,touching]);
    expect(overlappingGrains(inside,[horizontal])).toEqual([horizontal]);
  });
  it('does not warn for disjoint polygons with overlapping bounds',()=>{
    const a=polygonGrain([[0,0],[10,0],[0,10]]), b=polygonGrain([[10,10],[10,2],[2,10]]);
    expect(overlappingGrains(a,[b])).toEqual([]);
  });
});

function detector() {
  const calls:unknown[][]=[];
  const core:PreviewCore={
    async grainAt(...args){calls.push(['click',...args]);return rect();},
    async snapOutline(...args){calls.push(['snap',...args]);return args[1];},
    async outlineGrain(...args){calls.push(['outline',...args]);return polygonGrain(args[1]);},
    async refineGrain(...args){calls.push(['refine',...args]);return rect(2,2,5,5);},
  }; return {calls,core};
}
describe('shared segmentation routes',()=>{
  it('reruns click-grow with sensitivity without mutating area segmentation settings',async()=>{
    const {core,calls}=detector(), before=copy(params);
    await detectGrainPreview(core,'a',click,params,50,true,pad,10);
    await detectGrainPreview(core,'a',click,params,80,true,pad,10);
    expect((calls[0][4] as SegmentParams).click_sensitivity).toBe(.5);
    expect((calls[1][4] as SegmentParams).click_sensitivity).toBe(.8); expect(params).toEqual(before);
  });
  it('uses Outline for polygons and freehand, bypassing snapping for a free path',async()=>{
    const {core,calls}=detector(), points=rect().polygon_um;
    await detectGrainPreview(core,'a',{kind:'draw',points,free:false},params,50,true,pad,10);
    await detectGrainPreview(core,'a',{kind:'draw',points,free:true},params,50,true,pad,10);
    expect(calls.map(c=>c[0])).toEqual(['snap','outline','outline']);
    expect(calls.every(c=>c[0]!=='outline'||c[3]===null&&c[4]==='new')).toBe(true);
  });
  it('uses geometry directly without pixels and explains pixel-only modes',async()=>{
    const {core,calls}=detector();
    expect(await detectGrainPreview(core,'a',{kind:'draw',points:rect().polygon_um,free:false},null,50,false,pad,10)).toMatchObject({area_um2:100});
    expect(calls).toEqual([]);
    await expect(detectGrainPreview(core,'a',click,params,50,false,pad,10)).rejects.toThrow('Image pixels are unavailable');
  });
  it('clips refined edges at box intersections instead of clamping angled vertices',async()=>{
    const {core}=detector();
    core.refineGrain=async()=>polygonGrain([[-10,0],[10,0],[10,20]]);
    const result=await detectGrainPreview(core,'a',{kind:'box',points:rect().polygon_um},params,50,true,pad,10);
    expect(result?.bbox_um).toEqual([0,0,10,10]);
    expect(result?.area_um2).toBe(100);
  });
  it('refines only a detached box, preserving all existing records',async()=>{
    const {core,calls}=detector(), box=rect(), before=copy(box);
    const result=await detectGrainPreview(core,'a',{kind:'box',points:box.polygon_um},params,50,true,pad,10);
    expect(calls.map(c=>c[0])).toEqual(['refine']); expect(result?.bbox_um).toEqual([2,2,7,7]); expect(box).toEqual(before);
  });
});

describe('append transaction',()=>{
  it('checkpoints exactly once, appends only, persists once, and refuses invalid/stale/read-only commits',()=>{
    const old=rect(), next=namedNewGrain(rect(5,5),[old],'A','{sample}_{n}');
    let checkpoints=0,persists=0;
    const store={areaId:'a',readOnly:false,busy:false,grains:{a:[old],b:[rect()]} as Record<string,Grain[]>,checkpoint(){checkpoints++;},persist(){persists++;}};
    const b=store.grains.b;
    expect(commitGrainPreview(store,'a',next)).toBe(true); expect(checkpoints).toBe(1); expect(persists).toBe(1);
    expect(store.grains.a).toEqual([old,next]); expect(store.grains.a[0]).toBe(old); expect(store.grains.b).toBe(b);
    expect(commitGrainPreview(store,'a',next)).toBe(false);
    store.readOnly=true; expect(commitGrainPreview(store,'a',rect())).toBe(false);
    store.readOnly=false; expect(commitGrainPreview(store,'b',rect())).toBe(false);
    expect(()=>commitGrainPreview(store,'a',{...rect(),polygon_um:[[0,0],[1,1],[0,1],[1,0]]})).toThrow();
    expect(checkpoints).toBe(1); expect(persists).toBe(1);
  });
});
