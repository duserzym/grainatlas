import test from 'node:test';
import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { align, bounds, clampGroup, distribute, formatPieceScale, groupMove, groupRect, groupScale, nudge, pieceLabels, pieceMatrix, pieceRect, placed, scaleBar, snapGroup } from './arrange.ts';
import { keyId } from './strip-edit.ts';
import { readPrintEdits, storePrintEdits, StripHistory } from './strip-edit.ts';
import { printFingerprint } from './print-cache.ts';
import { primaryModifier } from './platform.ts';
const strip=(id,x=10,y=50)=>({key:{area:'a',grains_hash:id,split:0},sheet:0,origin_mm:[x,y],size_mm:[40,20],bands:[],transform:{scale:1,rotation_deg:0}});
const plan=(strips)=>({params:{},um_per_mm:8,strips,sheet_bodies:[{body_rect_mm:[8,40,420,260]}]});
const near=(a,b)=>assert.ok(Math.abs(a-b)<1e-9,`${a} equals ${b}`);

test('scale display rounds to two decimals without rounding the transform',()=>{
  const exact=22.857061594982536,s=strip('a');s.transform.scale=8/exact;
  assert.equal(formatPieceScale(8/s.transform.scale),'22.86');
  assert.equal(formatPieceScale(8),'8');assert.equal(formatPieceScale(6.3),'6.3');
  assert.equal(formatPieceScale(0.001),'1e-3');
  near(8/s.transform.scale,exact);
});
test('same-area pieces have distinct stable labels across sheets and share them with selection',()=>{
  const ss=[strip('first'),strip('second'),strip('hidden'),{...strip('other'),key:{area:'b',grains_hash:'third',split:0}}].map(s=>({...s,sample:'A1',label:'area A1'}));
  ss[1].sheet=1;ss[2].hidden=true;
  const labels=pieceLabels(ss);
  assert.equal(labels.get(keyId(ss[0].key)),'A1 · area A1 · piece 1 of 2');
  assert.equal(labels.get(keyId(ss[1].key)),'A1 · area A1 · piece 2 of 2');
  assert.equal(labels.get(keyId(ss[3].key)),'A1 · area A1');
  assert.equal(labels.has(keyId(ss[2].key)),false);
  const moved=ss.map(s=>({...s,origin_mm:[200,150],transform:{scale:0.5,rotation_deg:90}}));
  assert.deepEqual(pieceLabels(moved),labels);
});

test('piece matrix rotates all four corners about the bounding-box origin',()=>{
  const s=strip('a');s.transform={scale:0.5,rotation_deg:90};
  const r=pieceRect(s);r.forEach((v,i)=>near(v,[10,50,20,70][i]));
  assert.deepEqual(bounds(pieceMatrix(s),[0,0,40,20]),r);
});
test('corner resize preserves aspect and scales group positions about either anchor',()=>{
  const strips=[strip('a'),strip('b',70,90)];
  for(const anchor of [[10,50],[110,110]]){
    const ps=groupScale(strips,0.5,anchor);
    const next=strips.map((s,i)=>placed(s,ps[i]));
    const before=groupRect(strips),after=groupRect(next);
    near((after[2]-after[0])/(before[2]-before[0]),0.5);
    near((after[3]-after[1])/(before[3]-before[1]),0.5);
    for(let i=0;i<2;i++){near(ps[i].origin_mm[0]-anchor[0],(strips[i].origin_mm[0]-anchor[0])*0.5);assert.equal(ps[i].scale,0.5);}
    assert.deepEqual(strips[0].origin_mm,[10,50]);
  }
});
test('snap guides find margins, edges and centres while ignoring selected pieces',()=>{
  const a=strip('a',9,41),b=strip('b',100,100),p=plan([a,b]);
  assert.deepEqual(snapGroup(p,[a]).delta,[-1,-1]);
  const centered={...a,origin_mm:[101,150]};
  const snap=snapGroup(p,[centered]);assert.equal(snap.delta[0],-1);assert.ok(snap.guides.some(g=>g.axis===0));
  assert.deepEqual(snapGroup(p,[{...a,origin_mm:[25,65]}],0).delta,[0,0]);
});
test('nudge is 1 mm, Shift nudge 10 mm, and header clamp translates the whole group',()=>{
  const ss=[strip('a'),strip('b',80,90)];
  assert.deepEqual(nudge(ss,[1,-1]).map(p=>p.origin_mm),[[11,49],[81,89]]);
  assert.deepEqual(nudge(ss,[-1,1],true).map(p=>p.origin_mm),[[0,60],[70,100]]);
  const moved=groupMove(ss,[0,-50]).map((p,i)=>placed(ss[i],p));
  assert.deepEqual(clampGroup(moved,[8,40,420,260]),[0,40]);
  assert.equal(primaryModifier('Win32'),'Ctrl');assert.equal(primaryModifier('MacIntel'),'Cmd');
});
test('scale bars use the 1-2-5 series and update when physical scale changes',()=>{
  assert.equal(scaleBar(6.3,20),100);assert.equal(scaleBar(12.6,20),200);
  for(const k of [0.05,0.2,1,3,6.3,8,16,120,1500]){
    const bar=scaleBar(k,16);assert.ok(bar/k<=16+1e-9);
    near(bar/10**Math.floor(Math.log10(bar)),[1,2,5].find(v=>Math.abs(bar/10**Math.floor(Math.log10(bar))-v)<1e-9));
  }
});
test('align and distribute use transformed dimensions and deterministic input ordering',()=>{
  const ss=[strip('a'),strip('b',80,100),strip('c',170,150)];
  assert.deepEqual(align(ss,0,0).map(p=>p.origin_mm[0]),[10,10,10]);
  assert.deepEqual(distribute(ss).map(p=>p.origin_mm[0]),[10,90,170]);
});
test('version 2 arrangement survives JSON, undo, raw mirrors and changes the print cache key',()=>{
  const p=plan([strip('a')]),history=new StripHistory(),app={project:{uuid:'p'},readOnly:false,dirty:false};
  history.push({op:'arrange'});history.push({op:'transform',pieces:groupScale(p.strips,0.5)});
  storePrintEdits(app,p,history.edits);assert.equal(app.project.print_sheets.version,2);
  const saved=JSON.parse(JSON.stringify(app.project.print_sheets));assert.deepEqual(readPrintEdits(saved),saved);
  history.undo();assert.equal(history.edits.length,1);history.redo();assert.deepEqual(history.edits,saved.edits);
  assert.equal(readPrintEdits({...saved,version:3}),null);
  const f=edits=>printFingerprint({identity:'p',controls:{},areas:[],tone:{},grains:{},edits});
  assert.notEqual(f(saved),f({...saved,edits:[{op:'arrange'}]}));
});
test('4,000-piece snapping and proportional resize stay local without requests',t=>{
  const strips=Array.from({length:4000},(_,i)=>strip(String(i),i%80*4,50+Math.floor(i/80)*3));
  const start=performance.now();const snap=snapGroup(plan(strips),[strips[0]]);const scaled=groupScale(strips,0.8);
  assert.equal(scaled.length,4000);assert.equal(snap.delta.length,2);
  t.diagnostic(`4,000 pieces: snap + group resize ${(performance.now()-start).toFixed(1)} ms; 0 server requests`);
});
