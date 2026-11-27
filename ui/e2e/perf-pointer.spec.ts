// PERF_LABEL=after pnpm exec playwright test -c playwright.perf.config.ts e2e/perf-pointer.spec.ts
// Uses only invented geometry over the generated synthetic bundle. No network timing claim.
import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { root, generated, mockImageServer, projectURL, saveImageProject } from './support/image-edit';
import { distribution, report } from './support/perf';
import type { Grain, Spot } from '../src/lib/core/types';
test.beforeAll(()=>{execFileSync('cargo',['run','-p','grainatlas-core','--example','print_edit_bundle'],{cwd:root,stdio:'pipe'});});
for(const tool of ['spot','select'] as const) test(`${tool}: 120 pointer moves on 300 grains and 500 registered spots`,async({page})=>{
  const project=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  const template:Grain=project.grainatlas_ui.grains.a[0];
  // Reuse the invented area's extent, replacing all test geometry within that extent.
  const old:Grain[]=project.grainatlas_ui.grains.a;
  const left=Math.min(...old.map(g=>g.bbox_um[0])),top=Math.min(...old.map(g=>g.bbox_um[1]));
  const w=Math.max(...old.map(g=>g.bbox_um[2]))-left,h=Math.max(...old.map(g=>g.bbox_um[3]))-top;
  const grains:Grain[]=Array.from({length:300},(_,i)=>{
    const x=left+(i%20+.5)*w/20,y=top+(Math.floor(i/20)+.5)*h/15,rx=w/55,ry=h/42;
    return {...template,id:`70000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`,number:i+1,label:`synthetic/${i+1}`,centroid_um:[x,y],bbox_um:[x-rx,y-ry,x+rx,y+ry],polygon_um:Array.from({length:64},(_,j)=>[x+rx*Math.cos(j*Math.PI/32),y+ry*Math.sin(j*Math.PI/32)]),major_um:rx*2,minor_um:ry*2,orientation_deg:i%180};
  });
  const spots:Spot[]=Array.from({length:500},(_,i)=>{
    const g=grains[i%300];return {id:`80000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`,area_id:'a',name:`registered-${i}`,sample:'synthetic',grain_id:g.id,grain_number:g.number,n:i+1,x_um:g.centroid_um[0],y_um:g.centroid_um[1],x2_um:null,y2_um:null,diameter_um:8,kind:'spot',comment:'keep',created:'',created_by:''};
  });
  project.grainatlas_ui.grains.a=grains;project.grainatlas_ui.spots=spots;
  const unexpected=await mockImageServer(page,{saved:JSON.stringify(project),syncOnSave:false});
  await page.goto(projectURL);await page.getByTestId('edit-copy').click();await expect(page.getByTestId('spot')).toHaveCount(500);
  await page.getByTestId(`tool-${tool}`).click();
  for(const dismiss of await page.getByRole('button',{name:'Dismiss',exact:true}).all())await dismiss.click();
  // Leave the side table and auto contrast enabled, matching the reported slow scenario.
  const before=JSON.parse(await saveImageProject(page)).grainatlas_ui;
  const measurements=await page.getByRole('application',{name:'Mosaic canvas'}).evaluate(async(host,tool)=>{
    const rect=host.getBoundingClientRect(),handlerMs:number[]=[],frameMs:number[]=[],settleMs:number[]=[],sideMutations:number[]=[];
    let frame=0,lastFrame:number|null=null;
    const sampleFrame=(now:number)=>{if(lastFrame!==null)frameMs.push(now-lastFrame);lastFrame=now;frame=requestAnimationFrame(sampleFrame);};
    frame=requestAnimationFrame(sampleFrame);
    const side=document.querySelector('.side .list')!;
    let mutations=0;
    const observer=new MutationObserver(records=>{mutations+=records.length;});
    observer.observe(side,{attributes:true,childList:true,characterData:true,subtree:true});
    // Four raw moves in each animation-frame burst exercise 240 Hz input coalescing.
    // Two further frames include Svelte and deferred contrast before the next burst.
    const x0=rect.left+rect.width*.32,y0=rect.top+rect.height*.40;
    for(let burst=0;burst<30;burst++) {
      await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
      const burstStart=performance.now();
      for(let j=0;j<4;j++) {
        const i=burst*4+j,x=x0+Math.sin(i*.37)*90,y=y0+Math.cos(i*.29)*70;
        const target=document.elementFromPoint(x,y);
        if(!target?.closest('[data-view]'))throw new Error('Performance pointer covered by UI');
        const start=performance.now();
        target.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:x,clientY:y,pointerId:1,pointerType:'mouse',altKey:i%11===0}));
        handlerMs.push(performance.now()-start);
      }
      await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
      await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
      settleMs.push(performance.now()-burstStart);sideMutations.push(mutations);mutations=0;
    }
    observer.disconnect();cancelAnimationFrame(frame);return {tool,handlerMs,frameMs,settleMs,sideMutations};
  },tool);
  const handler=distribution(measurements.handlerMs),frames=distribution(measurements.frameMs),settle=distribution(measurements.settleMs);
  await report(`pointer-${tool}`,{grains:300,spots:500,moves:120,movesPerBurst:4,bursts:30,viewport:page.viewportSize(),handler,frames,settle,measurements,
    method:'Synthetic DOM pointermove dispatch to visible canvas, four raw moves per animation-frame burst (30 bursts); handler elapsed, continuous animation-frame intervals, and two-frame settle latency. Includes queued Svelte/preview/deferred contrast, excludes OS input delivery.',
    budget:'p90 handler <8 ms leaves half a 60 Hz frame for paint/input; p90 frame interval <34 ms permits one missed frame; p90 two-frame settle <50 ms permits one extra scheduling frame.'});
  expect(handler.p90!).toBeLessThan(8);expect(frames.p90!).toBeLessThan(34);expect(settle.p90!).toBeLessThan(50);
  expect(measurements.sideMutations.reduce((sum,n)=>sum+n,0)).toBe(0);
  // No placement/drag occurred: the saved records and links must remain byte-for-byte equal.
  const after=JSON.parse(await saveImageProject(page)).grainatlas_ui;
  expect(after.grains).toEqual(before.grains);expect(after.spots).toEqual(before.spots);expect(unexpected).toEqual([]);
});
