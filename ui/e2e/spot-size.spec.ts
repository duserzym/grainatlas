import { elementsOf, storedElements, diffElements, changeCounts } from '../src/lib/sync';
import { elementHash } from '../src/lib/sync-hash';
import type { Page } from '@playwright/test';
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

async function openSynthetic(page: Page, stalled = false, sizeCases = false) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  if (sizeCases) {
    for (const spot of ui.spots) { spot.diameter_um = 20; spot.created = '2026-01-01T00:00:00Z'; spot.kind ??= 'spot'; }
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
  await page.locator('.side .tabs').getByRole('radio', { name: /^Spots / }).click();
  const entry = page.locator('details.spot-entry');
  if (!(await entry.evaluate(e => (e as HTMLDetailsElement).open))) await entry.locator('summary').click();
  const toolbar = page.getByTestId('size-toolbar'), side = page.getByTestId('size-side');
  await toolbar.fill('25'); await toolbar.press('Enter'); await toolbar.blur();
  await expect(toolbar).toHaveValue('25'); await expect(side).toHaveValue('25');
  const first = mock.ui.spots[0], second = mock.ui.spots[1];
  await page.locator(`tr[data-spot-id="${first.id}"]`).click();
  await expect(toolbar).toHaveValue('20'); await expect(side).toHaveValue('20');
  await expect(page.locator('.presets button').filter({ hasText:/^20 µm$/ })).toHaveAttribute('aria-pressed','true');
  await expect(page.locator('.presets button').filter({ hasText:/^25 µm$/ })).toHaveAttribute('aria-pressed','false');
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

  const area = page.locator('.toolbar select[aria-label="Area"]');
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
