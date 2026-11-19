// Invented bundle only. The supervisor runs this spec with a preview server.
import { test, expect } from './mock-session';
import type { Page } from '@playwright/test';
import type { Grain, Spot } from '../src/lib/core/types';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { root, generated, mockImageServer, projectURL, saveImageProject, type SavedFixture } from './support/image-edit';

test.use({ viewport:{width:1280,height:800} });
test.beforeAll(()=>{execFileSync('cargo',['run','-p','grainatlas-core','--example','print_edit_bundle'],{cwd:root,stdio:'pipe'});});
test.beforeEach(({page})=>{
  page.on('console',message=>{
    if(message.type()==='error') console.error(`[image-edit console] ${message.text()}`,message.location());
  });
  page.on('pageerror',error=>console.error(`[image-edit pageerror] ${error.stack??error.message}`));
});
async function open(page:Page,fixture:SavedFixture={syncOnSave:false}) {
  const unexpected=await mockImageServer(page,fixture);
  await page.goto(projectURL);await expect(page.getByTestId('edit-copy')).toHaveText('Edit');await page.getByTestId('edit-copy').click();
  await page.getByTestId('edit-image').click();return unexpected;
}
async function screen(page:Page,points:number[][]) {
  return page.getByTestId('canvas-um-space').evaluate((g:SVGGElement,pts)=>{
    const m=g.getScreenCTM();if(!m)throw Error('No mount transform');
    return pts.map(([x,y])=>{const p=new DOMPoint(x,y).matrixTransform(m);return {x:p.x,y:p.y};});
  },points);
}
async function drag(page:Page,points:number[][],modifiers:string[]=[]) {
  const [a,b]=await screen(page,points);
  for(const key of modifiers)await page.keyboard.down(key);
  await page.mouse.move(a.x,a.y);await page.mouse.down();await page.mouse.move(b.x,b.y,{steps:12});await page.mouse.up();
  for(const key of modifiers.reverse())await page.keyboard.up(key);
}
async function print(page:Page) {
  await page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Layout',exact:true}).click();
  await page.getByTestId('sheet-packing').selectOption('packed');await page.getByTestId('sheet-scale').fill('4');
  await page.getByLabel('denoise (NLM)').uncheck();await page.getByLabel('sharpen',{exact:true}).uncheck();
  await page.getByTestId('sheets').click();await expect(page.getByTestId('sheet-stats')).toBeVisible();
}
async function canvas(page:Page) {
  await page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Canvas',exact:true}).click();
  await page.getByTestId('edit-image').click();
}
function registeredGrains(grains:Record<string,Grain[]>) {
  return Object.fromEntries(Object.entries(grains).map(([area,list])=>[area,list.map(g=>({
    id:g.id,number:g.number,label:g.label,polygon_um:g.polygon_um,centroid_um:g.centroid_um,
    bbox_um:g.bbox_um,area_um2:g.area_um2,artifact:g.artifact,
  }))]));
}
function registeredSpots(spots:(Spot&{acquisition_ids?:string[]})[]) {
  return spots.map(s=>({id:s.id,name:s.name,area_id:s.area_id,grain_id:s.grain_id,
    x_um:s.x_um,y_um:s.y_um,x2_um:s.x2_um,y2_um:s.y2_um,kind:s.kind,diameter_um:s.diameter_um,
    acquisition_id:s.acquisition_id,acquisition_ids:s.acquisition_ids,
  }));
}
async function expectEditorFits(page:Page) {
  const controls=page.getByRole('complementary',{name:'Edit image controls'});
  await expect(controls).toBeInViewport({ratio:1});
  await expect(controls.getByTestId('image-edit-footer')).toBeInViewport({ratio:1});
  for(const name of ['Undo','Redo','Reset','Done']) {
    await expect(controls.getByRole('button',{name,exact:true})).toBeInViewport({ratio:1});
  }
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.documentElement.scrollHeight<=innerHeight&&scrollY===0)).toBe(true);
  expect(await controls.evaluate(e=>e.scrollHeight<=e.clientHeight+2)).toBe(true);
}
async function screenshots(page:Page,state:'crop'|'rotated-spot'|'mask') {
  const initial=page.viewportSize()!;
  const output=path.join(root,'target/e2e-results');
  await mkdir(output,{recursive:true});
  for(const {width,height} of [{width:1440,height:900},{width:1280,height:800}]) {
    await page.setViewportSize({width,height});
    await expectEditorFits(page);
    if(state==='crop') {
      await expect(page.locator('rect[data-image-handle]')).toHaveCount(8);
      await expect(page.getByTestId('image-crop-outside')).toBeVisible();
    } else if(state==='mask') {
      await expect(page.getByTestId('image-mask-stroke')).toBeVisible();
      await expect(page.getByTestId('image-mask-hatch')).toBeVisible();
      expect((await page.getByTestId('image-mask-stroke').getAttribute('d'))!.split('L').length).toBeGreaterThan(2);
      await expect(page.getByRole('button',{name:'Paint out',exact:true})).toHaveAttribute('aria-pressed','true');
    } else await expect(page.getByTestId('spot')).toHaveCount(1);
    await page.screenshot({path:path.join(output,`image-edit-${state}-${width}x${height}.png`),fullPage:false});
  }
  await page.setViewportSize(initial);
  await expectEditorFits(page);
}

