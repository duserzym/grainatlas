import assert from 'node:assert/strict';
import test from 'node:test';
import { Worker } from 'node:worker_threads';
import '../../../scripts/perf/store-loader.mjs';
import { syncFixture } from '../../../scripts/perf/sync-fixture.ts';
const { current } = await import('./store.svelte.ts');
const { appStorage } = await import('./app-storage.ts');
const { flushWrites } = await import('./deferred-writes.ts');
const { runSyncWork } = await import('./sync-compute-work.ts');
const { canonicalJson, elementHash } = await import('./sync-hash.ts');
const { syncWireBody } = await import('./sync-api.ts');
const { diffElements, syncChanges, storedElements, firstSyncMerge, groupedGrains, uiSpots } = await import('./sync.ts');
const { pendingElements, writeDraft } = await import('./drafts.ts');
const copy = value => JSON.parse(JSON.stringify(value));

// Saved JSON keeps every original field; only per-record canonical additions are allowed.
function assertCanonicalRecords(before, after, canonical, additions) {
  assert.equal(after.length, before.length);
  assert.deepEqual(after.map(row => row.id), before.map(row => row.id));
  const rows = new Map(after.map(row => [row.id, row]));
  const server = new Map(canonical.map(row => [row.id, row]));
  assert.equal(rows.size, before.length);
  for (const old of before) {
    const row = rows.get(old.id), accepted = server.get(old.id);
    assert.ok(row, old.id);assert.ok(accepted, old.id);
    for (const key of Object.keys(old)) {
      assert.ok(Object.hasOwn(row, key), `${old.id}.${key} retained`);
      assert.deepEqual(row[key], old[key], `${old.id}.${key} unchanged`);
    }
    for (const key of Object.keys(row).filter(key => !Object.hasOwn(old, key))) {
      assert.ok(additions.includes(key), `${old.id}.${key} allowed`);
      assert.ok(Object.hasOwn(accepted, key), `${old.id}.${key} canonical`);
      assert.deepEqual(row[key], accepted[key], `${old.id}.${key} canonical value`);
    }
  }
}
function assertAcknowledgedRecords(previous, after, canonical) {
  assertCanonicalRecords(previous.spots, copy(after.spots), canonical.spots, ['acquisition_ids', 'project_uuid']);
  assert.deepEqual(Object.keys(after.grains), Object.keys(previous.grains));
  assertCanonicalRecords(Object.values(previous.grains).flat(), copy(Object.values(after.grains).flat()), canonical.grains, ['project_uuid', 'retired', 'merged_from']);
  for (const [area, grains] of Object.entries(previous.grains)) assert.deepEqual(after.grains[area].map(g => g.id), grains.map(g => g.id));
  const server = new Map(canonical.spots.map(spot => [spot.id, spot]));
  for (const spot of after.spots) {
    assert.deepEqual(spot.acquisition_ids, server.get(spot.id).acquisition_ids);
    assert.equal(spot.project_uuid, server.get(spot.id).project_uuid);
  }
}

