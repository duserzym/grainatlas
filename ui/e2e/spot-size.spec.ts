import { elementsOf, storedElements, diffElements, changeCounts } from '../src/lib/sync';
import { elementHash } from '../src/lib/sync-hash';
import type { Page, Locator } from '@playwright/test';
import { createHash } from 'node:crypto';
import { test, expect, mockSessionRead, mockRevisionRead } from './mock-session';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
const server = 'https://upbdb.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const mountId = '20000000-0000-4000-8000-000000000001';

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' });
});

async function openSynthetic(page: Page, stalled = false, sizeCases = false, mixed = false, initialSize = 20) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  if (sizeCases) {
    for (const spot of ui.spots) { spot.diameter_um = initialSize; spot.created = '2026-01-01T00:00:00Z'; spot.kind ??= 'spot'; }
    if (mixed) ui.spots[1].diameter_um = 40;
    const b = structuredClone(project.areas[0]);
    Object.assign(b, { id: 'b', uuid: '30000000-0000-4000-8000-000000000002', name: 'Synthetic second area', sample: 'Synthetic second', pixel_size_nm: 500 });
    b.mosaic.um_per_px = .5;
    project.areas.push(b);
    atlas.areas.push({ ...structuredClone(atlas.areas[0]), id: 'b', uuid: b.uuid, name: b.name, sample: b.sample, um_per_px: .5 });
    // Both invented areas reuse the same synthetic bitmap; their calibrations differ.
    ui.grains.b = ui.grains.a.map((g: any, i: number) => ({ ...g, id: `40000000-0000-4000-8000-${String(i+10).padStart(12,'0')}` }));
    ui.spots.push({ ...ui.spots[0], id: '50000000-0000-4000-8000-000000000010', name: 'Synthetic_B', area_id: 'b', grain_id: ui.grains.b[0].id, diameter_um: 40 });
    ui.spots.push({ ...ui.spots[2], id: '50000000-0000-4000-8000-000000000011', name: 'Synthetic_unset', x_um: 180, diameter_um: null });
    const bytes = Buffer.from(JSON.stringify(project));
    Object.assign(atlas.files.find((f: any) => f.path === 'project.json'), { size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
  }
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const elements = {
    grains: Object.entries(ui.grains).flatMap(([area_id, grains]) => (grains as any[]).map(g => ({ ...g, area_id, sample: project.areas.find((a: any) => a.id === area_id).name, project_uuid: projectId, retired: false, merged_from: [] }))),
    spots: ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, kind: 'spot', acquisition_ids: [] })),
    labels: ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  };
  await page.addInitScript(({ server }) => {
    const w = window as any;
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__commands = [];
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => cmd === 'bridge_token' ? 'synthetic-token' : cmd === 'bridge_take_command' ? w.__commands.shift() ?? null : null };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: false, quality: 100, nasRoot: '' }));
  }, { server });
  const unexpected: string[] = [];
  let identityReads = 0;
  let releaseIdentity!: () => void;
  const identityGate = new Promise<void>(resolve => { releaseIdentity = resolve; });
  await page.route(`${server}/**`, async route => {
    const request = route.request(), pathname = new URL(request.url()).pathname;
    const headers = { 'access-control-allow-origin': '*' };
    const json = (value: unknown) => route.fulfill({ json: value, headers });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...headers, 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (await mockRevisionRead(route)) return;
    const session = mockSessionRead(request.method(), pathname);
    if (session !== undefined) {
      if (['/v1/accounts/me', '/v1/server'].includes(pathname)) {
        identityReads++;
        if (stalled) await identityGate;
      }
      return json(session);
    }
    if (request.method() === 'GET') {
      if (pathname === '/v1/mounts') return json([{ id: mountId, label: 'Synthetic mount', mount_type: 'epoxy_1in', grain_count: elements.grains.length, spot_count: ui.spots.length, can_edit: true }]);
      if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') return json([]);
      if (pathname === `/atlas/projects/${projectId}/atlas.json`) return json(atlas);
      if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
        if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
        return route.fulfill({ headers, body: sizeCases && relative === 'project.json' ? Buffer.from(JSON.stringify(project)) : await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'Synthetic mount', name: project.name,
        package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1,
        counts: { spots: ui.spots.length, grains: elements.grains.length, labels: 1, grains_retired: 0, grains_without_atlas_id: 0 }, hashes, can_edit: true, current_project_uuid: projectId,
        project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
      if (pathname === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId });
      if (pathname === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
      if (pathname === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision: 1, since: 1, changes: [] });
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return route.fulfill({ status: 404, headers, json: { detail: 'Unexpected synthetic endpoint' } });
  });

  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  return { project, ui, elements, unexpected, releaseIdentity, identityReads: () => identityReads };
}

for (const stalled of [false, true]) test(`dragging a spot edge moves it; only the size input changes its diameter${stalled ? ' while identity reads are stalled' : ''}`, async ({ page }) => {
  const mock = await openSynthetic(page, stalled);
  // The third spot is isolated from the overlapping first two spots and their label.
  const spot = page.getByTestId('spot').filter({ hasText: 'Synthetic_3' });
  const circle = spot.locator('circle:not(.dot):not(.contrast-halo):not(.handle)');
  await expect(circle).toBeVisible();
  const centre = await circle.evaluate((c: SVGCircleElement) => {
    const p = new DOMPoint(c.cx.baseVal.value, c.cy.baseVal.value).matrixTransform(c.getScreenCTM()!);
    return { x: p.x, y: p.y };
  });
  // The mount locator can overlap this spot after the side panel grows. Its
  // passive current-area picture must leave the underlying canvas clickable.
  const hit = await page.evaluate(({ x, y }) => {
    const target = document.elementFromPoint(x, y);
    return { canvas: !!target?.closest('[data-view]'), blocker: target?.closest('[data-ui]')?.getAttribute('data-testid') ?? null };
  }, centre);
  expect(hit, 'The spot centre must receive canvas gestures through passive overlays').toMatchObject({ canvas: true });
  await page.mouse.click(centre.x, centre.y);
  await expect(spot).toHaveClass(/\bsel\b/);
  await expect(page.getByTestId('spot').locator('.handle')).toHaveCount(0);

  const before = await circle.evaluate((c: SVGCircleElement) => {
    const r = c.r.baseVal.value, cx = c.cx.baseVal.value, cy = c.cy.baseVal.value;
    const matrix = c.getScreenCTM()!;
    const centre = new DOMPoint(cx, cy).matrixTransform(matrix);
    const handle = new DOMPoint(cx + r * Math.SQRT1_2, cy + r * Math.SQRT1_2).matrixTransform(matrix);
    const dx = handle.x - centre.x, dy = handle.y - centre.y, length = Math.hypot(dx, dy);
    return { r, cx, x: handle.x, y: handle.y, endX: handle.x + 40 * dx / length, endY: handle.y + 40 * dy / length };
  });
  await page.mouse.move(before.x, before.y);
  await page.mouse.down();
  await page.mouse.move(before.endX, before.endY, { steps: 8 });
  await page.mouse.up();
  await expect(circle).toHaveAttribute('r', String(before.r));
  await expect(circle).not.toHaveAttribute('cx', String(before.cx));
  await expect(spot).toHaveClass(/\bsel\b/);

  await page.getByTestId('size-side').fill('24');
  await page.getByTestId('size-side').press('Enter');
  await expect(circle).toHaveAttribute('r', '12');
  if (stalled) {
    // All edits above completed with both identity responses still withheld.
    expect(mock.identityReads()).toBe(2);
    const labs = page.waitForResponse(r => new URL(r.url()).pathname === '/v1/labs');
    mock.releaseIdentity();
    await labs;
  }
  expect(mock.unexpected).toEqual([]);
});

for (const width of [1280, 1440]) test(`selection sizes, area calibration and stale hosted edits at ${width}px`, async ({ page }) => {
  await page.setViewportSize({ width, height: width === 1280 ? 800 : 900 });
  const mock = await openSynthetic(page, false, true);
  await page.locator('.side .tabs').getByRole('tab', { name: /^Spots / }).click();
  const entry = page.locator('details.spot-entry');
  if (!(await entry.evaluate(e => (e as HTMLDetailsElement).open))) await entry.locator('summary').click();
  const toolbar = page.getByTestId('size-toolbar'), side = page.getByTestId('size-side');
  await toolbar.fill('25'); await toolbar.press('Enter'); await toolbar.blur();
  await expect(toolbar).toHaveValue('25'); await expect(side).toHaveValue('25');
  const first = mock.ui.spots[0], second = mock.ui.spots[1];
  await page.locator(`tr[data-spot-id="${first.id}"]`).click();
  await expect(toolbar).toHaveValue('20'); await expect(side).toHaveValue('20');
  await expect(page.locator('.presets button').filter({ hasText:/^20 µm$/ })).toHaveAttribute('aria-checked','true');
  await expect(page.locator('.presets button').filter({ hasText:/^25 µm$/ })).toHaveAttribute('aria-checked','false');
  await expect(page.getByTestId('spot').filter({ hasText: first.name }).locator('circle:not(.dot):not(.contrast-halo)')).toHaveAttribute('r','10');

  const field = page.getByTestId('size-panel');
  await field.fill('30');
  // The bridge changes selection without blurring the active size field.
  await page.evaluate(value => {
    (window as any).__commands.push(value);
    window.dispatchEvent(new Event('lab-bridge:command'));
  }, { type:'focus-spot', project:projectId, spot:second.id, flash:false });
  await expect(page.locator(`tr[data-spot-id="${second.id}"]`)).toHaveClass(/selected/);
  await expect(field).toBeFocused(); await expect(field).toHaveValue('20');
  await field.blur();
  await expect(page.locator(`tr[data-spot-id="${second.id}"] td.num`)).toHaveText('20');
  await expect(page.getByTestId('spot').filter({ hasText: second.name }).locator('circle:not(.dot):not(.contrast-halo)')).toHaveAttribute('r','10');

  const area = page.getByRole('combobox', { name: 'Area', exact: true });
  await area.selectOption('b');
  await expect(toolbar).toHaveValue('40'); await expect(side).toHaveValue('40');
  await expect(page.getByTestId('canvas-um-space')).toHaveAttribute('transform','scale(2)');
  await expect(page.getByTestId('spot').filter({ hasText:'Synthetic_B' }).locator('circle:not(.dot):not(.contrast-halo)')).toHaveAttribute('r','20');
  await area.selectOption('a');
  const unset = mock.ui.spots.find((s: any) => s.name === 'Synthetic_unset');
  await expect(page.locator(`tr[data-spot-id="${unset.id}"] td.num`)).toHaveText('not set');
  await page.locator(`tr[data-spot-id="${unset.id}"]`).click();
  await expect(field).toHaveValue(''); await expect(field).toHaveAttribute('placeholder','not set');
  await expect(page.getByTestId('spot-no-size')).toHaveAttribute('stroke-dasharray','3 3');
  await expect(page.getByTestId('spots-no-size')).toContainText('1 spot has no size');
  await page.getByRole('button', {name:'Fix sizes…'}).click();
  await expect(page.getByTestId('fix-sizes-dialog')).toContainText('Synthetic_unset');
  await expect(page.locator(`tr[data-spot-id="${unset.id}"] td.num`)).toHaveText('not set');
  await page.getByTestId('fix-sizes-dialog').getByRole('button', {name:'Cancel',exact:true}).click();
  await expect(page.locator(`tr[data-spot-id="${unset.id}"] td.num`)).toHaveText('not set');
  // Render, selection and area navigation must round-trip registered sizes without edits.
  const download = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  // Invalid-size checks are advisory and cannot prevent saving the unchanged project.
  const outcome = await Promise.race([
    download.then(() => 'saved'),
    page.getByTestId('checks-continue').waitFor({ state:'visible' }).then(() => 'checks'),
  ]);
  if (outcome === 'checks') await page.getByTestId('checks-continue').click();
  const savedPath = await (await download).path();
  const saved = JSON.parse(await readFile(savedPath!, 'utf8'));
  const before = storedElements({ spots: mock.elements.spots, grains:[], labels:[] });
  const after = storedElements({ spots: elementsOf(saved).spots, grains:[], labels:[] });
  expect(await elementHash(after.spots)).toBe(await elementHash(before.spots));
  expect(changeCounts(diffElements(before,after))).toEqual({ spots:0, grains:0, labels:0 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect(mock.unexpected).toEqual([]);
});


// Screen-space radius assertions use the mosaic-pixel transform for zoom and the
// fixture's calibration independently of the µm overlay's transform.
async function expectScreenRadius(circle: Locator, diameter: number, calibration: number) {
  await expect(circle).toHaveCount(1);
  await expect.poll(async () => circle.evaluate((node, { diameter, calibration }) => {
    const c = node as SVGCircleElement;
    const m = c.getScreenCTM()!;
    const pixel = c.closest('svg')!.querySelector('g')!.getScreenCTM()!;
    const zoom = Math.hypot(pixel.a, pixel.b);
    return Math.abs(c.r.baseVal.value * Math.hypot(m.a, m.b) - diameter / calibration * zoom / 2);
  }, { diameter, calibration })).toBeLessThanOrEqual(1);
}
const ringOf = (page: Page, name: string) => page.getByTestId('spot').filter({ has: page.locator('text').filter({ hasText: new RegExp(`^${name}$`) }) }).locator('circle:not(.dot):not(.contrast-halo)');
type SizeControl = 'toolbar' | 'presets' | 'entry stepper' | 'floating panel';
async function changeSize(page: Page, control: SizeControl, initial: number) {
  if (control === 'presets') {
    await page.getByRole('radiogroup', { name: 'Spot diameter', exact: true }).getByRole('radio', { name: '35 µm', exact: true }).click();
    return 35;
  }
  const field = page.getByTestId(control === 'toolbar' ? 'size-toolbar' : control === 'floating panel' ? 'size-panel' : 'size-side');
  if (control === 'entry stepper') {
    // Exercise real +/- button events, including grouping a repeat as one edit.
    const box = field.locator('..');
    await box.getByRole('button', { name: '1 µm larger (Shift: 5 µm)', exact: true }).click();
    await expect(field).toHaveValue(String(initial + 1));
    await box.getByRole('button', { name: '1 µm larger (Shift: 5 µm)', exact: true }).click();
    return initial + 2;
  }
  await field.fill('30'); await field.press('Enter'); await field.blur();
  if (control === 'floating panel') {
    const plus = field.locator('..').getByRole('button', { name: '1 µm larger (Shift: 5 µm)', exact: true });
    await plus.click(); await expect(field).toHaveValue('31'); await plus.click();
    return 32;
  }
  return 30;
}
async function sizeSetup(page: Page, area: string, mixed = false, initialSize = 20) {
  const mock = await openSynthetic(page, false, true, mixed, initialSize);
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(area);
  await page.locator('.side .tabs').getByRole('tab', { name: /^Spots / }).click();
  const entry = page.locator('details.spot-entry');
  if (!(await entry.evaluate(e => (e as HTMLDetailsElement).open))) await entry.locator('summary').click();
  return mock;
}
async function draftElements(page: Page) {
  return page.evaluate(async projectId => {
    // Flush via the existing boundary. Read the write-ahead journal while an
    // IndexedDB commit is queued; otherwise read the committed outbox itself.
    window.dispatchEvent(new Event('pagehide'));
    const key = `grainatlas.draft.${projectId}`;
    const journal = JSON.parse(localStorage.getItem(`grainatlas.journal.${key}`) || 'null');
    const value = journal ? journal.value : await new Promise<string | null>((resolve, reject) => {
      const open = indexedDB.open('grainatlas', 1);
      open.onerror = () => reject(open.error);
      open.onsuccess = () => {
        const db = open.result;
        const read = db.transaction('items', 'readonly').objectStore('items').get(key);
        read.onsuccess = () => { db.close(); resolve(read.result ?? localStorage.getItem(key)); };
        read.onerror = () => { db.close(); reject(read.error); };
      };
    });
    const draft = JSON.parse(value || 'null');
    return draft?.pending ?? draft?.base ?? null;
  }, projectId);
}
async function ensureExpandedPanel(page: Page) {
  const expand = page.getByRole('button', { name: 'Expand spot and grain info', exact: true });
  if (await expand.isVisible()) await expand.click();
}

for (const width of [1280, 1440]) for (const area of ['a', 'b']) {
  for (const control of ['toolbar', 'presets', 'entry stepper'] as const) test(`${control}: next size, preview, placement and undo in area ${area} at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 1280 ? 800 : 900 });
    const mock = await sizeSetup(page, area);
    const calibration = mock.project.areas.find((a: any) => a.id === area).mosaic.um_per_px;
    const before = await draftElements(page);
    const count = await page.getByTestId('spot').count();
    await page.getByTestId('tool-spot').click();
    const diameter = await changeSize(page, control, area === 'a' ? 20 : 40);
    await expect(page.getByTestId('size-toolbar')).toHaveValue(String(diameter));
    expect(await draftElements(page)).toEqual(before);
    // A snapped preview must show the chosen size as well as its fixed snap cue.
    const grain = mock.ui.grains[area][0];
    const snapPoint = await page.getByTestId('canvas-um-space').evaluate((g, point) => {
      const p = new DOMPoint(point[0], point[1]).matrixTransform((g as SVGGraphicsElement).getScreenCTM()!);
      return { x: p.x, y: p.y };
    }, grain.centroid_um);
    await page.mouse.move(snapPoint.x, snapPoint.y);
    await expect(page.locator('[data-testid=spot-preview-layer] path')).toHaveCount(1);
    await expectScreenRadius(page.getByTestId('spot-ghost'), diameter, calibration);
    // Pick an unobscured empty canvas location, away from registered spots and labels.
    const point = await page.getByTestId('canvas-um-space').evaluate((g, spots) => {
      const m = (g as SVGGraphicsElement).getScreenCTM()!;
      const host = g.closest('[data-view]')!.getBoundingClientRect();
      const centres = spots.map((s: any) => new DOMPoint(s.x_um, s.y_um).matrixTransform(m));
      for (const fy of [.65, .55, .75, .4]) for (const fx of [.4, .5, .3, .6]) {
        const x = host.left + host.width * fx, y = host.top + host.height * fy;
        if (document.elementFromPoint(x, y)?.closest('[data-view]') && centres.every(p => Math.hypot(p.x-x, p.y-y) > 90)) return { x, y };
      }
      throw new Error('No uncovered empty synthetic canvas position');
    }, mock.ui.spots.filter((s: any) => s.area_id === area));
    await page.keyboard.down('Alt'); await page.mouse.move(point.x, point.y);
    await expectScreenRadius(page.getByTestId('spot-ghost'), diameter, calibration);
    await page.mouse.click(point.x, point.y); await page.keyboard.up('Alt');
    await expect(page.getByTestId('spot')).toHaveCount(count + 1);
    const added = page.getByTestId('spot').filter({ has: page.locator('circle') }).last().locator('circle:not(.dot):not(.contrast-halo)');
    await expectScreenRadius(added, diameter, calibration);
    const after = await draftElements(page);
    const created = after.spots.filter((s: any) => !mock.elements.spots.some((b: any) => b.id === s.id));
    expect(created).toHaveLength(1); expect(created[0].diameter_um).toBe(diameter);
    await page.getByRole('button', { name: 'Undo', exact: true }).click();
    await expect(page.getByTestId('spot')).toHaveCount(count);
    const restored = await draftElements(page);
    expect(storedElements(restored)).toEqual(storedElements(before ?? mock.elements));
    expect(mock.unexpected).toEqual([]);
  });
  for (const control of ['toolbar', 'presets', 'entry stepper', 'floating panel'] as const) test(`${control}: selected size redraw, outbox and exact undo in area ${area} at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 1280 ? 800 : 900 });
    const mock = await sizeSetup(page, area);
    const selected = mock.ui.spots.find((s: any) => s.area_id === area && s.name !== 'Synthetic_unset');
    const calibration = mock.project.areas.find((a: any) => a.id === area).mosaic.um_per_px;
    await page.locator(`tr[data-spot-id="${selected.id}"]`).click(); await ensureExpandedPanel(page);
    const baseline = storedElements(await draftElements(page) ?? mock.elements);
    const diameter = await changeSize(page, control, selected.diameter_um);
    await expectScreenRadius(ringOf(page, selected.name), diameter, calibration);
    await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
    await expectScreenRadius(ringOf(page, selected.name), diameter, calibration);
    const after = storedElements(await draftElements(page));
    expect(changeCounts(diffElements(baseline, after))).toEqual({ spots: 1, grains: 0, labels: 0 });
    expect(after.spots).toEqual(baseline.spots.map(s => s.id === selected.id ? { ...s, diameter_um: diameter } : s));
    await page.getByRole('button', { name: 'Undo', exact: true }).click();
    await expectScreenRadius(ringOf(page, selected.name), selected.diameter_um, calibration);
    expect(storedElements(await draftElements(page))).toEqual(baseline);
    expect(mock.unexpected).toEqual([]);
  });
}
for (const control of ['toolbar', 'presets', 'entry stepper', 'floating panel'] as const) test(`${control}: mixed selected sizes restore exactly with one undo`, async ({ page }) => {
  const mock = await sizeSetup(page, 'a', true);
  const [first, second] = mock.ui.spots;
  await page.locator(`tr[data-spot-id="${first.id}"]`).click();
  await page.locator(`tr[data-spot-id="${second.id}"]`).click({ modifiers: ['Shift'] });
  await ensureExpandedPanel(page);
  const baseline = storedElements(await draftElements(page) ?? mock.elements);
  // A mixed stepper needs an explicit base typed first; that commit is the same size run.
  if (control === 'entry stepper') {
    const field = page.getByTestId('size-side'); await field.fill('25'); await field.press('Enter'); await field.blur();
  }
  const diameter = await changeSize(page, control, 25);
  const calibration = mock.project.areas[0].mosaic.um_per_px;
  for (const spot of [first, second]) await expectScreenRadius(ringOf(page, spot.name), diameter, calibration);
  const after = storedElements(await draftElements(page));
  expect(changeCounts(diffElements(baseline, after))).toEqual({ spots: 2, grains: 0, labels: 0 });
  expect(after.spots).toEqual(baseline.spots.map(s => [first.id, second.id].includes(s.id) ? { ...s, diameter_um: diameter } : s));
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  for (const spot of [first, second]) await expectScreenRadius(ringOf(page, spot.name), spot.diameter_um, calibration);
  expect(storedElements(await draftElements(page))).toEqual(baseline);
  expect(mock.unexpected).toEqual([]);
});

