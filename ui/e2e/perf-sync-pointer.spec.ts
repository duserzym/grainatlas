// PERF=1 PERF_LABEL=after pnpm exec playwright test e2e/perf-sync-pointer.spec.ts
import { test, expect } from './mock-session';
import type { Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { syncFixture } from '../../scripts/perf/sync-fixture';
import { root, generated, mockImageElements, mockImageServer, projectURL, saveImageProject } from './support/image-edit';
import { distribution, report, outputPath } from './support/perf';

test.beforeAll(()=>{execFileSync('cargo',['run','-p','grainatlas-core','--example','print_edit_bundle'],{cwd:root,stdio:'pipe'});});
async function startCapture(page: Page) {
  await page.evaluate(()=>{
    const w=window as any;w.__GRAINATLAS_PERF__=true;
    const capture=w.__grainPerf={start:performance.now(),longTasks:[] as number[],spans:[] as {name:string;ms:number}[],observers:[] as PerformanceObserver[]};
    performance.clearMeasures();
    if(PerformanceObserver.supportedEntryTypes.includes('longtask')) {
      const observer=new PerformanceObserver(list=>{for(const entry of list.getEntries())if(entry.startTime>=capture.start)capture.longTasks.push(entry.duration);});
      observer.observe({type:'longtask',buffered:true});capture.observers.push(observer);
    }
    const observer=new PerformanceObserver(list=>{for(const entry of list.getEntries())if(entry.startTime>=capture.start&&(entry as PerformanceMeasure).detail?.thread==='main')capture.spans.push({name:entry.name,ms:entry.duration});});
    observer.observe({type:'measure'});capture.observers.push(observer);
  });
}
async function stopCapture(page: Page) {
  return page.evaluate(async()=>{
    await new Promise<void>(resolve=>setTimeout(resolve,50));
    const capture=(window as any).__grainPerf;
    for(const observer of capture.observers)observer.disconnect();
    return {longTasks:capture.longTasks as number[],spans:capture.spans as {name:string;ms:number}[],longTaskSupported:PerformanceObserver.supportedEntryTypes.includes('longtask')};
  });
}
async function moves(page: Page) {
  return page.getByRole('application',{name:'Mosaic canvas'}).evaluate(async host=>{
    const rect=host.getBoundingClientRect(),handlerMs:number[]=[],frameMs:number[]=[],settleMs:number[]=[];
    let frame=0,last:number|null=null,busyBursts=0,registrationBursts=0;
    const sample=(now:number)=>{if(last!==null)frameMs.push(now-last);last=now;frame=requestAnimationFrame(sample);};frame=requestAnimationFrame(sample);
    for(let burst=0;burst<30;burst++) {
      await new Promise<void>(r=>requestAnimationFrame(()=>r()));
      if(document.querySelector('[data-testid="sync-chip"]')?.getAttribute('aria-busy')==='true')busyBursts++;
      if((window as any).__grainRegistering)registrationBursts++;
      const start=performance.now();
      for(let j=0;j<4;j++) {
        const i=burst*4+j,x=rect.left+rect.width*.32+Math.sin(i*.37)*70,y=rect.top+rect.height*.4+Math.cos(i*.29)*60;
        const target=document.elementFromPoint(x,y);
        if(!target?.closest('[data-view]'))throw new Error('Performance pointer covered by UI');
        const t=performance.now();target.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:x,clientY:y,pointerId:77,pointerType:'mouse',altKey:i%11===0}));handlerMs.push(performance.now()-t);
      }
      await new Promise<void>(r=>requestAnimationFrame(()=>r()));await new Promise<void>(r=>requestAnimationFrame(()=>r()));settleMs.push(performance.now()-start);
    }
    cancelAnimationFrame(frame);return {handlerMs,frameMs,settleMs,busyBursts,registrationBursts};
  });
}
function budgets(value: Awaited<ReturnType<typeof moves>>, trace: Awaited<ReturnType<typeof stopCapture>>) {
  const handler=distribution(value.handlerMs),frames=distribution(value.frameMs),settle=distribution(value.settleMs);
  expect(handler.p90!).toBeLessThan(8);expect(frames.p90!).toBeLessThan(34);expect(settle.p90!).toBeLessThan(50);
  expect(trace.longTaskSupported).toBe(true);
  expect(Math.max(0,...trace.longTasks)).toBeLessThan(75);
  expect(trace.longTasks.reduce((n,ms)=>n+Math.max(0,ms-50),0)).toBeLessThan(50);
  return {handler,frames,settle,longTasks:trace.longTasks,mainSpans:trace.spans,raw:value};
}
async function blankPoints(page: Page, count: number) {
  return page.getByRole('application',{name:'Mosaic canvas'}).evaluate((host,count)=>{
    const rect=host.getBoundingClientRect(),space=document.querySelector('[data-testid="canvas-um-space"]') as SVGGElement;
    const matrix=space.getScreenCTM()!,inverse=matrix.inverse(),spots=[...space.querySelectorAll('[data-testid="spot"]')].map(el=>el.getBoundingClientRect());
    const result:{x:number;y:number}[]=[];
    for(let row=0;row<18;row++)for(let col=0;col<12;col++) {
      const x=rect.left+rect.width*(.12+col*.045),y=rect.top+rect.height*(.26+row*.03),point=new DOMPoint(x,y).matrixTransform(inverse);
      if(point.x<10||point.y<10||point.x>1590||point.y>2390||!document.elementFromPoint(x,y)?.closest('[data-view]'))continue;
      if(spots.some(b=>x>b.left-12&&x<b.right+12&&y>b.top-12&&y<b.bottom+12))continue;
      if(result.some(p=>Math.hypot(x-p.x,y-p.y)<20))continue;
      result.push({x,y});if(result.length===count)return result;
    }
    throw new Error(`Only ${result.length} unobstructed blank positions`);
  },count);
}

