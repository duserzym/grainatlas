import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import '../../../scripts/perf/store-loader.mjs';
import { syncFixture } from '../../../scripts/perf/sync-fixture.ts';
const { current } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const copy=value=>JSON.parse(JSON.stringify(value));
const canvas=readFileSync(new URL('../screens/Canvas.svelte',import.meta.url),'utf8');
const section=(start,end)=>canvas.slice(canvas.indexOf(start),canvas.indexOf(end,canvas.indexOf(start)));
const source=`
import { emptyAddPreview, previewTransition, nextGrainName, namedNewGrain, detectGrainPreview, commitGrainPreview } from '${new URL('./add-grain.ts',import.meta.url).href}';
import { editedOutline, outlineHandle } from '${new URL('./outline-vertices.ts',import.meta.url).href}';
import { polygonGrain } from '${new URL('./polygon-edit.ts',import.meta.url).href}';
import { latestFrame } from '${new URL('./canvas-pointer.ts',import.meta.url).href}';
import { CLICK_SLOP } from '${new URL('./canvas/gestures.ts',import.meta.url).href}';
import { formatName, newId, grainAt } from '${new URL('./spots.ts',import.meta.url).href}';
import { spotSample } from '${new URL('./grain-owners.ts',import.meta.url).href}';
import { tick } from '${import.meta.resolve('svelte')}';
export function create(app) {
  const ro=false, areaId='a', umPerPx=1, k=1, isViewer=false;
  const current=()=>app, sample='synthetic', patternFor=()=>'{sample}_{n:03}', segParams={}, owners=new Map(), modName='Ctrl';
  const nextRun=$derived(app.nextN??501); let snapPoint=null;
  const snapped=(x,y)=>snapPoint??[x,y], setSpotSelection=s=>{app.selection=s.ids;};
  const editImage=false, canPluck=false; let imageTool=null;
  let pluck=null, spotMenu=null, batch=null, sizeRepairs=null, editing=null;
  const detections=$derived(app.grains.a), grains=$derived(detections), editableGrain=null;
  const addName=$derived(nextGrainName(detections,sample,patternFor()));
  let addPreview=$state.raw(emptyAddPreview()), addSensitivity=50, addNeedsPixels=$state(false),
      addMode=$state('click'), sensitivityTimer=null, thenSpot=false, spotAfterGrain=null, addBox=null, selGrains=[], cursorScreen=null;
  const noPixels=$derived(addNeedsPixels);
  ${section('const effectiveAddMode =', 'const addName =')}
  let collapseSpot=null, pressScreen=null, dragCrossedSlop=false, livePrevious=null,
      pendingMoveSnap=null, lastHover=null, snapHint=null, gesture=null, previousSpot=null,
      splitStroke=null, lasso=null, lassoHover=null, stroke=null, imageDrag=null, imageDraft=null,
      rectangle=null, drawing=null, queued=null, snapEdges=false;
  const imagePress=()=>false, hoverFrame={flush(){},clear(){}};
  const dragFrame=latestFrame(dragNow,fn=>{queued=fn;return 1;},()=>{queued=null;});
  let resolver=null;
  const core={
    grainAt:async()=>polygonGrain([[0,0],[80,0],[80,80],[0,80]]),
    snapOutline:async(_a,p)=>p,
    outlineGrain:async(_a,p)=>polygonGrain(p),
    refineGrain:async(_a,g)=>g,
  };
  ${canvas.match(/let vertexDrag = [^\n]+/)[0]}
  ${section('function activateTool(', 'function editOutline(')}
  ${section('function cancelAddPreview()', '$effect(() => {\n    // Preview work')}
  ${section('async function makeAddPreview(', 'const automaticN =').replace('onDestroy(cancelAddPreview);','')}
  ${section('async function focusEditingCanvas()', 'function imageMode(')}
  ${section('function saveVertices(', 'let lastHover:')}
  ${section('function press(', 'function drag(p:')}
  ${section('function drag(p:', 'async function release(')}
  ${section('async function release(', 'function cancelGesture(')}
  ${section('function cancelGesture(', 'async function doSplit(')}
  ${section('async function finishLasso()', 'async function finishStroke(')}
  ${section('function inside(poly:', '/** Double-click a spot')}
  ${section('function now()', 'function updateSpot(')}
  ${section('async function click(', '// ---- hand-edited outlines')}
  ${section('function splitModifier(', 'function selectionDiameterChange(')}
  return { press, drag, release, click, key, set thenSpot(v){thenSpot=v;}, set snapPoint(v){snapPoint=v;}, get spotTarget(){return spotAfterGrain;}, accept:acceptGrainPreview, cancel:cancelAddPreview, frame:()=>queued?.(),
    detect:makeAddPreview, sensitivity:sensitivityChange,
    set mode(v){changeAddMode(v);}, get mode(){return effectiveAddMode;}, get options(){return addModeOptions;},
    set pixels(v){addNeedsPixels=!v;}, activate:activateTool, get preview(){return addPreview;}, get box(){return addBox;},
    get points(){return vertexDrag?.points;}, finish:finishLasso,
    defer(){core.grainAt=()=>new Promise(r=>{resolver=r;});}, resolve(){resolver(polygonGrain([[0,0],[80,0],[80,80],[0,80]]));},
  };
}`;
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText;
const compiled=compileModule(js,{filename:'add-grain-gesture.svelte.ts',generate:'client'}).js.code
  .replaceAll("'svelte/internal/client'",JSON.stringify(import.meta.resolve('svelte/internal/client')));
