import { test, expect, type Page } from '@playwright/test';
import { firstFreePlace, LABEL_MM } from '../src/lib/strip-edit';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/print-edit-bundle');
const server = 'https://upbdb.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const mountId = '20000000-0000-4000-8000-000000000001';

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'], { cwd: root, stdio: 'pipe' });
});

test.afterEach(async ({ page }) => {
  await expect(page.getByRole('alert').filter({ hasText: 'DataCloneError' })).toHaveCount(0);
});

// Serve only the invented bundle and database reads needed to enter Edit and print.
interface SavedFixture { saved?: string; syncOnSave?: boolean }
async function mockServer(page: Page, fixture: SavedFixture = {}) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const unexpected: string[] = [];
  const elements = {
    grains: ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', sample: project.areas[0].name, project_uuid: projectId, retired: false, merged_from: [] })),
    spots: ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, kind: 'spot', acquisition_ids: [] })),
    labels: ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  };
  await page.addInitScript(({ server, syncOnSave }) => {
    const w = window as any;
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => {
      if (cmd === 'bridge_token') return 'synthetic-token';
      return null;
    } };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave, quality: 100, nasRoot: '' }));
  }, { server, syncOnSave: fixture.syncOnSave ?? true });
  await page.route(`${server}/**`, async route => {
    const request = route.request(), pathname = new URL(request.url()).pathname;
    const headers = { 'access-control-allow-origin': '*' };
    const json = (value: unknown) => route.fulfill({ json: value, headers });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...headers, 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (request.method() === 'GET') {
      if (pathname === '/v1/mounts') return json([{ id: mountId, label: 'mount A', mount_type: 'epoxy_1in', grain_count: 30, spot_count: 0, can_edit: true }]);
      if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') return json([]);
      if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
        if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
        // Reopen the downloaded project record with the same invented image bundle.
        if (relative === 'project.json' && fixture.saved) return route.fulfill({ headers, body: fixture.saved, contentType: 'application/json' });
        if (relative === 'atlas.json' && fixture.saved) return json({ ...atlas, files: atlas.files.map((f: any) => f.path === 'project.json' ? { ...f, size: Buffer.byteLength(fixture.saved!), sha256: createHash('sha256').update(fixture.saved!).digest('hex') } : f) });
        return route.fulfill({ headers, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'mount A', name: project.name,
        package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1,
        counts: { spots: 0, grains: 30, labels: 0, grains_retired: 0, grains_without_atlas_id: 0 }, hashes, can_edit: true, current_project_uuid: projectId,
        project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
      if (pathname === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId });
      if (pathname === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
      if (pathname === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision: 1, since: 1, changes: [] });
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return route.fulfill({ status: 404, headers, json: { detail: 'Unexpected synthetic endpoint' } });
  });
  return unexpected;
}

function problems(page: Page) {
  return page.evaluate(() => {
    const W = innerWidth, H = innerHeight, out: string[] = [];
    const name = (e: Element) => `${e.tagName.toLowerCase()}.${[...e.classList].filter((c) => !c.startsWith('svelte-')).join('.')}${e.getAttribute('data-testid') ? '#' + e.getAttribute('data-testid') : ''} "${(e.textContent || '').trim().slice(0, 24)}"`;
    const inScroller = (e: Element) => {
      for (let a = e.parentElement; a; a = a.parentElement) {
        const o = getComputedStyle(a).overflowY;
        if ((o === 'auto' || o === 'scroll') && a.scrollHeight > a.clientHeight + 1) return true;
      }
      return false;
    };
    const hidden = (e: Element) => !!e.closest('details:not([open])') && !e.closest('summary') || !(e as HTMLElement).checkVisibility?.();
    const de = document.documentElement;
    if (de.scrollWidth > W + 1 || de.scrollHeight > H + 1) out.push(`page scrolls (${de.scrollWidth}x${de.scrollHeight})`);
    for (const e of document.querySelectorAll('.glass, aside, section, header, nav, main')) {
      const r = e.getBoundingClientRect();
      if (!r.width || hidden(e) || inScroller(e)) continue;
      if (r.right > W + 1 || r.bottom > H + 1 || r.left < -1 || r.top < -1) out.push(`outside the window: ${name(e)}`);
      const cs = getComputedStyle(e);
      if (cs.overflowY === 'hidden' && e.scrollHeight > e.clientHeight + 2) out.push(`cut off at the bottom: ${name(e)}`);
    }
    for (const e of document.querySelectorAll('button, input, select, textarea')) {
      const r = e.getBoundingClientRect();
      if (!r.width || !r.height || hidden(e) || inScroller(e)) continue;
      if (r.right > W + 1 || r.bottom > H + 1) out.push(`control out of reach: ${name(e)}`);
    }
    return out;
  });
}

async function openEditor(page: Page, packing = 'lines', fixture: SavedFixture = {}) {
  const unexpected = await mockServer(page, fixture);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('sheet-scale').fill('4');
  await page.getByTestId('sheet-packing').selectOption(packing);
  await page.getByLabel('denoise (NLM)').uncheck();
  await page.getByLabel('sharpen', { exact: true }).uncheck();
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('sheet-stats')).toContainText(packing === 'lines' ? 'in lines' : 'packed');
  await renderFinished(page);
  await page.getByTestId('edit-strips').check();
  await expect(page.getByTestId('strip-block').first()).toBeVisible();
  return unexpected;
}
async function registration(page: Page) {
  await renderFinished(page);
  const downloaded = page.waitForEvent('download');
  await page.getByTestId('export-registration').click();
  const file = await (await downloaded).path();
  if (!file) throw new Error('Registration download has no local path');
  return JSON.parse(await readFile(file, 'utf8'));
}
async function paper(page: Page) {
  const image = await page.getByTestId('page').boundingBox();
  if (!image) throw new Error('No page');
  return { ...image, px: image.width / 431.8 };
}
async function renderFinished(page: Page) {
  await expect(page.getByTestId('export-pdf')).not.toHaveAttribute('title', 'Wait for the strip preview');
  await expect(page.getByTestId('busy')).toHaveCount(0);
}
async function remakeSheets(page: Page) {
  const previous = await page.getByTestId('page').getAttribute('src');
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('page')).not.toHaveAttribute('src', previous!);
  await renderFinished(page);
}
async function firstStrip(page: Page) {
  await renderFinished(page);
  const block = page.getByTestId('strip-block').first();
  const initial = await paper(page);
  // Hover waits for fitting/resizing to settle before any pointer coordinates are read.
  await block.hover({ position: { x: 8 * initial.px, y: 2 * initial.px } });
  const r = await block.boundingBox();
  if (!r) throw new Error('No strip');
  const p = await paper(page);
  return { r, p };
}
async function drag(page: Page, dx: number, dy: number, alt = true) {
  const { r, p } = await firstStrip(page);
  // The label stays outside band crop handles even after selecting the strip.
  const x = r.x + 8 * p.px, y = r.y + 2 * p.px;
  if (alt) await page.keyboard.down('Alt');
  await page.mouse.move(x, y); await page.mouse.down();
  await page.mouse.move(x + dx * p.px, y + dy * p.px, { steps: 8 });
  await page.mouse.up();
  if (alt) await page.keyboard.up('Alt');
  await renderFinished(page);
}
async function selectFirst(page: Page) {
  await drag(page, 0, 0);
  await expect(page.getByRole('textbox', { name: 'Strip label' })).toBeVisible();
}
function sameBand(reg: any, before: any) {
  return reg.bands.find((b: any) => JSON.stringify(b.mosaic_rect_um) === JSON.stringify(before.mosaic_rect_um));
}