test('worker preserves exact wire bytes, canonical request keys, diffs, first merge and hashes for 150 grains/500 registered spots', async () => {
  const { elementsOf } = await import('./sync.ts');
  const elements = elementsOf(syncFixture());
  const worker = new Worker(new URL('../../../scripts/perf/sync-worker-node.mjs',import.meta.url));
  let nextId=0;
  const job = job => new Promise((resolve,reject)=>{
    const id=nextId++;
    worker.once('error',reject); worker.once('message',reply=>{worker.removeListener('error',reject);reply.error?reject(new Error(reply.error)):resolve(reply.value);});
    worker.postMessage({id,job});
  });
  try {
    const changed=copy(elements);changed.spots.push({...changed.spots[0],id:'new',name:'new',acquisition_id:undefined});
    for(const mode of ['snapshot','delta']) {
      const body={project_uuid:'synthetic',base_revision:1,mode,...syncChanges(mode,elements,changed,elements,'synthetic',elements)};
      assert.equal(await job({op:'key',body}),canonicalJson(syncWireBody(body)));
      const request={pending_request:{body,mine:changed,current:elements,id:'fixed',wire_id:undefined}};
      assert.equal(JSON.stringify(await job({op:'copy',value:request})),JSON.stringify(copy(request)));
      for(const dry_run of [undefined,true])for(const confirm_bulk of [undefined,true]) {
        const b={...body,dry_run,confirm_bulk};
        assert.equal(await job({op:'wire',body:b,requestId:'fixed'}),JSON.stringify({...syncWireBody(b),...(!dry_run?{request_id:'fixed'}:{})}));
      }
      const prepared=await job({op:'prepare',mode,base:elements,mine:changed,current:elements,uuid:'synthetic',seen:elements,since:elements});
      assert.equal(JSON.stringify(prepared.diff),JSON.stringify(syncChanges(mode,elements,changed,elements,'synthetic',elements)));
      assert.deepEqual(prepared.sinceSeen,diffElements(elements,elements));
    }
    assert.deepEqual(await job({op:'first-merge',base:elements,mine:elements,current:elements,uuid:'synthetic'}),firstSyncMerge(elements,elements,elements,'synthetic'));
    const project=syncFixture(),previous=project.grainatlas_ui;
    const display=await job({op:'display',elements,grains:previous.grains,spots:previous.spots,samples:{a:'synthetic'}});
    assert.equal(JSON.stringify(display),JSON.stringify({spots:uiSpots(copy(elements.spots),elements.grains,previous.spots,{a:'synthetic'}),grains:groupedGrains(copy(elements.grains),previous.grains),labels:copy(elements.labels.filter(l=>!l.retired))}));
    const parsed=await job({op:'parse',text:JSON.stringify(elements)});
    assert.equal(JSON.stringify(parsed),JSON.stringify(elements));
    const expected={spots:'16c832d050ee9654f619a0646714c9e96f7a692aa2acf257c84c863081eb9602',grains:'0c35c429b4c4a8a4d37bc75fd2640ee1e66685ed78072ed0472e7a18e7647572',labels:'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'};
    for(const kind of ['spots','grains','labels'])assert.equal(await elementHash(storedElements(parsed)[kind]),expected[kind]);
    await assert.rejects(job({op:'parse',text:'broken'}));
  } finally { await worker.terminate(); }
});

test('acknowledgement worker and fallback equal the 0.1.12 synchronous display mapping byte-for-byte', async () => {
  const { elementsOf } = await import('./sync.ts');
  const project = syncFixture(), previous = copy(project.grainatlas_ui), canonical = copy(elementsOf(project));
  canonical.spots = canonical.spots.map(s => ({ ...s, project_uuid: project.uuid, acquisition_ids: [s.acquisition_id, `new-link-${s.id}`] }));
  canonical.grains = canonical.grains.map(g => ({ ...g, project_uuid: project.uuid, retired: false, merged_from: [] }));
  const samples = { a: 'synthetic' };
  const input = { op: 'display', elements: canonical, grains: previous.grains, spots: previous.spots, samples };
  // These unchanged synchronous mappers are the original 0.1.12 acknowledgement route.
  const synchronous = () => ({
    spots: uiSpots(copy(canonical.spots), copy(canonical.grains), previous.spots, samples),
    grains: groupedGrains(copy(canonical.grains), previous.grains),
    labels: copy(canonical.labels.filter(l => !l.retired)),
  });
  const worker = new Worker(new URL('../../../scripts/perf/sync-worker-node.mjs', import.meta.url));
  let id = 0;
  const job = job => new Promise((resolve, reject) => {
    worker.once('error', reject);
    worker.once('message', reply => {worker.removeListener('error', reject);reply.error ? reject(new Error(reply.error)) : resolve(reply.value);});
    worker.postMessage({ id: id++, job });
  });
  const previousWorker = globalThis.Worker;
  globalThis.Worker = undefined;
  try {
    // Isolate the worker-less client so the session regression can still use its worker.
    const { syncCompute } = await import('./sync-compute.ts?acknowledgement-fallback');
    assert.equal(previous.spots.length, 500);
    for (const output of [await job(input), await syncCompute(input), runSyncWork(input)]) {
      assert.equal(JSON.stringify(output), JSON.stringify(synchronous()));
      assert.equal(output.spots.length, 500);
      assertAcknowledgedRecords(previous, output, canonical);
    }
    // Explicit geometry changes remain visible, including changes below diff tolerance.
    canonical.spots[0].x_um += .00000001;
    canonical.grains[0].polygon_um[0][0] += .00000001;
    for (const edited of [await job(input), await syncCompute(input)]) {
      assert.equal(JSON.stringify(edited), JSON.stringify(synchronous()));
      assert.equal(edited.spots[0].x_um, canonical.spots[0].x_um);
      assert.deepEqual(edited.grains.a[0].polygon_um, canonical.grains[0].polygon_um);
    }
  } finally { globalThis.Worker = previousWorker;await worker.terminate(); }
});

