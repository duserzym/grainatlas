import type { Page } from '@playwright/test';
import { test, expect } from './mock-session';
import { mockServer, problems, server, projectId, mountId } from './print-fixture';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
test.beforeAll(()=>{execFileSync('cargo',['run','-p','grainatlas-core','--example','print_edit_bundle'],{cwd:root,stdio:'pipe'});});
async function ready(page:Page){await expect(page.getByTestId('export-pdf')).not.toHaveAttribute('title','Wait for the strip preview');await expect(page.getByTestId('busy')).toHaveCount(0);}
async function open(page:Page,fixture:{saved?:string;syncOnSave?:boolean}={}){
  const unexpected=await mockServer(page,fixture);
  await page.addInitScript(()=>Object.defineProperty(navigator,'platform',{get:()=> 'Win32'}));
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Layout',exact:true}).click();
  await page.getByTestId('sheet-scale').fill('4');await page.getByTestId('sheet-packing').selectOption('packed');
  await page.getByLabel('denoise (NLM)').uncheck();await page.getByLabel('sharpen',{exact:true}).uncheck();
  await page.getByTestId('sheets').click();await ready(page);await page.getByTestId('arrange-mode').click();await ready(page);
  await expect(page.locator('[data-piece]').first()).toHaveCount(1);
  return unexpected;
}
async function registration(page:Page){await ready(page);const next=page.waitForEvent('download');await page.getByTestId('export-registration').click();const file=await(await next).path();return JSON.parse(await readFile(file!,'utf8'));}
async function paper(page:Page){const r=await page.getByTestId('page').boundingBox();if(!r)throw Error('No paper');return {...r,px:r.width/431.8};}
async function selectFirst(page:Page){const b=page.getByTestId('strip-block').first();await b.click({position:{x:8,y:4}});await expect(page.getByLabel('Piece scale',{exact:true})).toBeVisible();}
async function dragFirst(page:Page,dx:number,dy:number){const r=await page.getByTestId('strip-block').first().boundingBox(),p=await paper(page);await page.keyboard.down('Alt');await page.mouse.move(r!.x+8,r!.y+4);await page.mouse.down();await page.mouse.move(r!.x+8+dx*p.px,r!.y+4+dy*p.px,{steps:8});await page.mouse.up();await page.keyboard.up('Alt');await ready(page);}
async function screenshot(page:Page,state:string,w:number,h:number){
  await mkdir(path.join(root,'target/e2e-results'),{recursive:true});
  await page.getByTestId('print-controls-scroll').evaluate(el=>{el.scrollTop=0;});
  await page.screenshot({path:path.join(root,`target/e2e-results/arrange-${state}-${w}x${h}.png`),animations:'disabled'});
}
async function toolbarWithNotice(page:Page){
  await expect(page.getByText(/^Editing mount A\. Spots, grains and labels/)).toBeVisible();
  const covered=await page.getByTestId('arrange-toolbar').locator('button,select').evaluateAll(controls=>controls.flatMap(el=>{
    const r=el.getBoundingClientRect();
    const hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
    return hit&&(el===hit||el.contains(hit))?[]:[el.textContent?.trim()||el.getAttribute('aria-label')];
  }));
  expect(covered,'Arrange controls remain hit-testable while the notice is shown').toEqual([]);
}
async function footerDoesNotOverlap(page:Page){
  const scroll=page.getByTestId('print-controls-scroll'),footer=page.getByTestId('arrange-footer');
  const original=await scroll.evaluate(el=>el.scrollTop);
  const last=scroll.getByRole('button',{name:'Add sheet',exact:true});
  await last.scrollIntoViewIfNeeded();
  const lastRect=await last.boundingBox(),footerRect=await footer.boundingBox(),scrollRect=await scroll.boundingBox();
  expect(footerRect!.y,'Footer follows the last visible control').toBeGreaterThanOrEqual(lastRect!.y+lastRect!.height);
  expect(footerRect!.y,'The entire scrolling area ends above the footer').toBeGreaterThanOrEqual(scrollRect!.y+scrollRect!.height);
  expect(await last.evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));})).toBe(true);
  expect(await footer.evaluate(el=>getComputedStyle(el).backgroundColor)).toMatch(/^rgb\(/);
  await scroll.evaluate((el,top)=>{el.scrollTop=top;},original);
  // Background quality belongs to the footer, never to the tab/header area.
  for(const progress of await page.getByTestId('print-quality-busy').all()){
    expect(await progress.evaluate(el=>!!el.closest('[data-testid="arrange-footer"]'))).toBe(true);
  }
}
async function boxSelectPage(page:Page){
  const p=await paper(page),reg=await registration(page),blocks=reg.strip_blocks.filter((b:any)=>b.sheet===1);
  const left=Math.min(...blocks.map((b:any)=>b.rect_mm[0])),top=Math.min(...blocks.map((b:any)=>b.rect_mm[1])),right=Math.max(...blocks.map((b:any)=>b.rect_mm[2])),bottom=Math.max(...blocks.map((b:any)=>b.rect_mm[3]));
  await page.mouse.move(p.x+(left-2)*p.px,p.y+(top-2)*p.px);await page.mouse.down();await page.mouse.move(p.x+(right+2)*p.px,p.y+(bottom+2)*p.px,{steps:8});await page.mouse.up();
  await expect(page.getByRole('heading',{name:'2 pieces selected'})).toBeVisible();
}
async function storedDraft(page:Page){
  return page.evaluate(id=>new Promise<any>((resolve,reject)=>{
    const opening=indexedDB.open('grainatlas',1);opening.onerror=()=>reject(opening.error);
    opening.onsuccess=()=>{const db=opening.result,request=db.transaction('items','readonly').objectStore('items').get(`grainatlas.draft.${id}`);
      request.onsuccess=()=>{db.close();resolve(request.result?JSON.parse(request.result):null);};request.onerror=()=>{db.close();reject(request.error);};};
  }),projectId);
}