async function saveProject(page: Page) {
  await renderFinished(page);
  const downloaded = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  // The generated project can have advisory checks; keep the usual save path.
  const checks = page.getByTestId('checks-continue');
  const confirm = await Promise.race([downloaded.then(() => false), checks.waitFor({ state: 'visible' }).then(() => true).catch(() => false)]);
  if (confirm) await checks.click();
  const file = await (await downloaded).path();
  if (!file) throw new Error('Project download has no local path');
  return readFile(file, 'utf8');
}

async function recoveryPrintEdits(page: Page) {
  return page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const open = indexedDB.open('grainatlas', 1);
      open.onsuccess = () => resolve(open.result); open.onerror = () => reject(open.error);
    });
    try {
      const values = await new Promise<string[]>((resolve, reject) => {
        const get = db.transaction('items').objectStore('items').getAll();
        get.onsuccess = () => resolve(get.result); get.onerror = () => reject(get.error);
      });
      return values.map(v => JSON.parse(v)).map(v => (v?.project ?? v)?.print_sheets).filter(Boolean);
    } finally { db.close(); }
  });
}

test('a downloaded project reopens with its strip move and generating parameters', async ({ page }) => {
  const fixture: SavedFixture = { syncOnSave: false };
  const unexpected = await openEditor(page, 'packed', fixture);
  const original = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const before = await registration(page), band = before.bands[0];
  const thumb = page.getByTestId('page-thumb').nth(1);
  await thumb.hover();
  const { r, p } = await firstStrip(page), t = await thumb.boundingBox();
  if (!t) throw new Error('Missing page thumbnail');
  await page.mouse.move(r.x + 8 * p.px, r.y + 2 * p.px); await page.mouse.down();
  await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2, { steps: 12 }); await page.mouse.up();
  const edited = await registration(page);
  expect(sameBand(edited, band).sheet).toBe(2);
  await expect.poll(() => recoveryPrintEdits(page)).toEqual([expect.objectContaining({ edits: [expect.objectContaining({ op: 'move', sheet: 1 })] })]);
  fixture.saved = await saveProject(page);
  const saved = JSON.parse(fixture.saved);
  expect(saved.print_sheets).toMatchObject({ params: { packing: 'packed', um_per_mm: 4 }, um_per_mm: 4, edits: [{ op: 'move', sheet: 1 }] });
  for (const field of ['grains', 'spots', 'labels']) expect(saved.grainatlas_ui[field]).toMatchObject(original.grainatlas_ui[field]);
  expect(saved.grainatlas_ui.grains.a).toHaveLength(original.grainatlas_ui.grains.a.length);
  expect(saved.upbdb).toEqual(original.upbdb);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('sheet-scale')).toHaveValue('4');
  await expect(page.getByTestId('sheet-packing')).toHaveValue('packed');
  await page.getByTestId('sheets').click();
  await renderFinished(page);
  const reopened = await registration(page);
  for (const field of ['bands', 'strip_blocks', 'edits', 'left_off', 'layout']) expect(reopened[field]).toEqual(edited[field]);
  await expect(page.getByTestId('dropped-edits')).toHaveCount(0);
  // A preview and an ordinary save retain edits which cannot match changed parameters.
  await page.getByTestId('sheet-scale').fill('5'); await remakeSheets(page);
  await expect(page.getByTestId('dropped-edits')).toContainText('1 edit from the saved project could not be applied');
  await expect(page.getByTestId('dropped-edits')).toContainText('parameters changed');
  expect(JSON.parse(await saveProject(page)).print_sheets).toEqual(saved.print_sheets);
  // Returning to the saved scale can still replay the original list before any edit.
  await page.getByTestId('sheet-scale').fill('4'); await remakeSheets(page);
  expect((await registration(page)).bands).toEqual(edited.bands);
  await page.getByTestId('sheet-scale').fill('5'); await remakeSheets(page);
  await page.getByTestId('edit-strips').check();
  await expect(page.getByTestId('dropped-edits')).toHaveCount(1);
  await expect(page.getByRole('region', { name: 'Strip controls', exact: true }).getByTestId('dropped-edits')).toContainText('1 edit from the saved project could not be applied');
  await expect(page.getByTestId('dropped-edits')).toContainText('parameters changed');
  await page.getByRole('button', { name: 'Back to automatic', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Back to automatic' })).toContainText('Drop all 1 edits');
  await page.getByRole('dialog', { name: 'Back to automatic' }).getByRole('button', { name: 'Drop edits' }).click();
  const automatic = JSON.parse(await saveProject(page));
  expect(automatic).not.toHaveProperty('print_sheets');
  expect(automatic.grainatlas_ui.grains).toEqual(saved.grainatlas_ui.grains);
  expect(unexpected).toEqual([]);
});

