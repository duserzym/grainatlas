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

for (const stalled of [false, true]) test(`dragging a spot edge moves it; only the size input changes its diameter${stalled ? ' while identity reads are stalled' : ''}`, async ({ page }) => {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const elements = {
    grains: ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', sample: project.areas[0].name, project_uuid: projectId, retired: false, merged_from: [] })),
    spots: ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, kind: 'spot', acquisition_ids: [] })),
    labels: ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  };
  await page.addInitScript(({ server }) => {
    const w = window as any;
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => cmd === 'bridge_token' ? 'synthetic-token' : null };
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
      if (pathname === '/v1/mounts') return json([{ id: mountId, label: 'Synthetic mount', mount_type: 'epoxy_1in', grain_count: 2, spot_count: 3, can_edit: true }]);
      if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') return json([]);
      if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
        if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
        return route.fulfill({ headers, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'Synthetic mount', name: project.name,
        package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1,
        counts: { spots: 3, grains: 2, labels: 1, grains_retired: 0, grains_without_atlas_id: 0 }, hashes, can_edit: true, current_project_uuid: projectId,
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
    expect(identityReads).toBe(2);
    const labs = page.waitForResponse(r => new URL(r.url()).pathname === '/v1/labs');
    releaseIdentity();
    await labs;
  }
  expect(unexpected).toEqual([]);
});