const {create}=await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const point=([x,y],ev={})=>({x,y,sx:x,sy:y,ev});

test('Canvas Add grain modes preview, adjust, append one outbox grain and undo exactly without changing registered data',async()=>{
  const app=current(), project=syncFixture(), memory=new Map();
  appStorage.getItem=k=>memory.get(k)??null; appStorage.setItem=(k,v)=>memory.set(k,v);
  app.loadingProject=true; app.project=project; app.adopt(project); app.viewMode=false;
  app.link.syncAutomatically=false; app.areaId='a'; app.tool='grain-add'; app.loadingProject=false; app.clearHistory();
  const before=copy(app.fullProject().grainatlas_ui), base=copy(app.sync.elements);
  app.sync.draft={uuid:project.uuid,name:project.name,mount:project.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,synced:true,pending:null,pixels_base:null,pixels_changed:false,updated_at:'fixed',changes:{spots:0,grains:0,labels:0}};
  for (const mode of ['click','draw','freehand','box']) {
    const h=create(app);
    // An old selection must not become a movement/deletion target in the creation tool.
    app.selection=[before.spots[0].id];
    for (const key of ['ArrowRight','Delete']) h.key({key,target:{tagName:'DIV'},preventDefault(){}});
    assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
    app.selection=[];
    if (mode==='click') {
      await h.detect({kind:'click',point:[40,40]});
      h.sensitivity(80); assert.equal(h.preview.status,'working'); await new Promise(r=>setTimeout(r,150));
    } else if (mode==='box') {
      h.mode='box';h.press(point([0,0]));h.drag(point([80,80]));await h.release(point([80,80]));
    } else {
      h.mode='draw';
      if (mode==='draw') {
        for(const p of [[0,0],[80,0],[80,80],[0,80]]){h.press(point(p));await h.release(point(p));}
        await h.finish();
      } else {
        h.press(point([0,0]));for(const p of [[40,0],[80,0],[80,40],[80,80],[40,80],[0,80]])h.drag(point(p));
        await h.release(point([0,80]));
      }
    }
    assert.equal(h.preview.status,'ready',mode);
    assert.deepEqual(copy(app.fullProject().grainatlas_ui),before,`${mode} preview changed saved data`);
    assert.equal(app.canUndo,false);
    const p=h.preview.grain.polygon_um[0], target=[p[0]-5,p[1]-5];
    h.press(point(p));h.drag(point(target));await h.release(point(target));
    assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);
    assert.deepEqual(h.preview.grain.polygon_um[0],target);
    h.accept();app.flushPersistence();
    assert.equal(app.grains.a.length,before.grains.a.length+1);
    const added=app.grains.a.at(-1);assert.equal(added.label,`synthetic_${String(Math.max(...before.grains.a.map(g=>g.number))+1).padStart(3,'0')}`);
    assert.deepEqual(copy(app.grains.a.slice(0,-1)),before.grains.a); assert.deepEqual(copy(app.spots),before.spots);
    assert.deepEqual(app.sync.draft.changes,{spots:0,grains:1,labels:0});
    assert.equal(app.sync.draft.pending.grains.at(-1).id,added.id);
    app.undo();app.flushPersistence(); assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
    app.redo();app.flushPersistence();assert.equal(app.grains.a.at(-1).id,added.id);app.undo();app.flushPersistence();
    const cancel=create(app);await cancel.detect({kind:'click',point:[40,40]});cancel.cancel();cancel.accept();
    assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
  }
  const flow=create(app);flow.thenSpot=true;await flow.detect({kind:'click',point:[40,40]});flow.accept();app.flushPersistence();
  const added=app.grains.a.at(-1), withGrain=copy(app.fullProject().grainatlas_ui);
  assert.equal(app.tool,'spot');assert.equal(flow.spotTarget.id,added.id);
  // Even a click directly on an existing registered spot is a new placement, never a move.
  const xy=before.spots[0];assert.equal(flow.press(point([xy.x_um,xy.y_um])),false);
  await flow.click(point([500,500]));assert.equal(app.spots.length,500);assert.equal(app.tool,'spot');
  flow.snapPoint=[1000,1000];await flow.click(point([xy.x_um,xy.y_um]));app.flushPersistence();
  assert.equal(app.tool,'grain-add');assert.equal(flow.spotTarget,null);
  assert.equal(app.spots.length,501);assert.deepEqual(copy(app.spots.slice(0,-1)),before.spots);
  assert.equal(app.spots.at(-1).grain_id,added.id);assert.equal(app.spots.at(-1).grain_number,added.number);
  assert.equal(app.spots.at(-1).x_um,xy.x_um);assert.equal(app.spots.at(-1).y_um,xy.y_um);
  assert.deepEqual(copy(app.grains.a.slice(0,-1)),before.grains.a);
  assert.deepEqual(app.sync.draft.changes,{spots:1,grains:1,labels:0});
  app.undo();app.flushPersistence();assert.deepEqual(copy(app.fullProject().grainatlas_ui),withGrain);
  app.undo();app.flushPersistence();assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
  // Escape can invalidate an in-flight pixel job before it returns.
  // Invalid adjustment retains the last valid preview and adds a concrete retry.
  app.tool='grain-add';
  const invalid=create(app);await invalid.detect({kind:'click',point:[40,40]});
  const validPreview=copy(invalid.preview.grain);
  invalid.press(point([0,0]));invalid.drag(point([100,40]));await invalid.release(point([100,40]));
  assert.deepEqual(copy(invalid.preview.grain),validPreview);
  assert.match(app.notice,/valid preview is kept.*Drag a corner/);
  assert.equal(app.noticeAction.label,'Try on canvas');
  assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
  invalid.cancel();assert.equal(app.noticeAction,null);
  const h=create(app);h.defer();const work=h.detect({kind:'click',point:[40,40]});h.cancel();h.resolve();await work;
  assert.equal(h.preview.grain,null);h.accept();assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);
});