test('unreadable strip edits show a notice and survive saving until a new edit', async ({ page }) => {
  const original = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const raw = { params: { packing: 'packed' }, um_per_mm: 4, edits: [{ op: 'future_edit', value: ['zr-1', 42] }] };
  const fixture: SavedFixture = { saved: JSON.stringify({ ...original, print_sheets: raw }), syncOnSave: false };
  const unexpected = await openEditor(page, 'packed', fixture);
  await expect(page.getByTestId('unreadable-print-edits')).toContainText('Stored strip edits could not be read');
  const saved = JSON.parse(await saveProject(page));
  expect(saved.print_sheets).toEqual(raw);
  expect(saved.upbdb).toEqual(original.upbdb);
  for (const field of ['grains', 'spots', 'labels']) expect(saved.grainatlas_ui[field]).toMatchObject(original.grainatlas_ui[field]);
  await drag(page, 10, 0);
  await expect(page.getByTestId('unreadable-print-edits')).toHaveCount(0);
  expect(JSON.parse(await saveProject(page)).print_sheets.edits).toEqual([expect.objectContaining({ op: 'move' })]);
  expect(unexpected).toEqual([]);
});

test('drag changes only paper position; arrow keys nudge exactly and remaking replays edits', async ({ page }) => {
  await openEditor(page, 'packed');
  const before = await registration(page), band = before.bands[0];
  await drag(page, 10, 0);
  const moved = sameBand(await registration(page), band);
  expect(moved.mosaic_rect_um).toEqual(band.mosaic_rect_um);
  expect(moved.sheet_rect_mm[0] - band.sheet_rect_mm[0]).toBeCloseTo(10, 1);
  expect(moved.sheet_rect_mm[1]).toBeCloseTo(band.sheet_rect_mm[1], 2);
  await page.keyboard.press('ArrowRight');
  const nudged = sameBand(await registration(page), band);
  expect(nudged.sheet_rect_mm[0] - moved.sheet_rect_mm[0]).toBeCloseTo(0.5, 6);
  await page.keyboard.press('Shift+ArrowRight');
  const shifted = sameBand(await registration(page), band);
  expect(shifted.sheet_rect_mm[0] - nudged.sheet_rect_mm[0]).toBeCloseTo(5, 6);
  await remakeSheets(page);
  expect(sameBand(await registration(page), band).sheet_rect_mm).toEqual(shifted.sheet_rect_mm);
  await page.getByTestId('sheet-scale').fill('5');
  await remakeSheets(page);
  await expect(page.getByTestId('dropped-edits')).toContainText('parameters changed');
});

