// Synthetic published area and registered acquisition only; supervisor runs with preview server.
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
async function prepare(page:Page) {
  const source=JSON.parse(await readFile(path.join(generated,'project.json'),'utf8'));
  const grain:Grain=source.grainatlas_ui.grains.a[12];
  source.grainatlas_ui.spots=[{id:'50000000-0000-4000-8000-000000000001',name:'registered-keep-name',area_id:'a',grain_id:grain.id,grain_number:grain.number,sample:'synthetic',n:1,x_um:grain.centroid_um[0],y_um:grain.centroid_um[1],x2_um:null,y2_um:null,diameter_um:10,kind:'spot',acquisition_id:'60000000-0000-4000-8000-000000000001',comment:'keep',created:'',created_by:''}];
  const unexpected=await mockImageServer(page,{saved:JSON.stringify(source),syncOnSave:false});
  await page.goto(projectURL);await page.getByTestId('edit-copy').click();await expect(page.getByTestId('busy')).toHaveCount(0);
  for(const dismiss of await page.getByRole('button',{name:'Dismiss',exact:true}).all())await dismiss.click();
  await expect(page.getByTestId('tool-edit-outline')).toBeDisabled();
  await page.getByRole('tab',{name:/^Grains /}).click();
  await page.locator(`.side tr[data-grain-id="${grain.id}"]`).click();
  await expect(page.getByTestId('tool-edit-outline')).toBeEnabled();
  const before=await saved(page);
  return {grain:before.grainatlas_ui.grains.a.find(g=>g.id===grain.id)!,before,unexpected};
}
for(const viewport of [{width:1280,height:800},{width:1440,height:900}]) {
  test(`outline actions, vertex edit and exact undo at ${viewport.width} x ${viewport.height}`,async({page})=>{
    await page.setViewportSize(viewport);const {grain,before,unexpected}=await prepare(page);
    const toolbar=page.getByTestId('canvas-toolbar');
    expect(await toolbar.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
    for(const b of await toolbar.locator('button').all())await expect(b).toBeInViewport({ratio:1});
    // Main-row entry, then the header entry that survives a minimized Grain card.
    await page.getByTestId('tool-edit-outline').click();await expect(page.getByTestId('outline-vertex')).toHaveCount(grain.polygon_um.length);
    const minimize=page.getByRole('button',{name:'Minimize spot and grain info'});
    if(await minimize.count())await minimize.click();
    await expect(page.getByTestId('card-edit-outline-header')).toBeVisible();await page.getByTestId('card-edit-outline-header').click();
    await page.getByTestId('tool-select').click();await page.keyboard.press('e');
    await expect(page.getByTestId('tool-edit-outline')).toHaveAttribute('aria-pressed','true');
    await page.getByRole('button',{name:'Close',exact:true}).click();
    await page.getByTestId('side-collapse').click();
    const point=grain.polygon_um[0],end=[point[0]-5,point[1]-5];
    const [a,b]=await screen(page,[point,end]);
    expect(await page.evaluate(p=>!!document.elementFromPoint(p.x,p.y)?.closest('[data-view]'),a)).toBe(true);
    await page.mouse.move(a.x,a.y);await page.mouse.down();await page.mouse.move(b.x,b.y,{steps:12});await page.mouse.up();
    const after=await saved(page);
    expect(after.grainatlas_ui.grains.a.find(g=>g.id===grain.id)!.polygon_um).not.toEqual(grain.polygon_um);
    expect(after.grainatlas_ui.grains.a.filter(g=>g.id!==grain.id)).toEqual(before.grainatlas_ui.grains.a.filter(g=>g.id!==grain.id));
    expect(after.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
    await toolbar.getByRole('button',{name:'Undo',exact:true}).click();
    const restored=await saved(page);expect(restored.grainatlas_ui.grains).toEqual(before.grainatlas_ui.grains);expect(restored.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
    // Cancelled previews never touch persisted geometry, names or links.
    await page.mouse.move(a.x,a.y);await page.mouse.down();await page.mouse.move(b.x,b.y);await page.keyboard.press('Escape');await page.mouse.up();
    expect((await saved(page)).grainatlas_ui).toEqual(restored.grainatlas_ui);
    expect(unexpected).toEqual([]);
  });
}
test('edge addition and Alt corner removal preserve registered spots',async({page})=>{
  const {grain,before,unexpected}=await prepare(page);
  await page.getByTestId('tool-edit-outline').click();await page.getByRole('button',{name:'Close',exact:true}).click();await page.getByTestId('side-collapse').click();
  const a=grain.polygon_um[0],b=grain.polygon_um[1],middle=[(a[0]+b[0])/2,(a[1]+b[1])/2];
  const [point]=await screen(page,[middle]);await page.mouse.click(point.x,point.y);
  await expect(page.getByTestId('outline-vertex')).toHaveCount(grain.polygon_um.length+1);
  expect((await saved(page)).grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
  await page.keyboard.down('Alt');await page.mouse.click(point.x,point.y);await page.keyboard.up('Alt');
  await expect(page.getByTestId('outline-vertex')).toHaveCount(grain.polygon_um.length);
  expect((await saved(page)).grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
  await page.keyboard.press('ControlOrMeta+z');await page.keyboard.press('ControlOrMeta+z');
  const restored=await saved(page);expect(restored.grainatlas_ui.grains).toEqual(before.grainatlas_ui.grains);expect(restored.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
  expect(unexpected).toEqual([]);
});
