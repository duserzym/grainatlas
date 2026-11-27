import { describe, it, expect } from 'vitest';
import type { Grain, Spot } from './core/types';
import { BoundsGrid, SnapIndex, SpotIndex, nearestSpot, latestFrame } from './canvas-pointer';
import { grainAt, snap, snapTargets } from './spots';
import { editedOutline, outlineHandle } from './outline-vertices';
const grain=(i:number):Grain=>({id:`grain-${i}`,number:i+1,polygon_um:[[i*100,0],[i*100+80,0],[i*100+80,60],[i*100,60]],bbox_um:[i*100,0,i*100+80,60],centroid_um:[i*100+40,30],major_um:80,minor_um:60,orientation_deg:i%180,area_um2:4800,eq_diameter_um:78,mean_intensity:50,scale_bar_um:10,manual:false});
const spot=(i:number):Spot=>({id:`spot-${i}`,area_id:'a',name:`synthetic-${i}`,sample:'synthetic',grain_id:null,grain_number:null,n:i,x_um:i*20-100,y_um:(i%9)*30,diameter_um:i%7===0?null as unknown as number:10+i%40,kind: ['spot','line','raster'][i%3] as Spot['kind'],x2_um:i*20-150,y2_um:(i%9)*30+50,comment:'',created:'',created_by:''});
function random() { let n=123456789; return ()=>{n=(Math.imul(n,1664525)+1013904223)>>>0;return n/2**32;}; }
describe('pointer indices versus ordered brute force',()=>{
  it('matches snaps over 300 grains, sizes, negative coordinates, rotated tips and previous spot',()=>{
    const gs=Array.from({length:300},(_,i)=>grain(i)), rand=random();
    for(const diameter of [1,20,80,500]) {
      const index=new SnapIndex(gs,diameter),previous=spot(14),targets=snapTargets(gs,previous,diameter);
      for(let i=0;i<700;i++) {
        const x=rand()*30500-100,y=rand()*180-60,radius=rand()*130;
        expect(index.nearest(x,y,radius,previous)).toEqual(snap(x,y,targets,radius));
      }
    }
  });
  it('preserves strict threshold, first ties, previous ordering and radius zero',()=>{
    const a=grain(0),b={...grain(1),centroid_um:[60,30] as [number,number]},index=new SnapIndex([a,b],80);
    const previous={...spot(1),x_um:40,y_um:30};
    expect(index.nearest(50,30,10,previous)).toBeNull();
    expect(index.nearest(50,30,11,previous)?.what).toBe('grain 1 centre');
    expect(index.nearest(40,30,0,previous)).toBeNull();
    expect(index.nearest(40,30,1,previous)?.what).toBe('grain 1 centre');
    expect(index.nearest(-20,30,1,{...previous,x_um:-20})?.what).toBe('previous spot synthetic-1');
  });
  it('matches shape scoring for 500 overlapping spots, lines, rasters and missing sizes at several zooms',()=>{
    const ss=Array.from({length:500},(_,i)=>spot(i)),index=new SpotIndex(ss),rand=random();
    for(const k of [.01,.5,1,10,200]) for(let i=0;i<1000;i++) {
      const x=rand()*10400-300,y=rand()*450-100;
      expect(index.hit(x,y,k)?.id??null).toBe(nearestSpot(ss,x,y,k)?.id??null);
    }
    const ties=[spot(4),{...spot(4),id:'later'}];
    expect(new SpotIndex(ties).hit(ties[0].x_um,ties[0].y_um,1)?.id).toBe(ties[0].id);
  });
  it('keeps exact 8 px shape slack and 5 px missing-size marker boundaries',()=>{
    const s={...spot(0),kind:'spot' as const,x_um:128,y_um:128,x2_um:null,y2_um:null,diameter_um:10};
    for(const item of [s,{...s,diameter_um:null as unknown as number}]) {
      const index=new SpotIndex([item]);
      expect(index.hit(141,128,1)?.id).toBe(item.id);
      expect(index.hit(141+1e-8,128,1)).toBeNull();
    }
    expect(new SpotIndex([{...s,diameter_um:200}]).hit(230,128,1)?.id).toBe(s.id);
  });
  it('preserves previous-spot drag snapping across batches, modifiers and floating-point offsets',()=>{
    const gs=[grain(0),grain(1)],index=new SnapIndex(gs,20),origin={...spot(1),kind:'spot' as const,x_um:40.123456789,y_um:30.123456789};
    let brute=origin,indexed=origin;
    for(let i=0;i<120;i++) {
      const x=origin.x_um+(i%17)*1.334567891,y=origin.y_um+Math.sin(i)*9.887654321,enabled=i%11!==0;
      const a=enabled?snap(x,y,snapTargets(gs,brute,20),12):null,b=enabled?index.nearest(x,y,12,indexed):null;
      expect(b).toEqual(a);
      brute={...brute,x_um:origin.x_um+((a?.x??x)-origin.x_um),y_um:origin.y_um+((a?.y??y)-origin.y_um)};
      indexed={...indexed,x_um:origin.x_um+((b?.x??x)-origin.x_um),y_um:origin.y_um+((b?.y??y)-origin.y_um)};
      expect(indexed).toEqual(brute);
    }
  });
  it('uses inclusive bounds/source order, broad objects, empty and very large queries',()=>{
    const gs=[grain(0),grain(1)],grid=new BoundsGrid(gs,g=>g.bbox_um);
    for(const x of [0,80,100,180,90]) expect(grainAt(grid.query([x,30,x,30]),x,30)).toEqual(grainAt(gs,x,30));
    expect(new BoundsGrid([],()=>[0,0,0,0]).query([0,0,0,0])).toEqual([]);
    const huge={...grain(2),bbox_um:[-1e9,-1e9,1e9,1e9] as Grain['bbox_um']};
    expect(new BoundsGrid([huge,...gs],g=>g.bbox_um).query([0,0,1,1])).toEqual([huge,gs[0]]);
    expect(grid.query([-1e9,-1e9,1e9,1e9])).toEqual(gs);
  });
  it('rebuilds after edits, undo, area replacement, sizes and previous spot changes',()=>{
    const original=grain(0), moved={...original,centroid_um:[900,900] as [number,number]};
    expect(new SnapIndex([original],80).nearest(900,900,1,null)).toBeNull();
    expect(new SnapIndex([moved],80).nearest(900,900,1,null)?.x).toBe(900);
    expect(new SnapIndex([original],80).nearest(40,30,1,null)?.x).toBe(40);
    const ss=[spot(1)],s=ss[0];
    expect(new SpotIndex(ss).hit(s.x_um,s.y_um,1)?.id).toBe(s.id);
    expect(new SpotIndex([{...s,x_um:9000,x2_um:9100}]).hit(s.x_um,s.y_um,1)).toBeNull();
    expect(new SpotIndex([]).hit(s.x_um,s.y_um,1)).toBeNull();
    const tip=snapTargets([original],null,10)[1];
    expect(new SnapIndex([original],10).nearest(tip.x,tip.y,1,null)).toEqual(tip);
    expect(new SnapIndex([original],80).nearest(tip.x,tip.y,1,null)).toBeNull();
  });
});
describe('frame ordering',()=>{
  function setup() { let id=0;const callbacks=new Map<number,()=>void>(),seen:number[]=[];
    const queue=latestFrame<number>(n=>seen.push(n),fn=>{callbacks.set(++id,fn);return id;},id=>{callbacks.delete(id);});
    return {queue,seen,callbacks}; }
  it('coalesces 120 events to the last point per frame',()=>{
    const {queue,seen,callbacks}=setup();for(let i=0;i<120;i++) queue.push(i);
    expect(callbacks.size).toBe(1);expect(seen).toEqual([]);
    [...callbacks.values()][0]();expect(seen).toEqual([119]);
    queue.push(120);queue.flush();expect(seen).toEqual([119,120]);
  });
  it('flushes latest move before a click/release and cancels stale work on context change',()=>{
    const {queue,seen,callbacks}=setup();queue.push(1);queue.push(2);queue.flush();seen.push(3);
    expect(seen).toEqual([2,3]);expect(callbacks.size).toBe(0);
    queue.push(4);queue.clear();expect(callbacks.size).toBe(0);queue.flush();expect(seen).toEqual([2,3]);
    queue.push(5);queue.flush();expect(seen).toEqual([2,3,5]);
  });
});
describe('explicit outline vertex edits',()=>{
  it('finds corners before edge projections and handles ring closure',()=>{
    const points=grain(0).polygon_um;
    expect(outlineHandle(points,[0,1],8)).toMatchObject({kind:'vertex',index:0});
    expect(outlineHandle(points,[40,-2],8)).toEqual({kind:'edge',index:0,point:[40,0]});
    expect(outlineHandle(points,[-2,30],8)).toEqual({kind:'edge',index:3,point:[0,30]});
    expect(outlineHandle(points,[40,30],8)).toBeNull();
  });
  it('retains identity, labels, metadata and deliberate collinear corners without mutating input',()=>{
    const original={...grain(0),label:'original',sample:'synthetic',kept:true},before=structuredClone(original);
    const points=original.polygon_um.map(p=>[...p] as [number,number]);points.splice(1,0,[40,0]);
    const added=editedOutline(original,points);expect(added.polygon_um).toHaveLength(5);
    points[1]=[40,-10];const changed=editedOutline(original,points);
    expect(changed).toMatchObject({id:original.id,number:original.number,label:'original',sample:'synthetic',kept:true,drawn:true,manual:true});
    expect(changed.area_um2).toBeGreaterThan(original.area_um2);expect(original).toEqual(before);
    const removed=editedOutline(original,original.polygon_um.slice(0,3));expect(removed.polygon_um).toHaveLength(3);
  });
  it('rejects degenerate and crossed geometry',()=>{
    expect(()=>editedOutline(grain(0),[[0,0],[1,1]])).toThrow();
    expect(()=>editedOutline(grain(0),[[0,0],[10,10],[0,10],[10,0]])).toThrow();
    expect(()=>editedOutline(grain(0),[[0,0],[0,0],[10,0],[0,10]])).toThrow();
    expect(()=>editedOutline(grain(0),[[0,0],[80,0],[40,0],[80,80],[0,80]])).toThrow();
  });
});