test('a strip drops on another page thumbnail at the first free place', async ({ page }) => {
  await openEditor(page, 'packed');
  expect(await page.getByTestId('page-thumb').count()).toBeGreaterThanOrEqual(2);
  const before = await registration(page), band = before.bands[0];
  const thumb = page.getByTestId('page-thumb').nth(1);
  await thumb.hover();
  const { r, p } = await firstStrip(page);
  const t = await thumb.boundingBox();
  if (!t) throw new Error('Missing page thumbnail');
  await page.mouse.move(r.x + 8 * p.px, r.y + 2 * p.px); await page.mouse.down();
  await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2, { steps: 12 }); await page.mouse.up();
  const after = sameBand(await registration(page), band);
  expect(after.sheet).toBe(2);
  expect(after.mosaic_rect_um).toEqual(band.mosaic_rect_um);
  const block = before.strip_blocks[0].rect_mm;
  const expected = firstFreePlace(before.sheet_bodies[1].body_rect_mm, [block[2] - block[0], block[3] - block[1]], before.strip_blocks.filter((b: any) => b.sheet === 2).map((b: any) => b.rect_mm));
  expect(expected).not.toBeNull();
  expect(after.sheet_rect_mm[0]).toBeCloseTo(expected![0], 5);
  expect(after.sheet_rect_mm[1] - LABEL_MM).toBeCloseTo(expected![1], 5);
});

test('crop previews grains in red and counts exactly those left off', async ({ page }) => {
  await openEditor(page);
  await selectFirst(page);
  await renderFinished(page);
  const handle = page.getByTestId('crop-edge-0').first();
  await handle.hover();
  const r = await handle.boundingBox();
  const p = await paper(page);
  if (!r) throw new Error('No crop handle');
  await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2); await page.mouse.down();
  await page.mouse.move(r.x + r.width / 2 + 30 * p.px, r.y + r.height / 2, { steps: 8 });
  await expect(page.getByTestId('grain-leaving')).toHaveCount(2);
  await page.mouse.up();
  await renderFinished(page);
  await expect(page.getByTestId('sheet-stats')).toContainText('2 grains left off by crop');
  const after = await registration(page);
  expect(after.left_off.filter((g: any) => g.reason === 'cropped')).toHaveLength(2);
});

