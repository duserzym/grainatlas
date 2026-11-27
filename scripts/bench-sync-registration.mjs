import './perf/store-loader.mjs';
import { syncFixture } from './perf/sync-fixture.ts';
import { performance } from 'node:perf_hooks';
const { current } = await import('../ui/src/lib/store.svelte.ts');
const { diffElements, mergeElements, changeCounts, storedElements } = await import('../ui/src/lib/sync.ts');
const { canonicalJson, elementHash } = await import('../ui/src/lib/sync-hash.ts');
const { syncWireBody } = await import('../ui/src/lib/sync-api.ts');
const { writeDraft } = await import('../ui/src/lib/drafts.ts');
const { appStorage } = await import('../ui/src/lib/app-storage.ts');
const memory=new Map();
Object.defineProperty(globalThis,'localStorage',{configurable:true,value:{getItem:k=>memory.get(k)??null,setItem:(k,v)=>memory.set(k,String(v)),removeItem:k=>memory.delete(k),get length(){return memory.size;},key:i=>[...memory.keys()][i]??null}});
const rows={};
function measure(name, fn) {const start=performance.now();const result=fn();(rows[name]??=[]).push(performance.now()-start);return result;}
const p=syncFixture(), app=current();app.loadingProject=true;app.project=p;app.adopt(p);app.viewMode=false;app.link.syncAutomatically=false;app.loadingProject=false;
const base=JSON.parse(JSON.stringify(app.sync.elements));
app.sync.draft={uuid:p.uuid,name:p.name,mount:p.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,synced:true,pending:null,pixels_base:null,pixels_changed:false,updated_at:'2026-01-01T00:00:00Z',changes:{spots:0,grains:0,labels:0}};
for(let run=0;run<12;run++) {
  const local=measure('sync.clone-elements',()=>JSON.parse(JSON.stringify(app.sync.elements)));
  const changes=measure('sync.diff',()=>diffElements(base,local));
  measure('sync.change-counts',()=>changeCounts(changes));
  measure('sync.merge',()=>mergeElements(base,local,base));
  const body={project_uuid:p.uuid,base_revision:1,mode:'snapshot',...diffElements({spots:[],grains:[],labels:[]},local)};
  measure('sync.wire-and-key',()=>canonicalJson(syncWireBody(body)));
  measure('check.parse-elements',()=>JSON.parse(JSON.stringify(base)));
  measure('register.20-edits-and-flush',()=>{for(let i=0;i<20;i++){app.checkpoint();app.spots=[...app.spots,{...app.spots[0],id:`new-${run}-${i}`,name:`new-${i}`,acquisition_id:undefined}];app.persist();}app.flushPersistence();});
  measure('register.track',()=>app.sync.track(false));
  measure('draft.stringify-and-store',()=>writeDraft(app.sync.draft));
}
await appStorage.flushed();
const hashes={};for(const k of ['spots','grains','labels']){const start=performance.now();hashes[k]=await elementHash(storedElements(base)[k]);(rows['test-only.hash']??=[]).push(performance.now()-start);}
console.log(JSON.stringify({node:process.version,grains:150,spots:500,verticesPerGrain:256,projectBytes:JSON.stringify(p).length,draftBytes:JSON.stringify(app.sync.draft).length,method:'12 repetitions; synchronous elapsed sections in Node, not browser long tasks. Registration grows by 20 new spots per repetition. Hash is async wall time and test-only.',phases:Object.fromEntries(Object.entries(rows).map(([k,v])=>[k,{n:v.length,medianMs:[...v].sort((a,b)=>a-b)[Math.floor(v.length/2)],maxMs:Math.max(...v)}])),hashes},null,2));
