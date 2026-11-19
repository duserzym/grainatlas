import assert from 'node:assert/strict';
import test from 'node:test';
import { draftPrintRecipe, keepDraftPrintRecipe, readDraft, writeDraft } from './drafts.ts';
import { persistProject } from './stored-copy.ts';
import { keepPrintEdits } from './strip-edit.ts';
import { draftSyncStatus } from './sync-status.ts';
import { SyncApi } from './sync-api.ts';
import { emptyElements } from './sync.ts';
import { proxy } from 'svelte/internal/client';
import { DeferredWrites } from './deferred-writes.ts';

const key={area:'a',grains_hash:'synthetic',split:0};
const recipes=[
  {params:{packing:'packed'},um_per_mm:8,edits:[{op:'repack'}]},
  {version:1,params:{packing:'packed'},um_per_mm:8,edits:[{op:'repack'}]},
  {version:2,params:{packing:'packed'},um_per_mm:8,edits:[{op:'arrange'},{op:'transform',pieces:[{strip:key,sheet:0,origin_mm:[20,80],scale:0.6,rotation_deg:17.5}]}]},
  {version:99,edits:[{op:'future',keep:true}]},null,
  {params:{packing:'packed'},um_per_mm:4,edits:[{op:'future_edit',value:['zr-1',42]}]},
];
const project=recipe=>({uuid:'synthetic',package:{name:'mount A'},areas:[],print_sheets:structuredClone(recipe),grainatlas_ui:{grains:{},spots:[],labels:[]}});
const draft=()=>({uuid:'synthetic',name:'mount A',server:'https://recipe.test',mount:'mount',base_revision:1,base:emptyElements(),changes:{spots:0,grains:0,labels:0},pending:null,pixels_base:null,source_keys:[]});
function storage(){const values=new Map();return {getItem:k=>values.get(k)??null,setItem:(k,v)=>values.set(k,v),removeItem:k=>values.delete(k),key:i=>[...values.keys()][i]??null,get length(){return values.size;}};}

test('reactive recipes flush before Save without blocking or changing unreadable JSON',()=>{
  for(const recipe of recipes){
    const s=storage(),p=proxy(project(recipe)),d=proxy(draft());
    if(recipe!==null)assert.throws(()=>structuredClone(p.print_sheets),{name:'DataCloneError'},'exercise a real Svelte proxy');
    const writes=new DeferredWrites(()=>()=>{});
    let tracked;
    writes.enqueue(p,()=>{
      persistProject(p,null,'2026-01-01T00:00:00Z',s);
      tracked=proxy({...d,...draftPrintRecipe(p,d)});
      writes.enqueue(d,()=>writeDraft(tracked,s));
    });
    // Save flushes pending project and draft writes before producing the file.
    writes.flush();
    const saved=JSON.parse(JSON.stringify(p));
    assert.deepEqual(saved.print_sheets,recipe);
    const persisted=readDraft(p.uuid,s);
    assert.equal(persisted.pending,null);
    assert.deepEqual(persisted.print_recipe,{print_sheets:recipe});
    const incoming=proxy({uuid:p.uuid});
    keepDraftPrintRecipe(incoming,tracked,s);
    assert.deepEqual(JSON.parse(JSON.stringify(incoming.print_sheets)),recipe);
    if(recipe!==null){
      p.print_sheets.edits.push({op:'later'});
      assert.deepEqual(JSON.parse(JSON.stringify(tracked.print_recipe.print_sheets)),recipe,'tracking is an isolated snapshot');
      tracked.print_recipe.print_sheets.edits.push({op:'other'});
      assert.deepEqual(JSON.parse(JSON.stringify(incoming.print_sheets)),recipe,'recovery is an isolated snapshot');
    }
    delete p.print_sheets;
    assert.deepEqual(draftPrintRecipe(p,tracked),{print_recipe:{}},'reset also works with a reactive draft');
  }
});

test('v1, v2 and unknown recipes survive a clean outbox, restart and a missing or stale server copy',()=>{
  for(const recipe of recipes){
    const s=storage(),p=project(recipe),d={...draft(),...draftPrintRecipe(p,draft())};
    assert.equal(writeDraft(d,s),true);
    const restored=readDraft(p.uuid,s);
    assert.equal(draftSyncStatus(restored,s).pending,false,'recipe tracking creates no element or pixel upload');
    for(const incoming of [{uuid:p.uuid},{uuid:p.uuid,print_sheets:{version:1,edits:[]}}]){
      const rows={grains:{a:[{id:'new-server-grain'}]},spots:[{id:'new-server-spot'}],labels:[]};
      const reopened=keepDraftPrintRecipe({...incoming,grainatlas_ui:rows},restored,s);
      assert.deepEqual(reopened.print_sheets,recipe);
      assert.equal(reopened.grainatlas_ui,rows,'only the recipe is overlaid, never stale registered data');
      assert.deepEqual(keepPrintEdits({uuid:p.uuid},reopened).print_sheets,recipe,'core record refresh keeps it too');
    }
    assert.deepEqual(keepDraftPrintRecipe({uuid:'another-project'},restored,s),{uuid:'another-project'});
  }
});