async function overlap(page: Page) {
  await selectFirst(page);
  const blocks = page.getByTestId('strip-block');
  await blocks.nth(1).hover();
  const { r: first, p } = await firstStrip(page);
  const second = await blocks.nth(1).boundingBox();
  if (!second) throw new Error('Need two strips');
  await drag(page, (second.x - first.x) / p.px, (second.y - first.y) / p.px);
  await expect(page.getByTestId('strip-problems')).toContainText('overlap');
}

test('overlap blocks all exports; undo and redo restore validation', async ({ page }) => {
  await openEditor(page);
  await overlap(page);
  for (const name of ['Export PDF', 'Registration', 'Export SVG', 'Print…']) await expect(page.getByRole('button', { name, exact: true })).toBeDisabled();
  await expect(page.getByTestId('export-pdf')).toHaveAttribute('title', /Fix strip problems/);
  await page.keyboard.press('ControlOrMeta+z');
  await renderFinished(page);
  await expect(page.getByTestId('strip-problems')).toHaveCount(0);
  await expect(page.getByTestId('export-pdf')).toBeEnabled();
  await expect(page.getByTestId('export-registration')).toBeEnabled();
  await page.keyboard.press('ControlOrMeta+Shift+z');
  await renderFinished(page);
  await expect(page.getByTestId('strip-problems')).toContainText('overlap');
  await expect(page.getByTestId('export-registration')).toBeDisabled();
});

async function refusedThumbnailDrop(page: Page) {
  const controls = page.getByRole('region', { name: 'Strip controls', exact: true });
  const panel = page.locator('aside').filter({ has: controls });
  // Start where the operator edits, then do not scroll the panel to reveal the notice.
  await panel.evaluate(el => { el.scrollTop = 0; });
  const thumb = page.getByTestId('page-thumb').first();
  await thumb.hover();
  const { r, p } = await firstStrip(page), t = await thumb.boundingBox();
  if (!t) throw new Error('Missing page thumbnail');
  await page.mouse.move(r.x + 8 * p.px, r.y + 2 * p.px); await page.mouse.down();
  await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2, { steps: 12 }); await page.mouse.up();
  await renderFinished(page);
  await expect(page.getByTestId('dropped-edits')).toHaveCount(1);
  const notice = controls.getByTestId('dropped-edits');
  await expect(notice).toHaveAttribute('role', 'status');
  await expect(notice).toContainText('Not applied: no room on sheet 1');
  await expect(notice).toBeVisible();
  await expect(notice).toBeInViewport({ ratio: 1 });
  expect(await panel.evaluate(el => el.scrollTop)).toBe(0);
  await expect(page.getByTestId('page-thumb').nth(1)).toHaveAttribute('aria-current', 'page');
}