test('crop changes print selection; reset restores grains and registered spots survive',async({page})=>{
  const original=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  const g=original.grainatlas_ui.grains.a[0];
  original.grainatlas_ui.spots=[{id:'50000000-0000-4000-8000-000000000001',area_id:'a',grain_id:g.id,name:'A_1',sample:'A1',grain_number:1,n:1,x_um:g.centroid_um[0],y_um:g.centroid_um[1],diameter_um:15,kind:'spot',comment:'',created:'',created_by:''}];
  const unexpected=await open(page,{saved:JSON.stringify(original),syncOnSave:false});
  const before=JSON.parse(await saveImageProject(page));
  await page.getByRole('button',{name:'Rectangle',exact:true}).click();
  // Crop away the first cluster, keep the lower two; Windows modifiers do not place a spot.
  await drag(page,[[100,800],[1300,2300]],['Control','Shift']);
  await expect(page.getByTestId('image-crop-count')).toHaveText('10 grains left off by the crop');
  await expect(page.getByRole('status').filter({hasText:'stays registered'})).toHaveText('1 of these grains has a registered spot, which stays registered.');
  await screenshots(page,'crop');
  await print(page);await expect(page.getByTestId('sheet-stats')).toContainText('20 grains');await expect(page.getByTestId('sheet-stats')).toContainText('10 grains left off by crop');
  await canvas(page);await page.getByRole('button',{name:'Reset',exact:true}).click();await expect(page.getByTestId('image-crop-count')).toHaveText('0 grains left off by the crop');
  await print(page);await expect(page.getByTestId('sheet-stats')).toContainText('30 grains');
  const saved=JSON.parse(await saveImageProject(page));expect(saved.grainatlas_ui.grains).toEqual(before.grainatlas_ui.grains);expect(saved.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);expect(unexpected).toEqual([]);
});