test('an explicit reset is retained as an empty envelope and raw null remains a value',()=>{
  const d={...draft(),...draftPrintRecipe(project(recipes[2]),draft())};
  const reset={...d,...draftPrintRecipe({uuid:d.uuid},d)};
  assert.deepEqual(reset.print_recipe,{});
  const s=storage();writeDraft(reset,s);
  assert.equal(Object.hasOwn(keepDraftPrintRecipe(project(recipes[2]),readDraft(d.uuid,s),s),'print_sheets'),false);
  assert.deepEqual(draftPrintRecipe({uuid:d.uuid},draft()),{},'a legacy missing field is not an explicit reset');
  const raw={...draft(),...draftPrintRecipe(project(null),draft())};
  assert.deepEqual(keepDraftPrintRecipe({uuid:d.uuid},raw,s).print_sheets,null);
});

test('older clean drafts recover just the recipe from their full local project record',()=>{
  for(const path of [null,'mount A.gaproj']){
    const s=storage(),p=project(recipes[2]);persistProject(p,path,'2026-01-01T00:00:00Z',s);
    const source=path?`grainatlas.copy.${p.uuid}|${path}`:'grainatlas.project.mount A';
    const d={...draft(),source_keys:[source]};writeDraft(d,s);
    const incoming={uuid:p.uuid,name:'current server name',grainatlas_ui:{grains:{},spots:[],labels:[]}};
    assert.deepEqual(keepDraftPrintRecipe(incoming,readDraft(p.uuid,s),s),{...incoming,print_sheets:p.print_sheets});
    incoming.print_sheets.edits=[];
    assert.equal(p.print_sheets.edits.length,2,'reopening must not alias stored recipe bytes');
  }
});

test('element sync and canonical refetch do not acknowledge or discard a local recipe; 4,000 rows use the same requests',async t=>{
  const elements={...emptyElements(),grains:Array.from({length:4000},(_,i)=>({id:`grain-${i}`,number:i+1,area_id:'a'})),revision:2};
  const calls=[];
  t.mock.method(globalThis,'fetch',async(url,init)=>{
    calls.push({url:String(url),method:init.method??'GET',body:init.body&&JSON.parse(init.body)});
    return Response.json(init.method==='POST'?{revision:2}:elements);
  });
  const api=new SyncApi('https://recipe-roundtrip.test',null);
  const changes={spots:{upsert:[],delete:[]},grains:{upsert:[],delete:[],retire:[]},labels:{upsert:[],delete:[]}};
  const run=async local=>{
    const start=performance.now(),at=calls.length;
    const reply=await api.sync('mount',{project_uuid:'synthetic',base_revision:1,mode:'delta',...changes});assert.equal(reply.ok,true);
    const accepted=await api.elements('synthetic');
    const current={uuid:'synthetic',grainatlas_ui:{grains:{a:accepted.grains},spots:accepted.spots,labels:accepted.labels}};
    const reopened=local?keepDraftPrintRecipe(current,local):current;
    return {reopened,requests:calls.length-at,ms:performance.now()-start};
  };
  const baseline=await run(null);
  const d={...draft(),...draftPrintRecipe(project(recipes[2]),draft())};
  const retained=await run(JSON.parse(JSON.stringify(d)));
  assert.equal(baseline.requests,2);assert.equal(retained.requests,2);
  assert.equal(retained.reopened.grainatlas_ui.grains.a.length,4000);
  assert.deepEqual(retained.reopened.print_sheets,recipes[2]);
  assert.ok(calls.filter(c=>c.method==='POST').every(c=>!Object.hasOwn(c.body,'print_sheets')&&!Object.hasOwn(c.body,'print_recipe')));
  assert.deepEqual(d.print_recipe,{print_sheets:recipes[2]},'an element acknowledgement cannot clear the recipe');
  t.diagnostic(`4,000 rows: before ${baseline.requests} requests / ${baseline.ms.toFixed(1)} ms; after ${retained.requests} requests / ${retained.ms.toFixed(1)} ms; 0 extra requests`);
});
