// Synthetic pixels/geometry only. Supervisor runs this spec; no server is started by this task.
import { test, expect } from './mock-session';
import type { Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type { Grain, Spot } from '../src/lib/core/types';
import { root, generated, mockImageServer, projectURL, saveImageProject } from './support/image-edit';
test.beforeAll(()=>{execFileSync('cargo',['run','-p','grainatlas-core','--example','print_edit_bundle'],{cwd:root,stdio:'pipe'});});
type Saved={grainatlas_ui:{grains:Record<string,Grain[]>;spots:Spot[]}};
const saved=async(page:Page):Promise<Saved>=>JSON.parse(await saveImageProject(page));
async function screen(page:Page,points:number[][]) {
  return page.getByTestId('canvas-um-space').evaluate((g:SVGGElement,points)=>{
    const m=g.getScreenCTM();if(!m)throw new Error('No micrometre transform');
    return points.map(([x,y])=>{const p=new DOMPoint(x,y).matrixTransform(m);return {x:p.x,y:p.y};});
  },points);
}
async function clickAt(page:Page,point:number[]) {const [p]=await screen(page,[point]);await page.mouse.click(p.x,p.y);}
async function drag(page:Page,points:number[][]) {
  const p=await screen(page,points);await page.mouse.move(p[0].x,p[0].y);await page.mouse.down();
  for(const point of p.slice(1))await page.mouse.move(point.x,point.y,{steps:4});
  await page.mouse.up();
}
async function prepare(page:Page,pixels=true) {
  const source=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  const grain:Grain=source.grainatlas_ui.grains.a[12];
  source.settings.naming_pattern='{sample}_{n:03}';source.settings.area_patterns={};
  source.grainatlas_ui.spots=[{id:'50000000-0000-4000-8000-000000000001',name:'registered-keep-name',area_id:'a',grain_id:grain.id,grain_number:grain.number,sample:'synthetic',n:1,x_um:grain.centroid_um[0],y_um:grain.centroid_um[1],x2_um:null,y2_um:null,diameter_um:10,kind:'spot',acquisition_id:'60000000-0000-4000-8000-000000000001',comment:'keep every field',created:'',created_by:''}];
  const unexpected=await mockImageServer(page,{saved:JSON.stringify(source),syncOnSave:false});
  await page.goto(projectURL);await page.getByTestId('edit-copy').click();await expect(page.getByTestId('busy')).toHaveCount(0);
  if(pixels){
    const nav=page.getByRole('navigation',{name:'Screens'});
    await nav.getByRole('button',{name:'Area',exact:true}).click();await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
    await nav.getByRole('button',{name:'Canvas',exact:true}).click();await expect(page.getByTestId('busy')).toHaveCount(0);
  }
  for(const dismiss of await page.getByRole('button',{name:'Dismiss',exact:true}).all())await dismiss.click();
  await page.getByRole('tab',{name:/^Grains /}).click();
  await page.locator(`.side tr[data-grain-id="${grain.id}"]`).click();
  const close=page.getByRole('button',{name:'Close',exact:true});if(await close.count())await close.click();
  await page.getByTestId('side-collapse').click();
  const before=await saved(page), number=Math.max(...before.grainatlas_ui.grains.a.map(g=>g.number))+1;
  const sample=source.areas[0].sample||source.areas[0].name;
  await page.keyboard.press('g');await expect(page.getByTestId('tool-add-grain')).toHaveAttribute('aria-pressed','true');
  await expect(page.getByTestId(pixels?'add-mode-click':'add-mode-draw')).toHaveAttribute('aria-checked','true');
  await expect(page.getByTestId('add-grain-name')).toContainText(`${sample}_${String(number).padStart(3,'0')}`);
  return {grain,before,number,name:`${sample}_${String(number).padStart(3,'0')}`,unexpected};
}
function polygon(g:Grain):number[][] {const [x0,y0,x1,y1]=g.bbox_um;return [[x0-10,y0-10],[x1+10,y0-10],[x1+10,y1+10],[x0-10,y1+10]];}
async function preview(page:Page,g:Grain,mode:string) {
  await page.getByTestId(`add-mode-${mode==='polygon'||mode==='freehand'?'draw':mode}`).click();
  if(mode==='click')await clickAt(page,g.centroid_um);
  else if(mode==='box'){const p=polygon(g);await drag(page,[p[0],p[2]]);}
  else if(mode==='polygon'){
    for(const p of polygon(g))await clickAt(page,p);
    await page.keyboard.press('Enter'); // Close the polygon into a preview; next Enter accepts.
  } else {
    const p=polygon(g), mid=(a:number[],b:number[])=>[(a[0]+b[0])/2,(a[1]+b[1])/2];
    await drag(page,[p[0],mid(p[0],p[1]),p[1],mid(p[1],p[2]),p[2],mid(p[2],p[3]),p[3],mid(p[3],p[0])]);
  }
  await expect(page.getByTestId('add-grain-preview')).toHaveCount(1);
  await expect(page.getByTestId('add-grain-accept')).toBeEnabled();
}
async function adjust(page:Page) {
  // Use a genuine corner press; make the edit before accepting the new grain.
  const vertex=page.getByTestId('add-grain-vertex').first();
  const p=await vertex.evaluate((el:SVGCircleElement)=>[el.cx.baseVal.value,el.cy.baseVal.value]);
  await page.keyboard.press('e'); // E keeps the Add preview and makes its editing affordance explicit.
  await drag(page,[p,[p[0]-3,p[1]-3]]);
  const after=await vertex.evaluate((el:SVGCircleElement)=>[el.cx.baseVal.value,el.cy.baseVal.value]);
  expect(after).toEqual([p[0]-3,p[1]-3]);
}
for(const viewport of [{width:1280,height:800},{width:1440,height:900}]) {
  test(`Add grain short-path recovery and keyboard review retain the draft at ${viewport.width}`,async({page})=>{
    await page.setViewportSize(viewport);const {grain,before,unexpected}=await prepare(page,false);
    const points=polygon(grain);
    await clickAt(page,points[0]);await clickAt(page,points[1]);await page.keyboard.press('Enter');
    await expect(page.getByRole('status').filter({hasText:'Click another corner, then Enter to finish.'})).toBeVisible();
    const lasso=page.getByTestId('lasso');await expect(lasso).toHaveCount(1);
    const corners=await lasso.getAttribute('points');
    expect(new Set(corners?.split(' ')).size).toBe(2);
    await page.getByTestId('review-issues').focus();await page.keyboard.press('Enter');
    await expect(page.getByTestId('sync-details')).toBeFocused();
    await page.keyboard.press('Escape');await expect(page.getByTestId('review-issues')).toBeFocused();
    await expect(page.getByTestId('tool-add-grain')).toHaveAttribute('aria-pressed','true');
    // Enter/Escape in Review must never accept or cancel the active creation path.
    expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
    await expect(lasso).toHaveAttribute('points',corners!);
    await page.getByTestId('review-issues').click();
    await page.getByTestId('sync-details').getByRole('button',{name:'Back to canvas',exact:true}).click();
    await expect(page.getByRole('application',{name:'Mosaic canvas'})).toBeFocused();
    await clickAt(page,points[2]);await clickAt(page,points[3]);await page.keyboard.press('Enter');
    await expect(page.getByTestId('add-grain-preview')).toBeVisible();
    // A ready preview also survives keyboard review without an implicit Accept.
    await page.getByTestId('review-issues').focus();await page.keyboard.press('Enter');
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('add-grain-preview')).toBeVisible();
    expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
    await page.getByTestId('add-grain-cancel').click();
    await expect(page.getByTestId('add-grain-preview')).toHaveCount(0);
    await expect(page.getByTestId('canvas-toolbar').getByRole('button',{name:'Undo',exact:true})).toBeDisabled();
    expect(unexpected).toEqual([]);
  });
  for(const mode of ['click','polygon','freehand','box']) {
    test(`Add grain ${mode}: preview, adjust, accept and exact Undo at ${viewport.width} x ${viewport.height}`,async({page})=>{
      await page.setViewportSize(viewport);const {grain,before,number,name,unexpected}=await prepare(page);
      const toolbar=page.getByTestId('canvas-toolbar');expect(await toolbar.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
      for(const control of await toolbar.locator('button').all())await expect(control).toBeInViewport({ratio:1});
      await preview(page,grain,mode);
      if(mode==='click') {
        const old=await page.getByTestId('add-grain-preview').getAttribute('data-revision');
        await page.getByRole('slider',{name:'Grain sensitivity'}).focus();await page.keyboard.press('ArrowRight');
        await expect(page.getByTestId('add-grain-preview')).toHaveCount(1);
        await expect(page.getByTestId('add-grain-preview')).not.toHaveAttribute('data-revision',old!);
        await page.keyboard.press('Tab');
      }
      await expect(page.getByTestId('add-grain-overlap')).toContainText(grain.label!);
      expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
      await adjust(page);await page.keyboard.press('Enter');
      await expect(page.getByTestId('add-grain-preview')).toHaveCount(0);
      const after=await saved(page), added=after.grainatlas_ui.grains.a.filter(g=>!before.grainatlas_ui.grains.a.some(b=>b.id===g.id));
      expect(added).toHaveLength(1);expect(added[0]).toMatchObject({number,label:name});
      expect(after.grainatlas_ui.grains.a.filter(g=>g.id!==added[0].id)).toEqual(before.grainatlas_ui.grains.a);
      expect(after.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
      await expect(page.getByTestId('tool-add-grain')).toHaveAttribute('aria-pressed','true');
      await toolbar.getByRole('button',{name:'Undo',exact:true}).click();
      expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
      await expect(toolbar.getByRole('button',{name:'Undo',exact:true})).toBeDisabled();
      // Each mode also exercises Esc cancellation after an adjusted preview.
      await preview(page,grain,mode);await adjust(page);await page.keyboard.press('Escape');
      await expect(page.getByTestId('add-grain-preview')).toHaveCount(0);
      expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
      await expect(toolbar.getByRole('button',{name:'Undo',exact:true})).toBeDisabled();
      expect(unexpected).toEqual([]);
    });
  }
  test(`Add grain then one spot returns to Add grain at ${viewport.width} x ${viewport.height}`,async({page})=>{
    await page.setViewportSize(viewport);const {grain,before,unexpected}=await prepare(page);
    await page.getByTestId('add-then-spot').check();
    expect(await page.evaluate(()=>localStorage.getItem('grainatlas.add-grain-spot'))).toBe('true');
    await preview(page,grain,'polygon');
    await page.getByTestId('add-grain-accept').click();
    await expect(page.getByTestId('tool-spot')).toHaveAttribute('aria-checked','true');
    await expect(page.getByTestId('add-grain-hint')).toContainText('inside the new grain');
    const accepted=await saved(page),added=accepted.grainatlas_ui.grains.a.at(-1)!;
    // It overlaps a registered grain: spot must attach to the newly accepted grain.
    await clickAt(page,grain.centroid_um);
    await expect(page.getByTestId('tool-add-grain')).toHaveAttribute('aria-pressed','true');
    const after=await saved(page);expect(after.grainatlas_ui.spots).toHaveLength(before.grainatlas_ui.spots.length+1);
    expect(after.grainatlas_ui.spots.slice(0,-1)).toEqual(before.grainatlas_ui.spots);
    expect(after.grainatlas_ui.spots.at(-1)).toMatchObject({grain_id:added.id,grain_number:added.number});
    expect(after.grainatlas_ui.grains.a.slice(0,-1)).toEqual(before.grainatlas_ui.grains.a);
    // Spot and grain are independent normal history entries.
    await page.keyboard.press('ControlOrMeta+z');expect((await saved(page)).grainatlas_ui).toEqual(accepted.grainatlas_ui);
    await page.keyboard.press('ControlOrMeta+z');expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
    await expect(page.getByTestId('add-then-spot')).toBeChecked();
    await preview(page,grain,'polygon');await page.getByTestId('add-grain-accept').click();await page.keyboard.press('Escape');
    await expect(page.getByTestId('tool-add-grain')).toHaveAttribute('aria-pressed','true');
    await page.getByTestId('add-then-spot').uncheck();
    // Continue adding multiple grains without returning to the toolbar.
    await preview(page,grain,'polygon');await page.keyboard.press('Enter');await preview(page,grain,'polygon');await page.keyboard.press('Enter');
    expect((await saved(page)).grainatlas_ui.grains.a).toHaveLength(before.grainatlas_ui.grains.a.length+3);
    expect(unexpected).toEqual([]);
  });
  test(`remote geometry-only Add grain falls back to Draw at ${viewport.width} x ${viewport.height}`,async({page})=>{
    await page.setViewportSize(viewport);const {grain,before,unexpected}=await prepare(page,false);
    await expect(page.getByTestId('add-grain-pixels-hint')).toHaveText('Image pixels are unavailable. Use Draw; Click and Box need pixels.');
    for (const mode of ['click','box']) {
      await expect(page.getByTestId(`add-mode-${mode}`)).toBeDisabled();
      await expect(page.getByTestId(`add-mode-${mode}`)).toHaveAttribute('title',`${mode === 'click' ? 'Click' : 'Box'} needs image pixels. Use Draw.`);
    }
    const draw=page.getByTestId('add-mode-draw');await draw.focus();
    for(const key of ['ArrowLeft','ArrowRight','Home','End']) {
      await page.keyboard.press(key);await expect(draw).toHaveAttribute('aria-checked','true');await expect(draw).toBeFocused();
    }
    await preview(page,grain,'polygon');await adjust(page);await page.keyboard.press('Enter');
    expect((await saved(page)).grainatlas_ui.grains.a).toHaveLength(before.grainatlas_ui.grains.a.length+1);
    await page.keyboard.press('ControlOrMeta+z');expect((await saved(page)).grainatlas_ui).toEqual(before.grainatlas_ui);
    expect(unexpected).toEqual([]);
  });
}
