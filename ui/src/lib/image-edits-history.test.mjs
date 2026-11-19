import assert from 'node:assert/strict';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import { registerHooks } from 'node:module';

// Exercise the actual rune-backed store and sync class without a worker or DOM.
// Relative extension resolution mirrors Vite; only the pixel core is replaced.
const hooks = registerHooks({
  resolve(specifier, context, next) {
    try { return next(specifier, context); }
    catch (error) {
      if (specifier.startsWith('.') && (!/\.[a-z]+$/.test(specifier) || specifier.endsWith('.svelte'))) return next(specifier + '.ts', context);
      throw error;
    }
  },
  load(url, context, next) {
    if (url.endsWith('/core/core.ts')) return { format: 'module', shortCircuit: true, source: 'export const newCore = () => Promise.resolve({}); export const isDesktop = () => false; export const isHosted = () => false;' };
    const result = next(url, context);
    if (url.endsWith('.svelte.ts') || url.endsWith('/remote.ts')) {
      const source = ts.transpileModule(String(result.source), { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } }).outputText;
      return { format: 'module', shortCircuit: true, source: url.endsWith('.svelte.ts') ? compileModule(source, { filename: new URL(url).pathname, generate: 'client' }).js.code : source };
    }
    return result;
  },
});
const { current } = await import('./store.svelte.ts');
const { storeImageEdits, emptyImageEdits } = await import('./image-edits.ts');
const { keepPrintEdits } = await import('./strip-edit.ts');
const { persistProject, newerStoredCopy, answerStoredCopy } = await import('./stored-copy.ts');
const { neutralTone } = await import('./bundle-tone.ts');
const { ensureDraft, writeDraft, listDrafts, storedDraftProject } = await import('./drafts.ts');
const { readImageEdits } = await import('./image-edits.ts');
test.after(() => hooks.deregister());
const source={schema_version:1,uuid:'synthetic',name:'Synthetic',package:{name:'Synthetic',path:''},upbdb:{mount_id:null},settings:{},areas:[{id:'a',name:'A',sample:'A',params:{tone:neutralTone(),segment:{},stitch:{},photometric:{},blend:{}}}],grainatlas_ui:{grains:{a:[{id:'g1',number:1,centroid_um:[17.123456789,42],polygon_um:[[1,1],[2,1],[2,2]],bbox_um:[1,1,2,2],area_um2:1}]},spots:[{id:'s1',grain_id:'g1',name:'A_1',area_id:'a',x_um:17.123456789,y_um:42,diameter_um:15}],labels:[],tone:{}}};
test('real project history and saved-copy recovery preserve geometry through image edit, undo, redo and reset',()=>{
  const app=current();app.loadingProject=true;app.project=structuredClone(source);app.adopt(structuredClone(source));app.viewMode=false;app.clearHistory();
  const before=JSON.stringify({grains:app.grains,spots:app.spots,labels:app.labels});
  const e={...emptyImageEdits(),rotate_deg:90,crop:{kind:'rect',points_um:[[0,0],[10,10]]},levels:{black:18,white:236},masks:[{points_um:[[2,2],[4,4]],radius_um:1}]};
  storeImageEdits(app,'a',e);assert.deepEqual(app.project.image_edits.areas.a,e);
  app.undo();assert.equal(app.project.image_edits,undefined);app.redo();assert.deepEqual(app.project.image_edits.areas.a,e);
  storeImageEdits(app,'a',emptyImageEdits());assert.equal(app.project.image_edits,undefined);app.undo();assert.deepEqual(app.project.image_edits.areas.a,e);
  assert.equal(JSON.stringify({grains:app.grains,spots:app.spots,labels:app.labels}),before);
  const saved=app.fullProject();assert.deepEqual(keepPrintEdits({...source},saved).image_edits,saved.image_edits);
  const writes=new Map(),storage={getItem:k=>writes.get(k)??null,setItem:(k,v)=>writes.set(k,v)};
  assert.equal(persistProject(saved,'Synthetic.gaproj','2026-01-02T00:00:00Z',storage),true);
  const copy=newerStoredCopy(source,'Synthetic.gaproj',Date.parse('2026-01-01T00:00:00Z'),storage);assert.ok(copy);copy.ready=true;
  app.project=structuredClone(source);app.storedCopy=copy;answerStoredCopy(app,true,storage);
  assert.deepEqual(app.project.image_edits,saved.image_edits);assert.equal(JSON.stringify({grains:app.grains,spots:app.spots,labels:app.labels}),before);
  app.loadingProject=false;
});

test('loading a stored local draft restores the complete image recipe and original geometry',()=>{
  const saved=structuredClone(source);
  saved.image_edits={version:1,areas:{a:{
    ...emptyImageEdits(),rotate_deg:121.123456789,flip_h:true,flip_v:true,
    crop:{kind:'polygon',points_um:[[0,0],[100.123456789,0],[100,100],[0,100]]},
    levels:{black:18,white:236},masks:[{points_um:[[2,2],[4,4]],radius_um:1.123456789}],
  }}};
  // Only serialized storage survives a new document. Use the same draft lookup as Edit.
  let writes=new Map();
  const storage={get length(){return writes.size;},key:i=>[...writes.keys()][i]??null,
    getItem:k=>writes.get(k)??null,setItem:(k,v)=>writes.set(k,v),removeItem:k=>writes.delete(k)};
  assert.equal(persistProject(saved,null,'2026-01-02T00:00:00Z',storage),true);
  assert.equal(writeDraft(ensureDraft(saved,'https://example.invalid',storage),storage),true);
  writes=new Map(JSON.parse(JSON.stringify([...writes])));
  const drafts=listDrafts('https://example.invalid',storage);assert.equal(drafts.length,1);
  const restored=storedDraftProject(drafts[0],storage);assert.ok(restored);
  assert.deepEqual(readImageEdits(restored.image_edits),saved.image_edits);
  const app=current();app.loadingProject=true;
  try {
    app.project=restored;app.adopt(restored);
    const reopened=app.fullProject();
    assert.deepEqual(reopened.image_edits,saved.image_edits);
    assert.equal(JSON.stringify(reopened.grainatlas_ui.grains),JSON.stringify(saved.grainatlas_ui.grains));
    assert.equal(JSON.stringify(reopened.grainatlas_ui.spots),JSON.stringify(saved.grainatlas_ui.spots));
  } finally {app.loadingProject=false;}
});