async function chooseExactSize(page: Page, control: SizeControl, diameter: number, initial: number) {
  if (control === 'presets') {
    await page.getByRole('radiogroup', { name: 'Spot diameter', exact: true })
      .getByRole('radio', { name: `${diameter} µm`, exact: true }).click();
    return;
  }
  const field = page.getByTestId(control === 'toolbar' ? 'size-toolbar' : control === 'floating panel' ? 'size-panel' : 'size-side');
  if (control === 'entry stepper') {
    const direction = diameter < initial ? 'smaller' : 'larger';
    const button = field.locator('..').getByRole('button', { name: `1 µm ${direction} (Shift: 5 µm)`, exact: true });
    for (let d = initial; d !== diameter; d += Math.sign(diameter - initial) * 5) await button.click({ modifiers: ['Shift'] });
  } else {
    await field.fill(String(diameter)); await field.press('Enter'); await field.blur();
  }
}

async function emptySizePoint(page: Page, spots: any[]) {
  return page.getByTestId('canvas-um-space').evaluate((g, spots) => {
    const matrix = (g as SVGGraphicsElement).getScreenCTM()!;
    const host = g.closest('[data-view]')!.getBoundingClientRect();
    const centres = spots.filter(s => s.area_id === 'a').map(s => new DOMPoint(s.x_um, s.y_um).matrixTransform(matrix));
    for (const fy of [.65, .55, .75, .4, .85, .3]) for (const fx of [.4, .5, .3, .6, .7, .2]) {
      const x = host.left + host.width * fx, y = host.top + host.height * fy;
      if (document.elementFromPoint(x, y)?.closest('[data-view]') && centres.every(p => Math.hypot(p.x - x, p.y - y) > 90)) return { x, y };
    }
    throw new Error('No uncovered empty synthetic canvas position');
  }, spots);
}