test('Canvas pixel fallback disables detection modes and lazy pixels preserve explicit choices and drawn gestures',async()=>{
  const app=current(), project=syncFixture();
  app.loadingProject=true;app.project=project;app.adopt(project);app.viewMode=false;
  app.link.syncAutomatically=false;app.areaId='a';app.loadingProject=false;
  for(const mode of ['click','box','draw']) {
    const h=create(app);h.mode=mode;h.pixels=false;
    h.activate('select');h.activate('grain-add');
    assert.equal(h.mode,'draw');
    assert.deepEqual(h.options.map(o=>!!o.disabled),[true,false,true]);
    for(const option of h.options.filter(o=>o.disabled))assert.match(option.title,/needs image pixels\. Use Draw\./);
    h.mode='click';h.mode='box';assert.equal(h.mode,'draw');
    h.pixels=true;assert.equal(h.mode,mode,'temporary fallback must retain the explicit preference');
    assert.deepEqual(h.options.map(o=>!!o.disabled),[false,false,false]);
    h.pixels=false;h.mode='draw';h.pixels=true;
    assert.equal(h.mode,'draw','an explicit Draw choice must survive pixels arriving');
  }
  const h=create(app);h.pixels=false;h.activate('grain-add');
  h.press(point([0,0]));await h.release(point([0,0]));
  h.pixels=true;assert.equal(h.mode,'draw','starting a drawn gesture is an explicit Draw choice');
  for(const p of [[80,0],[80,80],[0,80]]){h.press(point(p));await h.release(point(p));}
  await h.finish();assert.equal(h.preview.status,'ready');assert.equal(h.preview.source.kind,'draw');
});