test('move, header clamp, Windows nudge and undo preserve an arrangement across tabs',async({page})=>{
  const unexpected=await open(page);await selectFirst(page);await page.getByLabel('Piece scale',{exact:true}).fill('8');await page.getByLabel('Piece scale',{exact:true}).press('Enter');await ready(page);const initial=await registration(page);
  await dragFirst(page,0,12);const moved=await registration(page);
  expect(moved.bands[0].sheet_rect_mm[1]-initial.bands[0].sheet_rect_mm[1]).toBeCloseTo(12,1);
  await page.getByTestId('page').click({position:{x:5,y:5}});await selectFirst(page);await page.keyboard.press('ArrowDown');await ready(page);
  await page.keyboard.press('Shift+ArrowDown');await ready(page);const nudged=await registration(page);
  expect(nudged.bands[0].sheet_rect_mm[1]-moved.bands[0].sheet_rect_mm[1]).toBeCloseTo(11,4);
  await page.keyboard.press('Control+z');await ready(page);
  expect((await registration(page)).bands[0].sheet_rect_mm[1]).toBeCloseTo(moved.bands[0].sheet_rect_mm[1]+1,4);
  await page.keyboard.press('Control+Shift+z');await ready(page);
  expect((await registration(page)).bands).toEqual(nudged.bands);
  const nav=page.getByRole('navigation',{name:'Screens'});await nav.getByRole('button',{name:'Canvas',exact:true}).click();await nav.getByRole('button',{name:'Layout',exact:true}).click();await ready(page);
  expect((await registration(page)).bands).toEqual(nudged.bands);
  await dragFirst(page,0,-500);const clamped=await registration(page);
  expect(clamped.strip_blocks[0].rect_mm[1]).toBeGreaterThanOrEqual(clamped.sheet_bodies[0].body_rect_mm[1]);
  await expect(page.getByText('Pieces stay below the header',{exact:true})).toBeVisible();expect(unexpected).toEqual([]);
});

