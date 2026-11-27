// Run with GRAINATLAS_SYNC_BASELINE_DIR=/tmp/grainatlas-task2-baseline for the
// recorded original sources, or without it for the working tree.
import './perf/store-loader.mjs';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
const NativeDate=Date;
globalThis.Date=class extends NativeDate { constructor(...args){super(...(args.length?args:['2026-01-01T00:00:00Z']));} };
crypto.randomUUID=()=> 'a0000000-0000-4000-8000-000000000001';
import { Worker as NodeWorker } from 'node:worker_threads';
import { syncFixture } from './perf/sync-fixture.ts';
import { publishFixture } from './perf/publish-fixture.mjs';
const { current }=await import('../ui/src/lib/store.svelte.ts');
const { appStorage }=await import('../ui/src/lib/app-storage.ts');
const { SyncApi }=await import('../ui/src/lib/sync-api.ts');
const { listDrafts }=await import('../ui/src/lib/drafts.ts');
const { HostedSync }=await import('../ui/src/lib/hosted-sync.ts');
const { HostedDraftReports }=await import('../ui/src/lib/hosted-draft-reports.ts');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const workers=[];
globalThis.Worker=class {
  constructor() { this.inner=new NodeWorker(new URL('./perf/sync-worker-node.mjs',import.meta.url));workers.push(this.inner);this.inner.on('message',data=>this.onmessage?.({data}));this.inner.on('error',error=>this.onerror?.(error)); }
  postMessage(value) {this.inner.postMessage(value);}
  terminate(){void this.inner.terminate();}
};
const memory=new Map();let journalWrites=0,journalBytes=0;
Object.defineProperty(globalThis,'localStorage',{configurable:true,value:{getItem:k=>memory.get(k)??null,setItem:(k,v)=>{if(k.startsWith('grainatlas.journal.')){journalWrites++;journalBytes+=v.length;}memory.set(k,String(v));},removeItem:k=>memory.delete(k),get length(){return memory.size;},key:i=>[...memory.keys()][i]??null}});
await appStorage.ready();
const durable = new Map();
appStorage.files = { write: async (key, value) => { await delay(1); if (value === null) durable.delete(key); else durable.set(key, value); } };
globalThis.window=new EventTarget();
globalThis.requestAnimationFrame=run=>setTimeout(()=>run(performance.now()),0);
globalThis.cancelAnimationFrame=clearTimeout;
globalThis.__GRAINATLAS_PERF__=true;
const app=current(),p=syncFixture();app.loadingProject=true;app.project=p;app.adopt(p);app.viewMode=false;app.link.syncAutomatically=false;app.loadingProject=false;
app.sync.uuid=p.uuid;
const base=JSON.parse(JSON.stringify(app.sync.elements));
const hashes={spots:'s1',grains:'g1',labels:'l1'};
const draft={uuid:p.uuid,name:p.name,mount:p.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,base_hashes:hashes,synced:process.env.GRAINATLAS_SYNC_MODE!=='snapshot',pending:null,pixels_base:null,pixels_changed:false,updated_at:'fixed',changes:{spots:0,grains:0,labels:0}};
app.sync.draft=draft;
const status={project_uuid:p.uuid,mount_id:p.upbdb.mount_id,revision:1,can_edit:true,hashes,atlas_version:1,counts:{spots:500,grains:150,labels:0}};
const owned = elements => Object.fromEntries(['spots','grains','labels'].map(k=>[k,elements[k].map(e=>({...e,project_uuid:p.uuid}))]));
let server={...owned(base),revision:1,hashes},serverBytes=JSON.stringify(server),replyBytes='',accepted,acceptedBytes,calls=0,requestBytes=0,requestHash='';
globalThis.fetch=async(url,init)=>{calls++;await delay(5);const name=String(url);if(name.endsWith('/elements'))return new Response(serverBytes);if(name.endsWith('/sync')){requestBytes=init.body.length;requestHash=createHash('sha256').update(init.body).digest('hex');server=accepted;serverBytes=acceptedBytes;return new Response(replyBytes);}return Response.json({...status,revision:server.revision});};
app.sync.api=async()=>new SyncApi('https://example.invalid',null);
const host=new HostedSync(()=>{});
const reports=new HostedDraftReports(()=>listDrafts(),drafts=>host.drafts(drafts));
const listener=process.env.GRAINATLAS_SYNC_BASELINE_DIR ? ()=>host.drafts(listDrafts()) : event=>reports.changed(event.detail);
window.addEventListener('grainatlas:drafts-changed',listener);
reports.changed();await delay(10);
const output=[];
async function phase(name,run) {
  const gaps=[];let last=performance.now();const timer=setInterval(()=>{const now=performance.now();gaps.push(now-last);last=now;},1);
  performance.clearMeasures();const beforeCalls=calls,beforeJournals=journalWrites,beforeBytes=journalBytes,start=performance.now();
  await run();await delay(10);clearInterval(timer);
  const groups={};for(const entry of performance.getEntriesByType('measure')){if(entry.detail?.thread!=='main')continue;const row=groups[entry.name]??={n:0,maxMs:0,totalMs:0};row.n++;row.maxMs=Math.max(row.maxMs,entry.duration);row.totalMs+=entry.duration;}
  output.push({phase:name,wallMs:performance.now()-start,longestEventLoopGapMs:Math.max(...gaps),gapsAbove50ms:gaps.filter(n=>n>50).length,requests:calls-beforeCalls,journalWrites:journalWrites-beforeJournals,journalCharacters:journalBytes-beforeBytes,mainSpans:groups});
}
try {
  await phase('check',()=>app.sync.check());
  await phase('register-20-create-and-rename',async()=>{
    for(let i=0;i<20;i++){app.checkpoint();app.spots=[...app.spots,{...app.spots[0],id:`new-${i}`,name:`new-${i}`,acquisition_id:undefined}];app.persist();app.checkpoint();app.spots=app.spots.map(s=>s.id===`new-${i}`?{...s,name:`renamed-${i}`}:s);app.persist();}
    await delay(250);await appStorage.flushed();
  });
  accepted={...owned(JSON.parse(JSON.stringify(app.sync.elements))),revision:2,hashes:{...hashes,spots:'s2'}};acceptedBytes=JSON.stringify(accepted);
  replyBytes=JSON.stringify({ok:true,mount_id:p.upbdb.mount_id,project_uuid:p.uuid,mode:process.env.GRAINATLAS_SYNC_MODE==='snapshot'?'snapshot':'delta',revision:2,previous_revision:1,applied:{spots:{created:20,updated:0,deleted:0,unchanged:500},grains:{created:0,adopted:0,updated:0,retired:0,unchanged:150},labels:{created:0,updated:0,retired:0,unchanged:0}},ids:{spots:{},grains:{},labels:{}},not_in_snapshot:{spots:[],grains:[],labels:[]},complete:true,warnings:[],rebased_over:0,dry_run:false});
  await phase('sync',()=>app.sync.sync());assert.equal(app.sync.error,'');
  await phase('sync-durable-tail',()=>appStorage.flushed());
  const storedDraft=durable.get(`grainatlas.draft.${p.uuid}`);assert.equal(storedDraft,JSON.stringify(app.sync.draft));
  const draftHash=createHash('sha256').update(storedDraft).digest('hex');
  assert.equal(requestHash,process.env.GRAINATLAS_SYNC_MODE==='snapshot'?'54d2db1569f70f838b8f96f546b89fa573cde5d821b0a1cf51b3f32cbfaa35e3':'e59716944caa4fb11992db491741d70eff8f5950f6585187054ee7a1729c7e1b');
  assert.equal(draftHash,'8bd33efae6fe41354cb859d868a3290c325ca261d8f80daaec4e8fb1928c8091');
  // Exercise real publishImages/engine after a synthetic, prebuilt core export.
  // Native/wasm image encoding is already off-thread and excluded here.
  const bundle=publishFixture(app.fullProject());
  let coreRecord=structuredClone(p);delete coreRecord.grainatlas_ui;
  app.core=Promise.resolve({atlasZip:async()=>bundle.zip,recordPublished:async rec=>{coreRecord.upbdb.published=[rec];},projectJson:async()=>JSON.stringify(coreRecord)});
  const publishedBytes=JSON.stringify(app.fullProject());
  globalThis.fetch=async(url,init={})=>{
    calls++;await delay(5);const path=new URL(url).pathname;
    if(path==='/v1/assets/check')return Response.json({have:[]});
    if(path.endsWith('/atlas/uploads'))return Response.json({upload_id:'synthetic-upload',missing:bundle.atlas.files.map(f=>f.sha256),expires_at:'2099-01-01T00:00:00Z'});
    if(init.method==='PUT')return Response.json({stored:true},{status:201});
    if(path.endsWith('/commit'))return Response.json({version:2});
    if(path.endsWith('/project.json'))return new Response(publishedBytes);
    return Response.json({...status,revision:2});
  };
  await phase('publish-images',()=>app.sync.publishImages(p.uuid));
  assert.equal(app.sync.imageError,'');
  await phase('publish-durable-tail',()=>appStorage.flushed());
  console.log(JSON.stringify({mode:process.env.GRAINATLAS_SYNC_MODE??'delta',draftHash,requestHash,version:process.env.GRAINATLAS_SYNC_BASELINE_DIR?'baseline':'working-tree',node:process.version,grains:150,spots:500,projectBytes:JSON.stringify(p).length,requestBytes,method:'Real rune store/SyncSession/AppStorage with invented geometry, memory localStorage, memory durable-file adapter and delayed mocked HTTP. 1 ms event-loop sampler reports maximum gap, a Node proxy for blocking (NOT browser long tasks). New code uses a Node worker adapter for the same pure browser jobs. Original sources loaded through an optional baseline directory. Browser/Svelte DOM, disk IPC and localStorage disk cost excluded.',phases:output},null,2));
} finally {app.sync.dispose();reports.dispose();for(const worker of workers)await worker.terminate();}
