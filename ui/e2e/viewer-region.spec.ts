// Invented atlas only; exercise the real web iframe transport used by the shell.
import { test, expect, mockSessionRead, mockRevisionRead } from './mock-session';
import type { Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ATLAS_EVENTS, FRAME_NAME_PREFIX, MESSAGE_TAG, PROTOCOL_VERSION, type BridgeCommand, type ViewerRegionEvent } from '@lab/bridge';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
const server = 'https://viewer-region.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const mountId = '20000000-0000-4000-8000-000000000001';
const shortMountLabel = 'Invented little mill hill'; // 25 characters, fits the normal header.
const mountLabels = ['Invented mount with a long display label', shortMountLabel];

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' });
});

async function command(page: Page, value: BridgeCommand) {
  await page.evaluate(({ value, tag, v }) => {
    document.querySelector('iframe')!.contentWindow!.postMessage({ tag, v, push: 'command', command: value }, location.origin);
  }, { value, tag: MESSAGE_TAG, v: PROTOCOL_VERSION });
}
async function events(page: Page): Promise<ViewerRegionEvent[]> {
  return page.evaluate(type => (window as unknown as { regionEvents: ViewerRegionEvent[] }).regionEvents.filter(e => e.type === type), ATLAS_EVENTS.viewerRegion);
}