test('corner resize updates scale bar, rotation glyph and PDF filename at 1280x800',async({page})=>{
  await page.setViewportSize({width:1280,height:800});await open(page);await selectFirst(page);
  const before=await registration(page),oldScale=before.bands[0].transform.um_per_mm;
  const oldBars=await page.locator('svg[data-testid="page"] text').allTextContents();
  const handle=page.getByTestId('resize-corner-2');const h=await handle.boundingBox(),p=await paper(page);
  await page.mouse.move(h!.x+h!.width/2,h!.y+h!.height/2);await page.mouse.down();await page.mouse.move(h!.x-20*p.px,h!.y-5*p.px,{steps:8});await page.mouse.up();await ready(page);
  const resized=await registration(page);expect(resized.bands[0].transform.um_per_mm).toBeGreaterThan(oldScale);
  // Cross a 1-2-5 threshold deliberately, then check the rendered label too.
  await page.getByLabel('Piece scale',{exact:true}).fill(String(oldScale*2.5));await page.getByLabel('Piece scale',{exact:true}).press('Enter');await ready(page);
  expect(await page.locator('svg[data-testid="page"] text').allTextContents()).not.toEqual(oldBars);
  await page.getByLabel('Rotate right 90 degrees').click();await ready(page);
  await expect(page.locator('[data-orientation]').first()).toBeVisible();await expect(page.locator('svg[data-testid="page"]')).toContainText('mount +y');
  const next=page.waitForEvent('download');await page.getByTestId('export-pdf').click();expect((await next).suggestedFilename()).toMatch(/_print_\d{4}-\d{2}-\d{2}\.pdf$/);
  expect(await problems(page)).toEqual([]);
});

test('box select and proportional group resize; auto-arrange selected and unselected',async({page})=>{
  await open(page);const p=await paper(page),reg=await registration(page);
  const blocks=reg.strip_blocks.filter((b:any)=>b.sheet===1),left=Math.min(...blocks.map((b:any)=>b.rect_mm[0])),top=Math.min(...blocks.map((b:any)=>b.rect_mm[1])),right=Math.max(...blocks.map((b:any)=>b.rect_mm[2])),bottom=Math.max(...blocks.map((b:any)=>b.rect_mm[3]));
  await page.mouse.move(p.x+(left-2)*p.px,p.y+(top-2)*p.px);await page.mouse.down();await page.mouse.move(p.x+(right+2)*p.px,p.y+(bottom+2)*p.px,{steps:8});await page.mouse.up();
  await expect(page.getByRole('heading',{name:/pieces selected/})).toBeVisible();
  const width=Number(await page.getByLabel('Width (mm)',{exact:true}).inputValue());await page.getByLabel('Width (mm)',{exact:true}).fill(String(width*0.6));await page.getByLabel('Width (mm)',{exact:true}).press('Enter');await ready(page);
  const resized=await registration(page);for(const b of resized.bands.filter((b:any)=>b.sheet===1))expect(b.transform.um_per_mm).toBeCloseTo(reg.bands.find((a:any)=>a.area===b.area&&a.grains[0].id===b.grains[0].id).transform.um_per_mm/0.6,1);
  await page.getByTestId('page').click({position:{x:2,y:2}});await selectFirst(page);await page.getByLabel('Auto-arrange scope').selectOption('unselected');await page.getByRole('button',{name:'Auto-arrange',exact:true}).click();await ready(page);
  const unselected=await registration(page);expect(unselected.bands.find((b:any)=>b.grains[0].id===resized.bands[0].grains[0].id).transform.um_per_mm).toEqual(resized.bands[0].transform.um_per_mm);
  await page.getByLabel('Auto-arrange scope').selectOption('selected');await page.getByRole('button',{name:'Auto-arrange',exact:true}).click();await ready(page);const selected=await registration(page);expect(selected.bands.find((b:any)=>b.grains[0].id===reg.bands[0].grains[0].id).transform.um_per_mm).toEqual(reg.um_per_mm);
});

