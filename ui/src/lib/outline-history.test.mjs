import assert from 'node:assert/strict';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import { registerHooks } from 'node:module';
// Real rune store/history/persistence; only the pixel worker is replaced, as in image history tests.
const hooks=registerHooks({
  resolve(specifier,context,next) { try{return next(specifier,context);}catch(error){if(specifier.startsWith('.')&&(!/\.[a-z]+$/.test(specifier)||specifier.endsWith('.svelte')))return next(specifier+'.ts',context);throw error;} },
  load(url,context,next) {
    if(url.endsWith('/core/core.ts'))return {format:'module',shortCircuit:true,source:'export const newCore=()=>Promise.resolve({});export const isDesktop=()=>false;export const isHosted=()=>false;'};
    const result=next(url,context);
    if(url.endsWith('.svelte.ts')||url.endsWith('/remote.ts')) {
      const source=ts.transpileModule(String(result.source),{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText;
      return {format:'module',shortCircuit:true,source:url.endsWith('.svelte.ts')?compileModule(source,{filename:new URL(url).pathname,generate:'client'}).js.code:source};
    }return result;
  },
});
const {current}=await import('./store.svelte.ts');
const {editedOutline}=await import('./outline-vertices.ts');
const {neutralTone}=await import('./bundle-tone.ts');
const {storedElements}=await import('./sync.ts');
test.after(()=>hooks.deregister());
const memory=new Map();
Object.defineProperty(globalThis,'localStorage',{configurable:true,value:{getItem:k=>memory.get(k)??null,setItem:(k,v)=>memory.set(k,String(v)),removeItem:k=>memory.delete(k),get length(){return memory.size;},key:i=>[...memory.keys()][i]??null}});
test('vertex geometry persists through existing history and exact undo/redo without changing registered spots',()=>{
  const grain={id:'g1',number:1,label:'keep-label',sample:'synthetic',centroid_um:[40,40],polygon_um:[[0,0],[80,0],[80,80],[0,80]],bbox_um:[0,0,80,80],area_um2:6400,major_um:80,minor_um:80,eq_diameter_um:90,orientation_deg:0,mean_intensity:1,scale_bar_um:10,manual:false};
  const source={schema_version:1,uuid:'synthetic-outline',name:'Synthetic',package:{name:'Synthetic',path:''},upbdb:{mount_id:null},settings:{},areas:[{id:'a',name:'A',sample:'synthetic',params:{tone:neutralTone(),segment:{},stitch:{},photometric:{},blend:{}}}],grainatlas_ui:{grains:{a:[grain]},spots:[{id:'s1',grain_id:'g1',grain_number:1,acquisition_id:'synthetic-acquisition',name:'keep-registered-name',area_id:'a',x_um:1,y_um:1,diameter_um:15,comment:'keep'}],labels:[],tone:{}}};
  const app=current();app.loadingProject=true;app.project=structuredClone(source);app.adopt(structuredClone(source));app.viewMode=false;app.clearHistory();
  app.link.syncAutomatically=false;app.loadingProject=false;
  const base=JSON.parse(JSON.stringify(app.sync.elements));
  app.sync.draft={uuid:source.uuid,name:source.name,mount:null,server:'https://example.invalid',base,base_revision:1,base_hashes:{spots:'s1',grains:'g1',labels:'l1'},changes:{spots:0,grains:0,labels:0},pending:null,pixels_base:null,pixels_changed:false,updated_at:'2026-01-01T00:00:00Z'};
  const before=JSON.stringify({grains:app.grains,spots:app.spots,labels:app.labels});
  const spots=JSON.stringify(app.spots);
  // Put the registered spot outside the new boundary: an explicit vertex edit still cannot
  // silently relink, rename, resize or move the existing acquisition.
  const edited=editedOutline(grain,[[20,20],[80,0],[80,80],[0,80]]);
  app.checkpoint();app.grains={...app.grains,a:[edited]};app.persist();app.flushPersistence();
  assert.deepEqual(app.sync.draft.changes,{spots:0,grains:1,labels:0});
  assert.equal(app.sync.draft.pending.grains[0].id,grain.id);
  assert.deepEqual(app.sync.draft.pending.spots,storedElements(base).spots);
  assert.equal(JSON.stringify(app.spots),spots);assert.equal(app.canUndo,true);
  const saved=app.fullProject();assert.deepEqual(saved.grainatlas_ui.grains.a[0],edited);
  assert.equal(JSON.stringify(saved.grainatlas_ui.spots),spots);
  app.undo();app.flushPersistence();assert.deepEqual(app.sync.draft.changes,{spots:0,grains:0,labels:0});assert.equal(JSON.stringify({grains:app.grains,spots:app.spots,labels:app.labels}),before);
  app.redo();app.flushPersistence();assert.deepEqual(app.sync.draft.changes,{spots:0,grains:1,labels:0});assert.deepEqual(app.fullProject().grainatlas_ui.grains.a[0],edited);assert.equal(JSON.stringify(app.spots),spots);
  app.undo();app.flushPersistence();app.loadingProject=false;
});