test('a refused thumbnail drop is reported and removed from history and saved edits after rendering', async ({ page }) => {
  const unexpected = await openEditor(page, 'packed', { syncOnSave: false });
  const before = await registration(page);
  const block = before.strip_blocks.find((b: any) => b.sheet === 2).rect_mm;
  expect(firstFreePlace(before.sheet_bodies[0].body_rect_mm,
    [block[2] - block[0], block[3] - block[1]],
    before.strip_blocks.filter((b: any) => b.sheet === 1).map((b: any) => b.rect_mm))).toBeNull();
  await page.getByTestId('page-thumb').nth(1).click();
  await refusedThumbnailDrop(page);
  expect((await registration(page)).bands).toEqual(before.bands);
  await expect(page.getByRole('button', { name: 'Back to automatic', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Redo', exact: true })).toBeDisabled();
  expect(JSON.parse(await saveProject(page))).not.toHaveProperty('print_sheets');
  await expect.poll(() => recoveryPrintEdits(page)).toEqual([]);

  const label = page.getByRole('textbox', { name: 'Strip label' });
  await label.fill('zr-edited'); await label.press('Tab');
  await renderFinished(page);
  await expect(page.getByTestId('dropped-edits')).toHaveCount(0);
  await refusedThumbnailDrop(page);
  await page.getByRole('button', { name: 'Back to automatic', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Back to automatic' });
  await expect(dialog).toContainText('Drop all 1 edits');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  const saved = JSON.parse(await saveProject(page));
  expect(saved.print_sheets.edits).toEqual([expect.objectContaining({ op: 'label', text: 'zr-edited' })]);
  await expect.poll(() => recoveryPrintEdits(page)).toEqual([expect.objectContaining({ edits: saved.print_sheets.edits })]);
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await renderFinished(page);
  expect((await registration(page)).bands).toEqual(before.bands);
  await expect(page.getByRole('button', { name: 'Undo', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Redo', exact: true }).click();
  await renderFinished(page);
  await remakeSheets(page);
  await expect(page.getByTestId('dropped-edits')).toHaveCount(0);
  expect(JSON.parse(await saveProject(page)).print_sheets.edits).toEqual(saved.print_sheets.edits);
  expect(unexpected).toEqual([]);
});

test('split and join restore the original mosaic and Back to automatic defaults to Cancel', async ({ page }) => {
  await openEditor(page);
  const before = await registration(page);
  await selectFirst(page);
  await page.getByRole('button', { name: 'Split', exact: true }).click();
  await renderFinished(page);
  const band = page.getByTestId('strip-band').first();
  await band.hover();
  const r = await band.boundingBox();
  if (!r) throw new Error('No band');
  await page.mouse.click(r.x + r.width / 2, r.y + r.height / 2);
  await renderFinished(page);
  await expect(page.getByRole('button', { name: 'Join', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Join', exact: true }).click();
  const joined = await registration(page);
  expect(joined.bands.map((b: any) => b.mosaic_rect_um)).toEqual(before.bands.map((b: any) => b.mosaic_rect_um));
  await page.getByRole('button', { name: 'Back to automatic', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Back to automatic' });
  await expect(dialog).toContainText('Drop all 2 edits and go back to the automatic layout?');
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).not.toBeVisible();
  expect((await registration(page)).layout).toBe('edited');
  await page.getByRole('button', { name: 'Back to automatic', exact: true }).click();
  await dialog.getByRole('button', { name: 'Drop edits' }).click();
  expect((await registration(page)).layout).toBeUndefined();
  await expect(page.getByRole('button', { name: 'Back to automatic', exact: true })).toBeDisabled();
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  test(`refused thumbnail drop stays in the strip controls at ${viewport.width} x ${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const unexpected = await openEditor(page, 'packed');
    await page.getByTestId('page-thumb').nth(1).click();
    await refusedThumbnailDrop(page);
    await page.getByTestId('edit-strips').uncheck();
    await expect(page.getByRole('region', { name: 'Strip controls', exact: true })).toHaveCount(0);
    await expect(page.getByTestId('dropped-edits')).toHaveCount(1);
    await expect(page.getByTestId('dropped-edits')).toContainText('1 edit could not be applied');
    await page.getByTestId('edit-strips').check();
    await expect(page.getByTestId('dropped-edits')).toHaveCount(1);
    await expect(page.getByRole('region', { name: 'Strip controls', exact: true }).getByTestId('dropped-edits')).toContainText('Not applied: no room on sheet 1');
    expect(unexpected).toEqual([]);
  });

  test(`strip editor with selection and problems fits ${viewport.width} x ${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const unexpected = await openEditor(page, 'packed');
    // Page 1 stays reachable while the edit notice is still shown.
    await page.getByTestId('page-thumb').first().hover();
    expect(await page.getByTestId('strip-block').count()).toBeGreaterThanOrEqual(2);
    await overlap(page);
    await expect(page.getByRole('textbox', { name: 'Strip label' })).toBeVisible();
    expect(await problems(page)).toEqual([]);
    await page.getByRole('button', { name: 'Fold print controls' }).click();
    expect(await problems(page)).toEqual([]);
    await page.getByRole('button', { name: 'Show print controls' }).click();
    await expect(page.getByTestId('strip-problems')).toBeVisible();
    expect(unexpected).toEqual([]);
  });
}