test('Shift adds pieces; overlap is refused and Back to automatic confirms',async({page})=>{
  await open(page);const blocks=page.getByTestId('strip-block');await blocks.nth(0).click({position:{x:8,y:4}});await blocks.nth(1).click({modifiers:['Shift'],position:{x:8,y:4}});await expect(page.getByRole('heading',{name:'2 pieces selected'})).toBeVisible();
  await blocks.first().click({position:{x:8,y:4}}); // selected group stays selected until clicking empty paper
  await page.getByTestId('page').click({position:{x:2,y:2}});await selectFirst(page);const before=await registration(page),other=before.strip_blocks[1].rect_mm;
  // Move onto the next piece's complete position in one pointer operation.
  const r=before.strip_blocks[0].rect_mm;await dragFirst(page,other[0]-r[0],other[1]-r[1]);await expect(page.getByTestId('dropped-edits')).toContainText('drop refused');
  await page.getByRole('button',{name:'Back to automatic',exact:true}).click();await expect(page.getByRole('dialog',{name:'Back to automatic'})).toBeVisible();await page.getByRole('button',{name:'Cancel',exact:true}).click();await expect(page.getByTestId('arrange-mode')).toHaveAttribute('aria-pressed','true');
  await page.getByRole('button',{name:'Back to automatic',exact:true}).click();await page.getByRole('button',{name:'Drop edits',exact:true}).click();await ready(page);await expect(page.getByTestId('arrange-mode')).toHaveAttribute('aria-pressed','false');
});


test('version 2 arrangement reopens from a saved mirror record without changing targets',async({page})=>{
  const fixture:{saved?:string;syncOnSave:boolean}={syncOnSave:false};await open(page,fixture);await selectFirst(page);
  await page.getByLabel('Piece scale',{exact:true}).fill('8');await page.getByLabel('Piece scale',{exact:true}).press('Enter');await ready(page);
  await page.getByLabel('Piece rotation',{exact:true}).fill('17.5');await page.getByLabel('Piece rotation',{exact:true}).press('Enter');await ready(page);
  const before=await registration(page),downloaded=page.waitForEvent('download');await page.getByTestId('save-project').click();
  const checks=page.getByTestId('checks-continue');const confirm=await Promise.race([downloaded.then(()=>false),checks.waitFor({state:'visible'}).then(()=>true).catch(()=>false)]);if(confirm)await checks.click();
  fixture.saved=await readFile((await(await downloaded).path())!,'utf8');expect(JSON.parse(fixture.saved).print_sheets.version).toBe(2);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);await page.getByTestId('edit-copy').click();await page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Layout',exact:true}).click();
  await page.getByTestId('sheets').click();await ready(page);await expect(page.getByTestId('arrange-mode')).toHaveAttribute('aria-pressed','true');
  const reopened=await registration(page);expect(reopened.bands).toEqual(before.bands);expect(reopened.strip_blocks).toEqual(before.strip_blocks);
});

test('a clean element outbox keeps its local arrangement when the server bundle has no recipe',async({page})=>{
  const unexpected=await open(page,{syncOnSave:false});await selectFirst(page);
  await page.getByLabel('Piece scale',{exact:true}).fill('8');await page.getByLabel('Piece scale',{exact:true}).press('Enter');await ready(page);
  await page.getByLabel('Piece rotation',{exact:true}).fill('17.5');await page.getByLabel('Piece rotation',{exact:true}).press('Enter');await ready(page);
  const before=await registration(page);
  await expect.poll(async()=>{const d=await storedDraft(page);return d?.pending===null&&d?.print_recipe?.print_sheets?.version===2;}).toBe(true);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation',{name:'Screens'}).getByRole('button',{name:'Layout',exact:true}).click();await page.getByTestId('sheets').click();await ready(page);
  expect((await registration(page)).bands).toEqual(before.bands);
  await page.getByRole('button',{name:'Back to automatic',exact:true}).click();await page.getByRole('dialog',{name:'Back to automatic'}).getByRole('button',{name:'Drop edits'}).click();await ready(page);
  await expect.poll(async()=>(await storedDraft(page))?.print_recipe).toEqual({});
  expect(unexpected).toEqual([]);
});