test('registration burst coalesces exact draft, protects registered records, history and unchanged display arrays', () => {
  const memory=new Map(),app=current(),p=syncFixture();
  appStorage.getItem=k=>{flushWrites();return memory.get(k)??null;};
  appStorage.setItem=(k,v)=>memory.set(k,v);
  app.loadingProject=true;app.project=p;app.adopt(p);app.viewMode=false;app.link.syncAutomatically=false;app.loadingProject=false;app.clearHistory();
  app.sync.displayElements(copy(app.sync.elements));
  const base=copy(app.sync.elements),registered=JSON.stringify(app.spots),grains=JSON.stringify(app.grains);
  app.sync.draft={uuid:p.uuid,name:p.name,mount:p.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,synced:true,pending:null,pixels_base:null,pixels_changed:false,updated_at:'fixed',changes:{spots:0,grains:0,labels:0}};
  let writes=0;appStorage.track=(_app,kind,write)=>{if(kind==='draft')writes++;return write();};
  for(let i=0;i<20;i++) {
    app.checkpoint();app.spots=[...app.spots,{...app.spots[0],id:`new-${i}`,name:`new-${i}`,acquisition_id:undefined}];app.persist();
    app.checkpoint();app.spots=app.spots.map(s=>s.id===`new-${i}`?{...s,name:`renamed-${i}`}:s);app.persist();
  }
  assert.equal(writes,0);app.flushPersistence();assert.equal(writes,1);
  assert.equal(JSON.stringify(app.spots.slice(0,500)),registered);assert.equal(JSON.stringify(app.grains),grains);
  assert.deepEqual(app.sync.draft.changes,{spots:20,grains:0,labels:0});
  const expected={...app.sync.draft,pending:pendingElements(app.sync.elements)};
  const isolated=new Map(),storage={getItem:k=>isolated.get(k)??null,setItem:(k,v)=>isolated.set(k,v)};
  writeDraft(expected,storage);
  assert.equal(memory.get(`grainatlas.draft.${p.uuid}`),isolated.get(`grainatlas.draft.${p.uuid}`));
  app.undo();app.flushPersistence();assert.equal(app.spots.at(-1).name,'new-19');app.redo();app.flushPersistence();assert.equal(app.spots.at(-1).name,'renamed-19');
  // Normalize once using the pre-existing mapping, then repeat a no-change acknowledgement.
  app.sync.displayElements(copy(app.sync.elements));
  const refs={spots:app.spots,grains:app.grains,labels:app.labels};
  app.sync.displayElements(copy(app.sync.elements));
  assert.equal(app.spots,refs.spots);assert.equal(app.grains,refs.grains);assert.equal(app.labels,refs.labels);
  assert.equal(JSON.stringify(app.spots.slice(0,500)),registered);
  app.flushPersistence();
});