// All original fields survive; additions must be canonical values for this exact id.
function expectCanonicalRecords(before: any[], after: any[], canonical: any[], additions: string[]) {
  expect(after).toHaveLength(before.length);
  expect(after.map(row => row.id)).toEqual(before.map(row => row.id));
  const rows = new Map(after.map(row => [row.id, row]));
  const server = new Map(canonical.map(row => [row.id, row]));
  expect(rows.size).toBe(before.length);
  for (const old of before) {
    const row = rows.get(old.id), accepted = server.get(old.id);
    expect(row, old.id).toBeDefined();expect(accepted, old.id).toBeDefined();
    for (const key of Object.keys(old)) {
      expect(Object.hasOwn(row, key), `${old.id}.${key} retained`).toBe(true);
      expect(row[key], `${old.id}.${key} unchanged`).toEqual(old[key]);
    }
    for (const key of Object.keys(row).filter(key => !Object.hasOwn(old, key))) {
      expect(additions, `${old.id}.${key} allowed`).toContain(key);
      expect(Object.hasOwn(accepted, key), `${old.id}.${key} canonical`).toBe(true);
      expect(row[key], `${old.id}.${key} canonical value`).toEqual(accepted[key]);
    }
  }
}

test('Spot: idle, delayed sync and registration burst on 150 grains/500 registered spots',async({page})=>{
  test.skip(!process.env.PERF,'Set PERF=1 to collect browser frame measurements');
  const template=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  const source=syncFixture(template),bodies:string[]=[];
  const unexpected=await mockImageServer(page,{saved:JSON.stringify(source),syncOnSave:false,sync:{delayMs:2500,bodies}});
  await page.goto(projectURL);await page.getByTestId('edit-copy').click();await expect(page.getByTestId('spot')).toHaveCount(500);
  await page.getByTestId('tool-spot').click();
  for(const dismiss of await page.getByRole('button',{name:'Dismiss',exact:true}).all())await dismiss.click();
  const before=JSON.parse(await saveImageProject(page)).grainatlas_ui;
  const registered=(spots:any[])=>spots.filter(s=>s.name.startsWith('registered-'));
  const registeredBefore=registered(before.spots);
  expect(before.spots).toHaveLength(500);expect(registeredBefore).toHaveLength(500);
  await startCapture(page);const idleMoves=await moves(page),idleTrace=await stopCapture(page);
  const [first]=await blankPoints(page,1);await page.keyboard.down('Alt');await page.mouse.click(first.x,first.y);await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(501);
  await startCapture(page);await page.getByTestId('sync-action').click();await expect(page.getByTestId('sync-chip')).toHaveAttribute('aria-busy','true');
  const syncMoves=await moves(page);
  await expect(page.getByTestId('sync-chip')).toHaveAttribute('aria-busy','false');
  const syncTrace=await stopCapture(page);expect(syncMoves.busyBursts).toBeGreaterThan(10);expect(bodies).toHaveLength(1);
  const points=await blankPoints(page,20);
  await startCapture(page);await page.evaluate(()=>{(window as any).__grainRegistering=true;});const registrationMoves=moves(page);
  for(let i=0;i<20;i++) {
    await page.keyboard.down('Alt');await page.mouse.click(points[i].x,points[i].y);await page.keyboard.up('Alt');
    await expect(page.getByTestId('spot')).toHaveCount(502+i);
    await page.getByTestId('panel-name').fill(`synthetic-new-${i}`);await page.getByTestId('panel-name').press('Tab');
  }
  await page.evaluate(()=>{(window as any).__grainRegistering=false;});const registeredMoves=await registrationMoves,registrationTrace=await stopCapture(page);
  expect(registeredMoves.registrationBursts).toBeGreaterThan(0);
  const after=JSON.parse(await saveImageProject(page)).grainatlas_ui;
  // Canonical metadata enrichment is the existing 0.1.12 acknowledgement behavior.
  const registeredAfter=registered(after.spots);
  const canonical=mockImageElements(source);
  expect(registeredAfter).toHaveLength(500);
  expectCanonicalRecords(registeredBefore,registeredAfter,canonical.spots,['acquisition_ids','project_uuid']);
  const canonicalSpots=new Map(canonical.spots.map((spot:any)=>[spot.id,spot]));
  for(const spot of registeredAfter) {
    const server=canonicalSpots.get(spot.id) as any;
    expect(spot.acquisition_ids).toEqual(server.acquisition_ids);expect(spot.project_uuid).toBe(server.project_uuid);
  }
  expect(after.spots).toHaveLength(521);
  expect(Object.keys(after.grains)).toEqual(Object.keys(before.grains));
  expectCanonicalRecords(Object.values(before.grains).flat(),Object.values(after.grains).flat(),canonical.grains,['project_uuid','retired','merged_from']);
  for(const [area,grains] of Object.entries(before.grains))expect(after.grains[area].map((g:any)=>g.id)).toEqual((grains as any[]).map(g=>g.id));
  expect(after.spots.filter((s:any)=>s.name.startsWith('synthetic-new-'))).toHaveLength(20);expect(unexpected).toEqual([]);
  const run1Idle=await readFile(await outputPath('pointer-spot'),'utf8').then(JSON.parse).catch(()=>null);
  // Save measurements before performance assertions so supervisor failures retain all evidence.
  await report('pointer-sync-registration',{grains:150,spots:500,moves:120,run1Idle,scenarios:{idle:{moves:idleMoves,trace:idleTrace},sync:{moves:syncMoves,trace:syncTrace},registration:{moves:registeredMoves,trace:registrationTrace}},
    method:'Same four-move/30-burst trace as run 1. Sync captures through acknowledgement after a 2500 ms mocked POST. Registration uses real UI placement and rename of 20 new spots. OS event delivery excluded.',
    budget:'p90 handler <8 ms, frame <34 ms, two-frame settle <50 ms; max long task <75 ms, sum(duration-50 ms) <50 ms per phase.'});
  console.log(JSON.stringify({idle:budgets(idleMoves,idleTrace),sync:budgets(syncMoves,syncTrace),registration:budgets(registeredMoves,registrationTrace)}));
});