for (const { viewport, mountLabel } of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]
  .flatMap(viewport => mountLabels.map(mountLabel => ({ viewport, mountLabel })))) {
  test(`viewer mount exploration at ${viewport.width} x ${viewport.height}, ${mountLabel.length}-character name`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport);
    const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
    const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
    // Reuse the invented pixels at a second area position; identifiers remain distinct.
    const b = structuredClone(project.areas[0]);
    b.id = 'b'; b.uuid = '30000000-0000-4000-8000-000000000002'; b.name = 'Area B'; b.sample = 'Sample B';
    project.areas[0].name = 'Area A'; project.areas[0].sample = 'Sample A';
    project.areas.push(b);
    atlas.areas.push({ ...structuredClone(atlas.areas[0]), id: 'b', uuid: b.uuid, name: b.name, sample: b.sample });
    const extra = { ...project.grainatlas_ui.spots[0], id: '50000000-0000-4000-8000-000000000004', area_id: 'b', name: 'Invented_B', grain_id: null, x_um: 180, y_um: 160 };
    project.grainatlas_ui.spots.push(extra);
    const spots = project.grainatlas_ui.spots;
    const groupIds: string[] = [spots[0].id, extra.id];
    const elements = { spots: spots.map((s: object) => ({ ...s, project_uuid: projectId, acquisition_ids: [] })), grains: [], labels: [] };
    const hashes = { spots: 'spots-1', grains: 'grains-1', labels: 'labels-1' };
    const counts = { spots: spots.length, grains: 0, labels: 0 };
    const unexpected: string[] = [], writes: string[] = [];
    await page.route(`${server}/**`, async route => {
      const request = route.request(), url = new URL(request.url()), p = url.pathname, method = request.method();
      const headers = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*', 'access-control-expose-headers': 'ETag, X-Atlas-Version', 'X-Atlas-Version': '1' };
      const json = (value: unknown, status = 200) => route.fulfill({ json: value, status, headers });
      if (method === 'OPTIONS') return route.fulfill({ status: 204, headers });
      if (await mockRevisionRead(route)) return;
      if (method !== 'GET') { writes.push(`${method} ${p}`); return json({ detail: 'Viewer must not write' }, 405); }
      if (p.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = p.slice(`/atlas/projects/${projectId}/`.length);
        if (relative === 'atlas.json') return json(atlas);
        if (relative === 'project.json') return json(project);
        return route.fulfill({ body: await readFile(path.join(generated, relative)), headers, contentType: 'image/webp' });
      }
      const session = mockSessionRead(method, p); if (session !== undefined) return json(session);
      if (p === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: mountLabel,
        name: project.name, package_name: project.package.name, atlas_version: 1, revision: 1, counts, hashes, can_edit: false });
      if (p === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId });
      if (p === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
      if (p === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision: 1, since: 1, changes: [] });
      if (['/v1/mounts', '/v1/atlas/projects', '/v1/sessions'].includes(p)) return json([]);
      unexpected.push(`${method} ${p}`); return json({ detail: 'Unexpected request' }, 404);
    });
    await page.addInitScript(({ server }) => {
      localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: false, syncAutomatically: false, quality: 100, nasRoot: '' }));
      sessionStorage.setItem('grainatlas.upbdb.token', 'invented-token');
    }, { server });
    // Parent responds to token/oversight reads and records notifications. Commands enter only
    // through postMessage from the actual parent, never a synthetic child MessageEvent.
    await page.route('**/viewer-region-host', route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><html><body style="margin:0"><script>
      window.regionEvents = [];
      addEventListener('message', e => {
        const m = e.data;
        if (e.source !== document.querySelector('iframe')?.contentWindow || m?.tag !== ${JSON.stringify(MESSAGE_TAG)}) return;
        if (m.op === 'notify') { regionEvents.push(m.event); return; }
        const result = m.op === 'token' ? 'invented-token' : null;
        e.source.postMessage({ tag: m.tag, v: m.v, id: m.id, ok: true, result }, location.origin);
      });
      </script><iframe name="${FRAME_NAME_PREFIX}grainatlas" src="/" style="display:block;width:100vw;height:100vh;border:0"></iframe></body></html>` }));
    await page.goto('/viewer-region-host');
    const frame = page.frameLocator('iframe');
    await expect(frame.locator('.ga-root')).toBeVisible();
    // Wait for onCommand registration, signalled by App's initial hosted draft report.
    await expect.poll(() => page.evaluate(() => (window as unknown as { regionEvents: { type: string }[] }).regionEvents.some(e => e.type === 'lab.atlas.drafts'))).toBe(true);
    await command(page, { type: 'set-presentation', presentation: 'viewer', inset: { collapsed: false, width: 180 } });
    // The command queue is asynchronous; wait for presentation before opening the atlas.
    await expect(frame.locator('.ga-root')).toHaveClass(/viewer/);
    await command(page, { type: 'open-project', project: projectId, server });
    await expect(frame.getByTestId('mount-inset-map')).toBeVisible();
    await command(page, { type: 'review-spots', projects: { [projectId]: spots.map((s: { id: string }) => s.id) }, groups: { [projectId]: [
      { name: 'Sample A', color: '#a34d78', spots: groupIds }, { name: 'Sample B', color: '#347d96', spots: [spots[2].id] },
    ] } });
    await expect(frame.getByTestId('mount-inset-group')).toHaveCount(2);
    // Intentionally before ANY new-header assertion: SCREENSHOTS=1 can capture the old build.
    if (process.env.SCREENSHOTS === '1') await page.screenshot({ path: testInfo.outputPath(`viewer-region-${viewport.width}.png`) });
    const header = frame.getByTestId('mount-inset-label');
    await expect(header).toHaveAttribute('title', `${mountLabel}: Sample A, Sample B`);
    await expect(header).toHaveText(`Mount: ${mountLabel}`);
    await expect(header).toHaveAccessibleName(`Mount: ${mountLabel}`);
    if (mountLabel === shortMountLabel) {
      expect(mountLabel).toHaveLength(25);
      await expect(frame.getByTestId('mount-inset-body')).toHaveCSS('width', '180px');
      await expect.poll(() => header.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
    }
    const caption = frame.locator('[data-testid="mount-inset-body"] .head .name');
    await expect(caption).toHaveAttribute('title', 'Sample A, Sample B');
    await expect(caption).toHaveText('');
    const areaSelect = frame.getByRole('combobox', { name: 'Area', exact: true });
    const camera = frame.locator('.viewer[data-view] > svg.overlay > g');
    const before = await camera.getAttribute('transform');
    const child = page.frames().find(f => f.name() === `${FRAME_NAME_PREFIX}grainatlas`)!;
    const savedCopies = () => child.evaluate(() => Object.fromEntries(Object.entries(localStorage)
      .filter(([key]) => /grainatlas\.(project\.|copy\.|draft|journal)/.test(key))));
    const storedBefore = await savedCopies();
    await header.click();
    await expect.poll(async () => (await events(page)).at(-1)?.kind).toBe('mount');
    expect((await events(page)).at(-1)).toMatchObject({ project: projectId, mount: mountId, mountLabel, area: null, sample: null, spots: [] });
    expect(await camera.getAttribute('transform')).toBe(before);
    // A real pointer click hits the polygon above area B; the summary includes area A too.
    const region = frame.locator('polygon.region[data-area="b"]');
    await expect(region).toHaveAttribute('role', 'button');
    await region.click();
    await expect(areaSelect).toHaveValue('b');
    await expect.poll(async () => (await events(page)).at(-1)?.kind).toBe('sample');
    expect((await events(page)).at(-1)).toMatchObject({ area: 'b', areaLabel: 'Sample B', sample: 'Sample A', spots: groupIds });
    await expect.poll(() => camera.getAttribute('transform')).not.toBe(before);
    // Shown area also fits. Keyboard avoids the overlapping sample polygon.
    const areaB = frame.locator('[data-testid="mount-inset-area"][data-area="b"]');
    await areaB.focus(); await areaB.press('Enter');
    await expect.poll(async () => (await events(page)).at(-1)?.kind).toBe('area');
    expect((await events(page)).at(-1)?.spots).toEqual([extra.id]);
    await expect.poll(() => camera.getAttribute('transform')).toBe(before);
    // An area pointer click outside the sample region switches back and fits the image.
    const areaA = frame.locator('[data-testid="mount-inset-area"][data-area="a"]');
    await areaA.click({ position: { x: 5, y: 5 } });
    await expect(areaSelect).toHaveValue('a');
    await expect.poll(async () => (await events(page)).at(-1)?.area).toBe('a');
    expect((await events(page)).at(-1)?.spots).toEqual(spots.filter((s: { area_id: string }) => s.area_id === 'a').map((s: { id: string }) => s.id));
    await expect.poll(() => camera.getAttribute('transform')).toBe(before);
    // Both keyboard activation keys explore sample regions.
    await region.focus(); await region.press('Space'); await expect(areaSelect).toHaveValue('b');
    await region.press('Enter');
    await expect.poll(async () => (await events(page)).slice(-2).map(e => e.kind)).toEqual(['sample', 'sample']);
    await command(page, { type: 'set-presentation', presentation: 'viewer', inset: { collapsed: false, width: 480 } });
    await expect(caption).toHaveText('Sample A, Sample B');
    await expect(header).toHaveAttribute('title', mountLabel);
    // Without Review groups, use the unique sample labels of all mount areas in mount order.
    await command(page, { type: 'review-spots', projects: {} });
    await expect(frame.getByTestId('mount-inset-group')).toHaveCount(0);
    await expect(caption).toHaveAttribute('title', 'Sample A, Sample B');
    for (const width of [120, 180]) {
      await command(page, { type: 'set-presentation', presentation: 'viewer', inset: { collapsed: false, width } });
      await expect(frame.getByTestId('mount-inset-body')).toHaveCSS('width', `${width}px`);
      await expect(frame.locator('[data-testid="mount-inset-body"] .head')).toHaveCSS('height', '24px');
      await expect(caption).toHaveText('');
      await expect(header).toHaveAttribute('title', `${mountLabel}: Sample A, Sample B`);
      const titleBox = await header.boundingBox(), bodyBox = await frame.getByTestId('mount-inset-body').boundingBox();
      expect(titleBox!.width).toBeGreaterThan(0); expect(titleBox!.y + titleBox!.height).toBeLessThanOrEqual(bodyBox!.y + 26);
    }
    await frame.getByTestId('mount-inset-close').click();
    const opener = frame.getByTestId('mount-inset-open');
    await expect(opener).toHaveText(/^Mount · Invented/); await expect(opener).toHaveAttribute('title', mountLabel);
    expect((await opener.boundingBox())!.width).toBeLessThanOrEqual(180);
    // Smallest SEM tile keeps the name available in the fixed overlay at any zoom.
    await page.locator('iframe').evaluate((el: HTMLIFrameElement) => { el.style.width = '320px'; el.style.height = '240px'; });
    await expect(opener).toBeVisible(); await opener.click(); await expect(header).toBeVisible();
    if (process.env.SCREENSHOTS === '1') await page.screenshot({ path: testInfo.outputPath(`viewer-region-small-${viewport.width}.png`) });
    expect(await savedCopies()).toEqual(storedBefore);
    expect(writes).toEqual([]); expect(unexpected).toEqual([]);
  });
}