for(const {width,height} of [{width:1440,height:900},{width:1280,height:800}]){
  test(`Arrange review screenshots at ${width}x${height}`,async({page})=>{
    await page.setViewportSize({width,height});const unexpected=await open(page);await toolbarWithNotice(page);await selectFirst(page);
    const names=await page.getByLabel('Piece list',{exact:true}).locator('button span:first-child').allTextContents();
    expect(new Set(names).size).toBe(names.length);for(const name of names)expect(name).toMatch(/ · piece \d+ of \d+$/);
    await expect(page.getByTestId('piece-selection-readout')).toHaveText(names[0]);
    await footerDoesNotOverlap(page);
    await page.getByLabel('Piece scale',{exact:true}).fill('8');await page.getByLabel('Piece scale',{exact:true}).press('Enter');await ready(page);
    // Hold a move 1 mm from the margin: Snap makes the guide visible while the
    // piece remains selected. Capture before pointerup clears the guides.
    const b=await page.getByTestId('strip-block').first().boundingBox(),p=await paper(page);
    await page.mouse.move(b!.x+8,b!.y+4);await page.mouse.down();await page.mouse.move(b!.x+8+p.px,b!.y+4+12*p.px,{steps:8});
    // An SVG guide has zero geometric width or height; Playwright's visible
    // rectangle predicate does not describe a stroked line.
    await expect.poll(()=>page.getByTestId('snap-guide').count()).toBeGreaterThan(0);
    await screenshot(page,'selected-snap',width,height);await page.mouse.up();await ready(page);
    // Leave the group's lower handle clear of the floating zoom controls.
    await page.getByTestId('strip-block').nth(1).click({position:{x:8,y:4}});
    await page.getByLabel('Position y (mm)').fill('120');await page.getByLabel('Position y (mm)').press('Enter');await ready(page);
    await boxSelectPage(page);
    const bars=()=>page.locator('[data-piece] g[stroke-linecap="butt"] text').allTextContents(),before=await bars();
    const a=await page.getByTestId('resize-corner-0').boundingBox(),z=await page.getByTestId('resize-corner-2').boundingBox();
    const ax=a!.x+a!.width/2,ay=a!.y+a!.height/2,zx=z!.x+z!.width/2,zy=z!.y+z!.height/2;
    await page.mouse.move(zx,zy);await page.mouse.down();await page.mouse.move(ax+(zx-ax)*0.35,ay+(zy-ay)*0.35,{steps:8});await page.mouse.up();await ready(page);
    expect(await bars()).not.toEqual(before);await expect(page.getByRole('heading',{name:'2 pieces selected'})).toBeVisible();
    await footerDoesNotOverlap(page);
    await screenshot(page,'group-resized',width,height);
    await page.getByTestId('page').click({position:{x:2,y:2}});await selectFirst(page);
    const precise=22.857061594982536,scale=page.getByLabel('Piece scale',{exact:true});
    await scale.fill(String(precise));await scale.press('Enter');await ready(page);
    const exact=(await registration(page)).bands[0].transform.um_per_mm;
    expect(exact).toBeCloseTo(precise,10);expect(exact).not.toBe(22.86);
    await expect(scale).toHaveValue('22.86');await scale.focus();await scale.press('Tab');
    expect((await registration(page)).bands[0].transform.um_per_mm).toBe(exact);
    // Leave clearance for the rotated bounding box without moving its neighbours.
    await page.getByLabel('Position x (mm)').fill('200');await page.getByLabel('Position x (mm)').press('Enter');await ready(page);
    await page.getByLabel('Rotate right 90 degrees').click();await ready(page);
    await expect(page.locator('[data-orientation]').first()).toBeVisible();await expect(page.getByTestId('page')).toContainText('mount +y');
    await expect(page.getByTestId('piece-selection-readout')).toHaveText(names[0]);await footerDoesNotOverlap(page);
    await screenshot(page,'rotated',width,height);
    expect(await problems(page)).toEqual([]);expect(unexpected).toEqual([]);
  });
}