test('rotation and flip inverse place a spot in mount µm, save and reopen retain the recipe',async({page})=>{
  const fixture:SavedFixture={syncOnSave:false};const unexpected=await open(page,fixture);
  const before=JSON.parse(await saveImageProject(page));
  await page.getByRole('button',{name:'90° right',exact:true}).click();await page.getByRole('button',{name:'Flip horizontal',exact:true}).click();
  await page.getByLabel('Black point',{exact:true}).fill('18');await page.getByLabel('White point',{exact:true}).fill('236');await page.getByLabel('White point',{exact:true}).press('Tab');
  const source=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  // A central grain remains clear of the toolbar and side panel after fitting the orientation.
  const p=source.grainatlas_ui.grains.a[12].centroid_um;
  await page.getByRole('button',{name:'Done',exact:true}).click();await page.getByTestId('tool-spot').click();
  await expect(page.getByRole('complementary',{name:'Edit image controls'})).toHaveCount(0);
  await expect(page.getByTestId('edit-image')).toHaveAttribute('aria-pressed','false');
  const [at]=await screen(page,[p]);await page.mouse.click(at.x,at.y);
  await page.getByTestId('edit-image').click();
  await screenshots(page,'rotated-spot');
  const saved=JSON.parse(await saveImageProject(page));
  expect(saved.image_edits.areas.a).toMatchObject({rotate_deg:90,flip_h:true,levels:{black:18,white:236}});
  const spot=saved.grainatlas_ui.spots.at(-1);expect(spot.x_um).toBeCloseTo(p[0],2);expect(spot.y_um).toBeCloseTo(p[1],2);
  expect(saved.grainatlas_ui.grains).toEqual(before.grainatlas_ui.grains);
  // Startup consumes the published-project query; a bare reload returns to Open.
  // Reopen that link and choose Edit to resume the local draft. The server still
  // serves the original project, so neither the recipe nor the spot can come from it.
  await page.reload();
  await expect(page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Open',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(page.getByRole('button',{name:'Open local',exact:true})).toBeVisible();
  await page.goto(projectURL);await expect(page.getByTestId('edit-copy')).toHaveText('Edit');await page.getByTestId('edit-copy').click();await page.getByTestId('edit-image').click();
  await expect(page.getByRole('button',{name:'Flip horizontal',exact:true})).toHaveAttribute('aria-pressed','true');await expect(page.getByLabel('Black point',{exact:true})).toHaveValue('18');await expect(page.getByLabel('White point',{exact:true})).toHaveValue('236');
  const reopened=JSON.parse(await saveImageProject(page));
  expect(reopened.image_edits).toEqual(saved.image_edits);
  expect(registeredSpots(reopened.grainatlas_ui.spots)).toEqual(registeredSpots(saved.grainatlas_ui.spots));
  // origin/main fills sample from the area and defaults merged_from in pending
  // sync records. Compare the identity, geometry and links edits must preserve.
  expect(registeredGrains(reopened.grainatlas_ui.grains)).toEqual(registeredGrains(saved.grainatlas_ui.grains));
  expect(unexpected).toEqual([]);
});

test('polygon, mask, erase and Ctrl undo fit laptop windows',async({page})=>{
  await open(page);
  for(const size of [{width:1280,height:800},{width:1440,height:900}]) {
    await page.setViewportSize(size);
    await expectEditorFits(page);
  }
  await page.getByRole('button',{name:'Polygon',exact:true}).click();
  for(const p of [[100,800],[1300,800],[1300,2300],[100,2300]]) {const [s]=await screen(page,[p]);await page.mouse.click(s.x,s.y);}
  await page.getByRole('button',{name:'Finish polygon',exact:true}).click();await expect(page.getByTestId('image-crop-count')).toHaveText('10 grains left off by the crop');
  await page.getByRole('button',{name:'Paint out',exact:true}).click();await page.getByLabel('Mask brush radius').fill('80');
  await drag(page,[[160,900],[210,2150]],['Alt']);
  await expect(page.getByRole('complementary',{name:'Edit image controls'})).toContainText('Mask · 1 stroke');
  await screenshots(page,'mask');
  await page.keyboard.press('Control+z');await expect(page.getByRole('complementary',{name:'Edit image controls'})).not.toContainText('Mask ·');
  await page.keyboard.press('Control+Shift+z');await page.getByRole('button',{name:'Erase mask',exact:true}).click();await drag(page,[[160,900],[210,2150]]);
  await expect(page.getByRole('complementary',{name:'Edit image controls'})).not.toContainText('Mask ·');
});