for (const groupControl of ['toolbar', 'presets', 'entry stepper', 'floating panel'] as const)
  for (const nextControl of ['toolbar', 'presets', 'entry stepper'] as const)
    for (const clearing of ['keep selected', 'Escape', 'empty canvas'] as const)
      test(`size-stuck regression: ${groupControl} group → ${nextControl} next, ${clearing}, 1280x800`, async ({ page }) => {
        await page.setViewportSize({ width: 1280, height: 800 });
        const mock = await sizeSetup(page, 'a', false, 25);
        const [first, second] = mock.ui.spots;
        const baseline = storedElements(await draftElements(page) ?? mock.elements);
        await page.locator(`tr[data-spot-id="${first.id}"]`).click();
        await page.locator(`tr[data-spot-id="${second.id}"]`).click({ modifiers: ['Shift'] });
        await ensureExpandedPanel(page);
        await chooseExactSize(page, groupControl, 20, 25);
        const grouped = storedElements(await draftElements(page));
        expect(grouped.spots).toEqual(baseline.spots.map(s => [first.id, second.id].includes(s.id) ? { ...s, diameter_um: 20 } : s));
        const calibration = mock.project.areas[0].mosaic.um_per_px;
        for (const spot of [first, second]) await expectScreenRadius(ringOf(page, spot.name), 20, calibration);
        await expect(page.getByTestId('size-toolbar')).toHaveAccessibleName(/2 selected · sets next spot/);

        if (clearing === 'Escape') await page.keyboard.press('Escape');
        if (clearing === 'empty canvas') {
          await page.getByTestId('tool-select').click();
          const point = await emptySizePoint(page, grouped.spots);
          await page.mouse.click(point.x, point.y);
        }
        if (clearing !== 'keep selected') await expect(page.getByTestId('size-toolbar')).toHaveAccessibleName(/next spot;/);
        // With no selection, start the next controls at 25 so their actual
        // field, chip or stepper event must take the next size back to 20.
        if (clearing !== 'keep selected') await chooseExactSize(page, 'toolbar', 25, 20);
        // Reaffirm 20 through the next control. An unchanged text commit need
        // not emit a write, so the earlier choice must already size placement.
        await chooseExactSize(page, nextControl, 20, clearing === 'keep selected' ? 20 : 25);
        expect(storedElements(await draftElements(page))).toEqual(grouped);
        await page.getByTestId('tool-spot').click();
        const placements = [];
        for (let n = 0; n < 2; n++) {
          const before = storedElements(await draftElements(page));
          const point = await emptySizePoint(page, before.spots);
          await page.keyboard.down('Alt'); await page.mouse.move(point.x, point.y);
          await expectScreenRadius(page.getByTestId('spot-ghost'), 20, calibration);
          await page.mouse.click(point.x, point.y); await page.keyboard.up('Alt');
          await expect.poll(async () => (await draftElements(page))?.spots.length).toBe(before.spots.length + 1);
          const after = storedElements(await draftElements(page));
          const added = after.spots.filter(s => !before.spots.some(b => b.id === s.id));
          expect(added).toHaveLength(1); expect(added[0].diameter_um).toBe(20);
          await expect(page.locator(`tr[data-spot-id="${added[0].id}"]`)).toHaveClass(/selected/);
          await expectScreenRadius(ringOf(page, added[0].name), 20, calibration);
          placements.push(after);
        }
        const newest = placements[1].spots.find(s => !placements[0].spots.some(b => b.id === s.id))!;
        // The last placement is auto-selected. Its size edit must address that
        // new ID rather than the earlier two-spot transaction.
        await ensureExpandedPanel(page);
        await chooseExactSize(page, groupControl === 'floating panel' ? groupControl : nextControl, 15, 20);
        const resized = storedElements(await draftElements(page));
        expect(resized.spots).toEqual(placements[1].spots.map(s => s.id === newest.id ? { ...s, diameter_um: 15 } : s));
        await expectScreenRadius(ringOf(page, newest.name), 15, calibration);
        for (const expected of [placements[1], placements[0], grouped, baseline]) {
          await page.getByRole('button', { name: 'Undo', exact: true }).click();
          expect(storedElements(await draftElements(page))).toEqual(expected);
          for (const spot of expected.spots.filter(s => s.area_id === 'a' && s.diameter_um))
            await expectScreenRadius(ringOf(page, spot.name), spot.diameter_um!, calibration);
        }
        for (const expected of [grouped, placements[0], placements[1], resized]) {
          await page.getByRole('button', { name: 'Redo', exact: true }).click();
          expect(storedElements(await draftElements(page))).toEqual(expected);
          for (const spot of expected.spots.filter(s => s.area_id === 'a' && s.diameter_um))
            await expectScreenRadius(ringOf(page, spot.name), spot.diameter_um!, calibration);
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        expect(mock.unexpected).toEqual([]);
      });
