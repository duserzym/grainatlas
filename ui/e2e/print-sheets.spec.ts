import type { Page } from '@playwright/test';
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

// Serve only the invented bundle and database reads needed to enter Edit and print.
async function mockServer(page: Page) {
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
  await page.addInitScript(({ server }) => {
    const w = window as any;
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => {
      if (cmd === 'bridge_token') return 'synthetic-token';
      return null;
    } };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: true, quality: 100, nasRoot: '' }));
  }, { server });
  await page.route(`${server}/**`, async route => {
    const request = route.request(), pathname = new URL(request.url()).pathname;
    const headers = { 'access-control-allow-origin': '*' };
    const json = (value: unknown) => route.fulfill({ json: value, headers });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...headers, 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (await mockRevisionRead(route)) return;
    const session = mockSessionRead(request.method(), pathname);
    if (session !== undefined) return json(session);
    if (request.method() === 'GET') {
      if (pathname === '/v1/mounts') return json([{ id: mountId, label: 'mount A', mount_type: 'epoxy_1in', grain_count: 2, spot_count: 3, can_edit: true }]);
      if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') return json([]);
      if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
        if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
        return route.fulfill({ headers, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'mount A', name: project.name,
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

for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  test(`grain sheets switch from lines to packed and fit ${viewport.width} x ${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.addInitScript(() => {
      const w = window as any; w.__printCoreCalls = 0; w.__pdfExports = []; w.__downloadRequests = [];
      const blobs = new Map<string, Blob>();
      const createURL = URL.createObjectURL.bind(URL);
      URL.createObjectURL = blob => { const url = createURL(blob); if (blob instanceof Blob) blobs.set(url, blob); return url; };
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function() {
        if (this.download) w.__downloadRequests.push(this.download);
        if (this.download.endsWith('.pdf')) {
          const blob = blobs.get(this.href);
          const saved = { name: this.download, type: blob?.type, size: blob?.size, embedded: false };
          w.__pdfExports.push(saved);
          void blob?.text().then(text => { saved.embedded = text.includes('/EmbeddedFiles') && text.includes('print_registration.json'); });
          return;
        }
        return click.call(this);
      };
      const send = Worker.prototype.postMessage;
      Worker.prototype.postMessage = function(message: any, ...args: any[]) {
        if (['printSheets', 'printPlanRender', 'printWhole'].includes(message?.op)) w.__printCoreCalls++;
        return Reflect.apply(send, this, [message, ...args]);
      };
    });
    const unexpected = await mockServer(page);
    await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
    await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
    await page.getByTestId('edit-copy').click();
    await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
    const packing = page.getByTestId('sheet-packing');
    const stats = page.getByTestId('sheet-stats');
    await expect(packing).toHaveValue('lines');
    await page.getByTestId('sheets').click();
    await expect(stats).toContainText('in lines');
    await expect(page.getByTestId('page')).toBeVisible();

    const pageURL = await page.getByTestId('page').getAttribute('src');
    const originalStats = await stats.innerText();
    const coreCalls = await page.evaluate(() => (window as any).__printCoreCalls);
    expect(coreCalls).toBeGreaterThan(0);
    const nav = page.getByRole('navigation', { name: 'Screens' });
    await nav.getByRole('button', { name: 'Canvas', exact: true }).click();
    await nav.getByRole('button', { name: 'Layout', exact: true }).click();
    await expect(stats).toHaveText(originalStats);
    // The same live object URL proves the generated page was retained, rather
    // than made again. No Make action occurs on return.
    await expect(page.getByTestId('page')).toHaveAttribute('src', pageURL!);
    await expect(page.getByTestId('busy')).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__printCoreCalls)).toBe(coreCalls);
    await expect(page.getByText(/line width/)).toBeVisible();
    expect(await problems(page)).toEqual([]);

    // An oversized automatic band must explain why export is unavailable,
    // even with the strip editor closed. Do not bypass the geometry guard.
    await page.getByTestId('sheet-scale').fill('0.1');
    await page.getByTestId('sheets').click();
    await expect(page.getByTestId('export-blocked')).toContainText('outside sheet body');
    await expect(page.getByTestId('export-pdf')).toBeDisabled();
    await page.getByTestId('sheet-scale').fill('');
    await packing.selectOption('packed');
    await expect(stats).toContainText('in lines');
    await page.getByTestId('sheets').click();
    await expect(stats).toContainText('packed');
    await expect(stats).not.toContainText('in lines');
    await expect(page.getByTestId('page')).toBeVisible();
    await expect(page.getByText(/blank on this sheet/)).toBeVisible();
    await expect(page.getByText(/line width/)).toHaveCount(0);

    await expect(page.getByRole('button', { name: 'Export SVG', exact: true })).toHaveCount(0);
    await page.getByTestId('export-pdf').click();
    await expect.poll(() => page.evaluate(() => (window as any).__pdfExports.length)).toBe(1);
    await expect.poll(() => page.evaluate(() => (window as any).__pdfExports[0]?.embedded)).toBe(true);
    const pdfExport = await page.evaluate(() => (window as any).__pdfExports[0]);
    const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
    expect(pdfExport.name).toBe(`${project.package.name}_print_${new Date().toISOString().slice(0, 10)}.pdf`);
    expect(await page.evaluate(() => (window as any).__downloadRequests)).toEqual([pdfExport.name]);
    expect(pdfExport.type).toBe('application/pdf');
    expect(pdfExport.size).toBeGreaterThan(100);

    const downloaded = page.waitForEvent('download');
    await page.getByTestId('export-registration').click();
    const file = await (await downloaded).path();
    if (!file) throw new Error('Registration download has no local path');
    const registration = JSON.parse(await readFile(file, 'utf8'));
    expect(registration.layout).toBe('packed');
    expect(registration.bands.length).toBeGreaterThan(0);
    expect(await page.locator('aside.side').evaluate(e => e.getBoundingClientRect().width / parseFloat(getComputedStyle(document.documentElement).fontSize))).toBeLessThanOrEqual(21);
    expect(await problems(page)).toEqual([]);
    expect(unexpected).toEqual([]);
  });
}
