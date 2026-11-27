import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import '../../../scripts/perf/store-loader.mjs';
import { syncFixture } from '../../../scripts/perf/sync-fixture.ts';
const { current } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const copy = value => JSON.parse(JSON.stringify(value));

// Compile the real Canvas state and pointer handlers with the Svelte client compiler.
// Pure polygon/history tests miss deep-state proxying of a raw store record. No DOM,
// pixel worker, network or port is needed to exercise press -> queued drag -> commit.
const canvas = readFileSync(new URL('../screens/Canvas.svelte',import.meta.url),'utf8');
const section = (start,end) => canvas.slice(canvas.indexOf(start),canvas.indexOf(end,canvas.indexOf(start)));
const source = `
import { editedOutline, outlineHandle } from '${new URL('./outline-vertices.ts',import.meta.url).href}';
import { latestFrame } from '${new URL('./canvas-pointer.ts',import.meta.url).href}';
import { CLICK_SLOP } from '${new URL('./canvas/gestures.ts',import.meta.url).href}';
export function create(app) {
  const ro=false, areaId='a', umPerPx=.5, k=.25, isViewer=false;
  const detections=$derived(app.grains.a), editableGrain=$derived(detections[0]);
  let collapseSpot=null, pressScreen=null, dragCrossedSlop=false, livePrevious=null,
      pendingMoveSnap=null, lastHover=null, snapHint=null, gesture=null, previousSpot=null,
      splitStroke=null, lasso=null, stroke=null, imageDrag=null, imageDraft=null,
      rectangle=null, drawing=null, queued=null;
  const imagePress=()=>false, hoverFrame={flush(){},clear(){}};
  const dragFrame=latestFrame(dragNow,fn=>{queued=fn;return 1;},()=>{queued=null;});
  ${canvas.match(/let vertexDrag = [^\n]+/)[0]}
  ${section('function saveVertices(', 'let lastHover:')}
  ${section('function press(', 'function drag(p:')}
  ${section('function drag(p:', 'async function release(')}
  ${section('async function release(', 'function cancelGesture(')}
  ${section('function cancelGesture(', 'async function doSplit(')}
  return { press, drag, release, cancel:cancelGesture, frame:()=>queued?.(),
    get points(){return vertexDrag?.points;}, get base(){return vertexDrag?.base;} };
}`;
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText;
const compiled=compileModule(js,{filename:'outline-gesture.svelte.ts',generate:'client'}).js.code
  .replaceAll("'svelte/internal/client'",JSON.stringify(import.meta.resolve('svelte/internal/client')));
const { create } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const point = ([x,y],ev={}) => ({x:x/.5,y:y/.5,sx:x,sy:y,ev});

test('Canvas vertex and edge gestures commit once, preserve all registered fields, and undo exactly',async()=>{
  const app=current(),project=syncFixture(),memory=new Map();
  const grain=project.grainatlas_ui.grains.a[0];
  grain.polygon_um=[[0,0],[80,0],[80,80],[0,80]];
  appStorage.getItem=key=>memory.get(key)??null;appStorage.setItem=(key,value)=>memory.set(key,value);
  app.loadingProject=true;app.project=project;app.adopt(project);app.viewMode=false;
  app.link.syncAutomatically=false;app.tool='grain-edit';app.loadingProject=false;app.clearHistory();
  const before=copy(app.fullProject().grainatlas_ui),base=copy(app.sync.elements);
  app.sync.draft={uuid:project.uuid,name:project.name,mount:project.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,synced:true,pending:null,pixels_base:null,pixels_changed:false,updated_at:'fixed',changes:{spots:0,grains:0,labels:0}};
  let gesture=create(app);
  assert.equal(gesture.press(point([0,0])),true);
  assert.equal(gesture.base,app.grains.a[0]);
  gesture.drag(point([-5,-5]));
  // Release before the animation frame must flush the queued final point.
  await gesture.release(point([-5,-5]));app.flushPersistence();
  assert.deepEqual(app.grains.a[0].polygon_um,[[-5,-5],[80,0],[80,80],[0,80]]);
  assert.deepEqual(copy(app.spots),before.spots);assert.deepEqual(app.sync.draft.changes,{spots:0,grains:1,labels:0});
  assert.deepEqual(app.sync.draft.pending.grains[0].polygon_um,app.grains.a[0].polygon_um);
  app.undo();app.flushPersistence();assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
  app.redo();app.flushPersistence();assert.deepEqual(app.grains.a[0].polygon_um[0],[-5,-5]);app.undo();app.flushPersistence();

  gesture=create(app);gesture.press(point([40,0]));await gesture.release(point([40,0]));app.flushPersistence();
  assert.deepEqual(app.grains.a[0].polygon_um,[[0,0],[40,0],[80,0],[80,80],[0,80]]);
  gesture=create(app);gesture.press(point([40,0],{altKey:true}));await gesture.release(point([40,0]));app.flushPersistence();
  assert.deepEqual(app.grains.a[0].polygon_um,before.grains.a[0].polygon_um);
  assert.deepEqual(copy(app.spots),before.spots);
  app.undo();app.undo();app.flushPersistence();assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);

  gesture=create(app);gesture.press(point([0,0]));gesture.drag(point([-5,-5]));gesture.frame();
  assert.deepEqual(gesture.points[0],[-5,-5]);assert.deepEqual(app.grains.a[0].polygon_um,before.grains.a[0].polygon_um);
  gesture.cancel();await gesture.release(point([-5,-5]));app.flushPersistence();
  assert.deepEqual(copy(app.fullProject().grainatlas_ui),before);assert.equal(app.canUndo,false);
});