test('an edit during worker display is rebased again; outbox stays durable and 500 registered records gain canonical metadata', async () => {
  const { SyncSession }=await import('./sync-session.svelte.ts');
  const app=current(),p=syncFixture(),memory=new Map();
  appStorage.getItem=k=>{flushWrites();return memory.get(k)??null;};appStorage.setItem=(k,v)=>memory.set(k,v);appStorage.track=(_app,_kind,write)=>write();
  app.flushPersistence();app.loadingProject=true;app.project=p;app.adopt(p);app.viewMode=false;app.link.syncAutomatically=false;app.loadingProject=false;
  const session=new SyncSession(app);app.sync=session;
  const base=copy(session.elements),registered=copy(app.spots),grains=copy(app.grains),hashes={spots:'s1',grains:'g1',labels:'l1'};
  session.draft={uuid:p.uuid,name:p.name,mount:p.upbdb.mount_id,server:'https://example.invalid',base,base_revision:1,synced:true,pending:null,pixels_base:null,pixels_changed:false,updated_at:'fixed',changes:{spots:0,grains:0,labels:0}};
  session.registered=true;session.reachable=true;session.status={project_uuid:p.uuid,mount_id:p.upbdb.mount_id,revision:1,hashes,can_edit:true,counts:{spots:500,grains:150,labels:0}};
  app.checkpoint();app.spots=[...app.spots,{...app.spots[0],id:'new-in-flight',name:'submitted',acquisition_id:undefined}];app.persist();
  const submitted=copy(session.elements);let accepted,requests=0,displayCalls=0,release;
  let started;const displayStarted=new Promise(resolve=>{started=resolve;});
  const previous=globalThis.Worker;
  globalThis.Worker=class {
    postMessage({id,job}) {
      const value=runSyncWork(structuredClone(job));
      const reply=()=>queueMicrotask(()=>this.onmessage({data:{id,value:structuredClone(value)}}));
      if(job.op==='display'&&displayCalls++===0){release=reply;started();}else reply();
    }
    terminate() {}
  };
  session.api=async()=>({
    elements:async()=>accepted??{...base,revision:1,hashes},
    lookup:async()=>({...session.status,revision:2}),
    sync:async(_mount,body,id)=>{
      requests++;
      const persisted=JSON.parse(memory.get(`grainatlas.draft.${p.uuid}`));
      assert.equal(persisted.pending_request.id,id);assert.equal(persisted.pending_request.body.spots.upsert[0].name,'submitted');
      assert.equal(body.spots.upsert.length,1);assert.equal(body.spots.upsert[0].id,'new-in-flight');assert.deepEqual(body.spots.delete,[]);
      accepted={...Object.fromEntries(['spots','grains','labels'].map(k=>[k,submitted[k].map(e=>({...e,project_uuid:p.uuid,...(k==='spots'?{acquisition_ids:e.acquisition_id?[e.acquisition_id,`new-link-${e.id}`]:[]}:k==='grains'?{retired:false,merged_from:[]}:{})}))])),revision:2,hashes};
      return {ok:true,revision:2,previous_revision:1,mode:body.mode,applied:{spots:{created:1,updated:0,deleted:0},grains:{created:0,adopted:0,updated:0,retired:0},labels:{created:0,updated:0,retired:0}},not_in_snapshot:{spots:[],grains:[],labels:[]},complete:true,warnings:[],rebased_over:0};
    },
  });
  try {
    const syncing=session.sync();await displayStarted;
    app.checkpoint();app.spots=app.spots.map(s=>s.id==='new-in-flight'?{...s,name:'edited-during-worker'}:s);app.persist();app.flushPersistence();
    release();await syncing;app.flushPersistence();
    assert.equal(session.error,'');assert.equal(requests,1);assert.equal(displayCalls,2);
    assert.equal(app.spots.at(-1).name,'edited-during-worker');assert.equal(session.draft.base.spots.at(-1).name,'submitted');
    assert.equal(session.draft.pending_request,null);assert.deepEqual(session.counts,{spots:1,grains:0,labels:0});
    assert.equal(app.spots.slice(0,500).length,500);
    assertAcknowledgedRecords({spots:registered,grains},{spots:app.spots.slice(0,500),grains:app.grains},accepted);
    assert.equal(JSON.parse(memory.get(`grainatlas.draft.${p.uuid}`)).pending.spots.at(-1).name,'edited-during-worker');
    assert.equal(session.draft.base.spots[0].project_uuid,p.uuid);assert.deepEqual(session.draft.base.spots[0].acquisition_ids,accepted.spots[0].acquisition_ids);
    for(let i=0;i<20;i++) {
      app.checkpoint();app.spots=[...app.spots,{...app.spots[0],id:`burst-${i}`,name:`new-${i}`,acquisition_id:undefined}];app.persist();
      app.checkpoint();app.spots=app.spots.map(s=>s.id===`burst-${i}`?{...s,name:`renamed-${i}`}:s);app.persist();
    }
    app.flushPersistence();
    const saved=app.fullProject().grainatlas_ui;
    assert.equal(saved.spots.length,521);assert.equal(saved.spots.filter(s=>s.name.startsWith('registered-')).length,500);
    assert.equal(registered.length,500);assert.equal(saved.spots.filter(s=>s.name.startsWith('renamed-')).length,20);
    assertAcknowledgedRecords({spots:registered,grains},{spots:saved.spots.filter(s=>s.name.startsWith('registered-')),grains:saved.grains},accepted);
  } finally {session.dispose();globalThis.Worker=previous;}
});

test('4,000 element rows revalidate with two requests and identical decoded data',async t=>{
  const {SyncApi}=await import('./sync-api.ts');
  const spots=Array.from({length:4000},(_,i)=>({id:`synthetic-${i}`,name:`row-${i}`,area_id:'a',x_um:i,y_um:i}));
  const reply={spots,grains:[],labels:[],revision:1,hashes:{spots:'s1',grains:'g1',labels:'l1'}};
  let calls=0;
  t.mock.method(globalThis,'fetch',async(_url,init)=>{
    calls++;
    if(calls===2){assert.equal(init.headers['If-None-Match'],'"synthetic-large"');return new Response(null,{status:304});}
    return Response.json(reply,{headers:{ETag:'"synthetic-large"'}});
  });
  const start=performance.now(),api=new SyncApi('https://synthetic-large.test',null);
  assert.deepEqual(await api.elements('synthetic'),reply);assert.deepEqual(await api.elements('synthetic'),reply);
  assert.equal(calls,2);t.diagnostic(`4,000 rows: ${calls} requests including 304 revalidation, ${(performance.now()-start).toFixed(1)} ms (mock timings)`);
});
