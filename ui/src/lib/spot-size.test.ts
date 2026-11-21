import { describe, it, expect } from 'vitest';
import { spotDiameter, nextSpotDiameter, selectionSize, areaCalibration, planSizeRepairs, applySizeRepairs } from './spot-size';
import { mountSpots } from './upbdb';
import { orderSpots } from './batch';
import { SizeEditor } from './size-editor';
import { spotDistance } from './spots';
import { viewElements, emptyElements } from './sync';
import type { Spot } from './core/types';

const spot = (id: string, area_id: string, diameter_um: unknown, created = '') => ({ id, area_id, diameter_um, created, name: `Synthetic ${id}`, grain_id: 'synthetic-grain', acquisition_id: 'synthetic-link', x_um: 10, y_um: 10, kind: 'spot' }) as Spot;
describe('spot sizes without implicit data edits', () => {
  it('accepts only finite positive numeric sizes, without coercion or clamping', () => {
    for (const d of [null, undefined, 0, -1, NaN, Infinity, '20', '', false]) expect(spotDiameter({ diameter_um: d })).toBeNull();
    for (const d of [0.2, 20, 501]) expect(spotDiameter({ diameter_um: d })).toBe(d);
  });
  it('A: selection value, mixed range, invalid sizes and next-spot value are distinct', () => {
    expect(selectionSize([spot('one','a',20)],25)).toMatchObject({ value:20, caption:'1 selected' });
    expect(selectionSize([spot('one','a',20),spot('two','a',40)],25)).toMatchObject({ value:null, placeholder:'mixed', range:'⌀ 20–40 µm' });
    expect(selectionSize([spot('one','a',null)],25)).toMatchObject({ value:null, placeholder:'not set' });
    expect(selectionSize([spot('one','a',null),spot('two','a',20)],25)).toMatchObject({ value:null, placeholder:'mixed', range:'⌀ 20 µm; not set' });
    expect(selectionSize([],25)).toMatchObject({ value:25, caption:'next spot' });
  });
  it('A: area mode wins, then project mode, then Settings; invalid sizes never vote', () => {
    const rows = [spot('a1','a',20),spot('a2','a',20),spot('a3','a',40,'2026-01-03'),
      ...Array.from({length:4},(_,i)=>spot(`b${i}`,'b',30)),
      ...[null,undefined,0,-1,NaN,Infinity,'20'].map((d,i)=>spot(`bad${i}`,'c',d,'2026-01-05'))];
    expect(nextSpotDiameter(rows,'a',25)).toBe(20); expect(nextSpotDiameter(rows,'b',25)).toBe(30);
    expect(nextSpotDiameter(rows,'c',25)).toBe(30); expect(nextSpotDiameter(rows,null,25)).toBe(30);
    expect(nextSpotDiameter(rows.filter(s=>s.area_id==='c'),'c',35)).toBe(35);
    expect(nextSpotDiameter([],'a',35)).toBe(35);
  });
  it('A: 59 imported 40 µm spots outweigh one dated local 25 µm spot', () => {
    const imported = Array.from({length:59},(_,i)=> {
      const s = spot(`synthetic-import-${i}`,'a',40);
      const { created, ...undated } = s;
      return i % 2 ? undated : s;
    });
    const rows = [...imported, spot('synthetic-local','a',25,'2026-11-10')];
    expect(nextSpotDiameter(rows,'a',35)).toBe(40);
    expect(nextSpotDiameter(rows,'empty-area',35)).toBe(40);
  });
  it('A: tied modes use their newest spot, treating missing dates as oldest, then later position', () => {
    const { created, ...undated40 } = spot('undated40','a',40);
    const rows = [spot('new','a',40,'2026-01-03'),spot('old','a',20,'2026-01-01'),
      undated40,spot('undated20','a',20)];
    expect(nextSpotDiameter(rows,'a',25)).toBe(40);
    expect(nextSpotDiameter(rows,'empty-area',25)).toBe(40);
    const sameDate = [...rows.slice(0,3),spot('last','a',20,'2026-01-03')];
    expect(nextSpotDiameter(sameDate,'a',25)).toBe(20);
    expect(nextSpotDiameter(sameDate,'empty-area',25)).toBe(20);
    const undated = [undated40,spot('last','a',20)];
    expect(nextSpotDiameter(undated,'a',25)).toBe(20);
    expect(nextSpotDiameter([...undated].reverse(),'a',25)).toBe(40);
  });
  it('C: calibration is absent until known; uses only the supplied current area and mosaic', () => {
    expect(areaCalibration(null,null)).toBeNull(); expect(areaCalibration({pixel_size_nm:200},null)).toBe(.2);
    expect(areaCalibration({pixel_size_nm:200},{um_per_px:.4})).toBe(.4);
    expect(areaCalibration({pixel_size_nm:0},{um_per_px:NaN})).toBeNull();
  });
  it('invalid-size hit tests use the marker centre, without an assumed µm radius', () => {
    expect(spotDistance(spot('bad','a',null),13,14)).toBe(5);
    expect(spotDistance(spot('good','a',20),13,14)).toBe(0);
    expect(spotDistance(spot('bad','a',null),13,14,5)).toBe(0);
  });
  it('repair preview is immutable, area mode then project mode then Settings; only invalid sizes change', () => {
    const rows = [spot('a1','a',20),spot('a2','a',20),spot('a3','a',40),spot('bad-a','a',null),spot('b1','b',40),spot('bad-b','b',0),spot('bad-c','c','20')];
    const before = JSON.stringify(rows), plan = planSizeRepairs(rows,25);
    expect(plan.map(r=>r.proposed)).toEqual([20,40,20]); expect(JSON.stringify(rows)).toBe(before);
    const applied = applySizeRepairs(rows,plan);
    expect(applied[0]).toBe(rows[0]); expect(applied[1]).toBe(rows[1]); expect(applied[2]).toBe(rows[2]);
    for (const s of applied) expect(s).toMatchObject({name:`Synthetic ${s.id}`,grain_id:'synthetic-grain',acquisition_id:'synthetic-link'});
    expect(planSizeRepairs([spot('bad','a',null)],35)[0].proposed).toBe(35);
    expect(applySizeRepairs([spot('bad-a','a',30)],plan)[0].diameter_um).toBe(30);
    expect(applySizeRepairs([spot('bad-a','a',0)],plan)[0].diameter_um).toBe(0); // changed after preview
  });
  it('exports preserve valid diameters exactly and reject unset sizes without changing data', () => {
    const good = spot('one','a',20.125), bad = spot('bad','a',null);
    expect(mountSpots('synthetic','Synthetic',[good]).body.spots[0].diameter_um).toBe(20.125);
    expect(() => mountSpots('synthetic','Synthetic',[good,bad])).toThrow(/no size/);
    expect(bad.diameter_um).toBeNull(); expect(good.diameter_um).toBe(20.125);
  });
  it('batch row tolerance ignores invalid sizes and stays deterministic when none are set', () => {
    const rows = [spot('one','a',20),spot('two','a',null),spot('three','a',40)];
    rows[0].y_um=200; rows[1].y_um=100; rows[2].y_um=0;
    expect(orderSpots(rows,'rows',[],[]).map(s=>s.id)).toEqual(['three','two','one']);
    expect(orderSpots(rows.map(s=>({...s,diameter_um:NaN})),'rows',[],[]).map(s=>s.id)).toEqual(['three','two','one']);
  });
  it('large selections and next-size lookup do not modify stored rows', () => {
    const rows=Array.from({length:5000},(_,i)=>spot(`synthetic-${i}`,i%2?'a':'b',i%2?20:40,String(i).padStart(5,'0')));
    const before=JSON.stringify(rows);
    expect(selectionSize(rows,25)).toMatchObject({value:null,range:'⌀ 20–40 µm'});
    expect(nextSpotDiameter(rows,'a',25)).toBe(20); expect(nextSpotDiameter(rows,'b',25)).toBe(40);
    expect(JSON.stringify(rows)).toBe(before);
  });
  it('D: null server geometry preserves bundle size intentionally; valid server sizes supersede it', () => {
    const bundle = {...emptyElements(),spots:[spot('one','a',20)]};
    const server = {...emptyElements(),spots:[spot('one','a',null)]};
    expect(viewElements(bundle,server).spots[0].diameter_um).toBe(20);
    server.spots[0].diameter_um=40;
    expect(viewElements(bundle,server).spots[0].diameter_um).toBe(40);
    expect(bundle.spots[0].diameter_um).toBe(20);
  });
});
describe('B: focus transactions', () => {
  it('drops stale text after same-size target change, even if it changes back before blur', () => {
    const e = new SizeEditor(), writes:number[]=[];
    e.sync(20,'a'); e.focus('a',d=>writes.push(d));
    expect(e.sync(20,'b')).toBe('20'); e.sync(20,'a');
    expect(e.commit('30',20,'a')).toBeNull(); expect(writes).toEqual([]);
  });
  it('external value changes reset typing and invalidate pending input; explicit refocus permits editing', () => {
    const e = new SizeEditor(), writes:number[]=[];
    e.sync(20,'a'); e.focus('a',d=>writes.push(d)); expect(e.sync(40,'a')).toBe('40');
    expect(e.commit('30',40,'a')).toBeNull(); e.blur(); e.focus('a',d=>writes.push(d));
    expect(e.commit('30',40,'a')).toBe(30); expect(e.sync(30,'a')).toBeNull(); expect(writes).toEqual([30]);
  });
  it('guards changes before reactive effects run and accepts mixed/null entry only explicitly', () => {
    const e = new SizeEditor(); e.sync(20,'a'); e.focus('a',()=>{throw Error('stale');});
    expect(e.commit('30',20,'b')).toBeNull(); e.blur(); e.sync(null,'b'); e.focus('b',()=>{});
    expect(e.commit('',null,'b')).toBeNull(); expect(e.commit('25',null,'b')).toBe(25);
  });
});
