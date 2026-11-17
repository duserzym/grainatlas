import { test, expect, type Page } from '@playwright/test';
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

async function command(page: Page, value: object) {
  await page.evaluate(value => {
    (window as any).__commands.push(value);
    window.dispatchEvent(new Event('lab-bridge:command'));
  }, value);
}

async function downloadText(page: Page, click: () => Promise<unknown>) {
  const download = page.waitForEvent('download');
  await click();
  const file = await (await download).path();
  if (!file) throw new Error('Download has no local path');
  return readFile(file, 'utf8');
}

// Read the core's STORE-only zip independently, stripping its <project>.atlas/ prefix.
function readStoredZip(zip: Buffer): Map<string, Buffer> {
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error('Atlas upload is not a zip');
  const files = new Map<string, Buffer>();
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  for (let i = 0; i < count; i++) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error('Invalid zip directory');
    const size = zip.readUInt32LE(at + 20);
    if (zip.readUInt16LE(at + 10) !== 0 || size !== zip.readUInt32LE(at + 24)) throw new Error('Atlas zip must use STORE');
    const nameLength = zip.readUInt16LE(at + 28);
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength);
    const prefix = `${projectId}.atlas/`;
    if (!name.startsWith(prefix)) throw new Error(`Unexpected zip entry ${name}`);
    const relative = name.slice(prefix.length);
    if (files.has(relative)) throw new Error(`Duplicate zip entry ${name}`);
    const local = zip.readUInt32LE(at + 42);
    if (zip.readUInt32LE(local) !== 0x04034b50) throw new Error('Invalid zip local header');
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    if (start + size > zip.length) throw new Error(`Truncated zip entry ${name}`);
    files.set(relative, zip.subarray(start, start + size));
    at += 46 + nameLength + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
  }
  return files;
}

async function mockServer(page: Page, onLevelZero = () => {}, unregistered = false) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  const syncBodies: any[] = [];
  const createBodies: any[] = [];
  const mounts = unregistered ? [] : [{ id: mountId, label: 'Synthetic mount', mount_type: 'epoxy_1in', grain_count: 2, spot_count: 3, can_edit: true }];
  const unexpected: string[] = [];
  let revision = 1;
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  let elements = {
    grains: ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', sample: 'Synthetic area', project_uuid: projectId, retired: false, merged_from: [] })),
    spots: ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, kind: 'spot', acquisition_ids: [] })),
    labels: ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  };
  await page.addInitScript(({ server }) => {
    const w = window as any;
    // Exercise the real hosted bridge command route while keeping all image work in wasm.
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__commands = [];
    w.__events = [];
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string, args: any) => {
      if (cmd === 'bridge_take_command') return w.__commands.shift() ?? null;
      if (cmd === 'bridge_notify') { w.__events.push(args.event); return null; }
      if (cmd === 'bridge_token') return 'synthetic-token';
      return null;
    } };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: true, quality: 100, nasRoot: '' }));
  }, { server });
  const uploads: Map<string, Buffer>[] = [];
  const bundleRequests: { path: string; version: number }[] = [];
  let atlasVersion = 1;
  await page.route(`${server}/**`, async route => {
    const request = route.request(), url = new URL(request.url()), pathname = url.pathname;
    const json = (value: unknown, status = 200) => route.fulfill({ status, json: value, headers: { 'access-control-allow-origin': '*' } });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (pathname === '/v1/mounts') {
      if (request.method() === 'GET') return json(mounts);
      if (request.method() === 'POST') {
        expect(request.headers().authorization).toBe('Bearer synthetic-token');
        const body = request.postDataJSON();
        createBodies.push(body);
        if (mounts.some(m => m.label.toLowerCase() === body.label.toLowerCase())) return json({ detail: 'A mount with this label already exists.', code: 'label_taken' }, 409);
        const mount = { id: '20000000-0000-4000-8000-000000000002', ...body, grain_count: 0, spot_count: 0, can_edit: true, sample_count: 0, projects: [], prepared_at: null, atlas: null };
        mounts.push(mount);
        return json(mount, 201);
      }
    }
    if (pathname === '/v1/atlas/projects') return json([]);
    if (unregistered && pathname === `/v1/atlas/projects/${projectId}`) return json({ detail: 'Project not registered.' }, 404);
    if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
      const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
      if (relative.includes('/0/')) onLevelZero();
      bundleRequests.push({ path: relative, version: atlasVersion });
      const uploaded = uploads.at(-1);
      if (uploaded) {
        const body = uploaded.get(relative);
        if (!body) throw new Error(`Unexpected uploaded bundle path ${relative}`);
        return route.fulfill({ headers: { 'access-control-allow-origin': '*' }, body, contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
      return route.fulfill({ headers: { 'access-control-allow-origin': '*' }, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
    }
    if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'Synthetic mount', name: project.name,
      package_name: project.package.name, atlas_version: atlasVersion, atlas_bundle_id: `bundle-${atlasVersion}`, revision, counts: { spots: 3, grains: 2, labels: 1, grains_retired: 0, grains_without_atlas_id: 0 },
      hashes, can_edit: true, current_project_uuid: projectId, project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
    if (pathname === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision, hashes, project_uuids: [projectId], current_project_uuid: projectId });
    if (pathname === '/v1/sessions') return json([]);
    if (pathname === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
    const resultsMount = mounts.find(m => pathname === `/v1/mounts/${m.id}/results`);
    if (resultsMount) return json({ mount_id: resultsMount.id, spots: [] });
    if (pathname === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision, since: 1, changes: [] });
    if (request.method() === 'POST' && pathname === `/v1/mounts/${mountId}/atlas`) {
      const body = request.postDataBuffer();
      if (!body) throw new Error('Atlas upload has no multipart body');
      const form = await new Response(new Uint8Array(body), { headers: { 'content-type': request.headers()['content-type'] } }).formData();
      const bundle = form.get('bundle');
      if (!bundle || typeof bundle === 'string') throw new Error('Atlas upload has no bundle file');
      uploads.push(readStoredZip(Buffer.from(await bundle.arrayBuffer())));
      atlasVersion++;
      return json({ mount_id: mountId, project_id: projectId, version: atlasVersion, created: true, warnings: [],
        spot_check: { in_bundle: 3, differ: 0, not_in_database: 0, only_in_database: 0, names: [] } });
    }
    if (pathname === `/v1/mounts/${mountId}/sync`) {
      const body = request.postDataJSON(); syncBodies.push(body);
      for (const kind of ['spots', 'grains', 'labels'] as const) {
        const upserts = body[kind].upsert;
        elements[kind] = elements[kind].map((e: any) => ({ ...e, ...upserts.find((u: any) => u.id === e.id) }));
      }
      revision++;
      return json({ ok: true, mount_id: mountId, project_uuid: projectId, mode: body.mode, previous_revision: revision-1, revision,
        applied: { spots: { created: 0, updated: 0, deleted: 0, unchanged: 3 }, grains: { created: 0, adopted: 0, updated: 0, retired: 0, unchanged: 2 }, labels: { created: 0, updated: 0, retired: 0, unchanged: 1 } },
        ids: { spots: {}, grains: {}, labels: {} }, acquisitions_linked: 0, complete: true, warnings: [], rebased_over: 0, not_in_snapshot: { spots: [], grains: [], labels: [] } });
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return route.fulfill({ status: 404, json: { detail: 'Unexpected synthetic endpoint' } });
  });
  return { project, atlas, syncBodies, unexpected, uploads, bundleRequests, createBodies };
}

test('Canvas creates and selects a mount, and keeps duplicate-label errors in the form', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const { project, createBodies, unexpected } = await mockServer(page);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('tab-link').click();
  const newMount = page.getByRole('button', { name: 'New mount…', exact: true });
  await expect(newMount).toBeDisabled();
  await page.getByTestId('edit-copy').click();
  await newMount.click();
  const form = page.getByRole('form', { name: 'New mount' });
  await expect(form.getByLabel('Label', { exact: true })).toHaveValue(project.name);
  await form.getByLabel('Label', { exact: true }).fill('Synthetic mount');
  await form.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(form.getByRole('alert')).toHaveText('Creating mount: A mount with this label already exists.');
  await form.getByLabel('Label', { exact: true }).fill('New synthetic mount');
  await form.getByTestId('mount-type').selectOption('grain_tape');
  await form.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByTestId('mount-select')).toHaveValue('20000000-0000-4000-8000-000000000002');
  await expect(form).toHaveCount(0);
  expect(createBodies).toEqual([
    { label: 'Synthetic mount', mount_type: 'epoxy_1in' },
    { label: 'New synthetic mount', mount_type: 'grain_tape' },
  ]);
  expect(unexpected).toEqual([]);
});

test('registration creates and selects a mount when the mount list is empty', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  const { createBodies, unexpected } = await mockServer(page, () => {}, true);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('sync-action')).toHaveText('Register…');
  await page.getByTestId('sync-action').click();
  const dialog = page.getByRole('dialog', { name: 'Register project' });
  await expect(dialog.getByTestId('mount-note')).toHaveText('No mounts yet: create one.');
  await expect(dialog.getByRole('button', { name: 'Register and sync', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'New mount…', exact: true }).click();
  await dialog.getByLabel('Label', { exact: true }).fill('New registration mount');
  await dialog.getByTestId('mount-type').selectOption('thin_section');
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(dialog.getByTestId('mount-select')).toHaveValue('20000000-0000-4000-8000-000000000002');
  await expect(dialog.getByRole('button', { name: 'Register and sync', exact: true })).toBeEnabled();
  expect(createBodies).toEqual([{ label: 'New registration mount', mount_type: 'thin_section' }]);
  expect(unexpected).toEqual([]);
});

test('published images support tone, sheets, save, sync and returning to view', async ({ page }) => {
  let levelZeroRequests = 0;
  const { project, atlas, syncBodies, unexpected } = await mockServer(page, () => levelZeroRequests++);
  const ui = project.grainatlas_ui;
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  const nav = page.getByRole('navigation', { name: 'Screens' });
  const area = nav.getByRole('button', { name: 'Area', exact: true });
  const layout = nav.getByRole('button', { name: 'Layout', exact: true });
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await expect(area).toBeDisabled(); await expect(layout).toBeDisabled();
  // Canvas may fetch z=0 for drawing; entering Edit alone must not fetch all images.
  const initial = levelZeroRequests;
  await page.getByTestId('edit-copy').click();
  await expect(area).toBeEnabled(); await expect(layout).toBeEnabled();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  await area.click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.getByText('Published image from the database. Stitching and shading correction need the raw package.')).toBeVisible();
  expect(levelZeroRequests).toBeGreaterThan(initial);
  const histogram = page.locator('svg.hist path').first();
  await expect(histogram).toHaveAttribute('d', /^M0 90 L/);
  const before = await histogram.getAttribute('d');
  const mosaicBefore = await page.getByTestId('area-view').locator('canvas').first().evaluate((c: HTMLCanvasElement) => c.toDataURL());
  await page.getByTestId('low-pct').fill('12');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
  await expect(histogram).not.toHaveAttribute('d', before!);
  await expect.poll(() => page.getByTestId('area-view').locator('canvas').first().evaluate((c: HTMLCanvasElement) => c.toDataURL())).not.toBe(mosaicBefore);
  await layout.click();
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.locator('.page')).toBeVisible();
  const svg = await downloadText(page, () => page.getByRole('button', { name: /SVG/ }).click());
  // Sheets carry the mosaic and sample legend; spots are drawn by hand at the laser.
  expect(svg).toContain('<image');
  expect(svg).toContain('Synthetic area');
  for (const spot of ui.spots) expect(svg).not.toContain(spot.id);
  const download = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  // Invented overlapping spots and missing acquisition labels produce advisory warnings.
  await expect(page.getByTestId('checks-continue')).toBeVisible();
  await page.getByTestId('checks-continue').click();
  const savedPath = await (await download).path();
  const saved = JSON.parse(await readFile(savedPath!, 'utf8'));
  expect(saved.areas[0].params.tone).toEqual(project.areas[0].params.tone);
  expect(saved.grainatlas_ui.tone.a).toEqual(project.grainatlas_ui.tone.a);
  expect(saved.grainatlas_ui.bundle_tone.a.curve.low_pct).toBe(12);
  // This synthetic bundle publishes its input bytes unchanged, so this also checks tile decode and assembly bit for bit.
  expect(saved.grainatlas_ui.bundle_tone.a.on_sha256).toBe(atlas.areas[0].mosaic_sha256);
  expect(saved.grainatlas_ui.spots.map((s: any) => [s.id,s.name])).toEqual(ui.spots.map((s: any) => [s.id,s.name]));
  expect(saved.grainatlas_ui.grains.a.map((g: any) => [g.id,g.label])).toEqual(ui.grains.a.map((g: any) => [g.id,g.label]));
  expect(saved.grainatlas_ui.labels.map((l: any) => [l.id,l.text])).toEqual(ui.labels.map((l: any) => [l.id,l.text]));
  expect(saved.upbdb.mount_id).toBe(mountId);
  await expect(page.getByRole('dialog', { name: 'Sync to database' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Publish a new atlas version with these images' }).uncheck();
  await page.getByRole('dialog').getByRole('button', { name: 'Sync to database', exact: true }).click();
  await expect.poll(() => syncBodies.length).toBeGreaterThan(0);
  for (const body of syncBodies) {
    for (const spot of body.spots.upsert) expect(ui.spots.some((s: any) => s.id === spot.id && s.name === spot.name)).toBe(true);
    for (const grain of body.grains.upsert) expect(ui.grains.a.some((g: any) => g.id === grain.id && g.label === grain.label)).toBe(true);
    expect(body.spots.delete).toEqual([]); expect(body.grains.delete).toEqual([]);
    expect(body.project_uuid).toBe(projectId);
  }
  await command(page, { type: 'set-mode', project: projectId, mode: 'view', id: 'view-again' });
  await expect(area).toBeDisabled(); await expect(layout).toBeDisabled();
  await expect(nav.getByRole('button', { name: 'Canvas', exact: true })).toHaveAttribute('aria-pressed', 'true');
  expect(unexpected).toEqual([]);
});

test('publishing a tone change bakes the overlay and reopens the new images without pixel changes', async ({ page }) => {
  const { project, atlas, syncBodies, uploads, bundleRequests, unexpected } = await mockServer(page);
  const ui = project.grainatlas_ui;
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  const area = page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Area', exact: true });
  const low = page.getByTestId('low-pct');
  const chip = page.getByTestId('sync-chip');
  const histogram = page.locator('svg.hist path').first();
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await area.click();
  await expect(low).toHaveValue('0');
  await expect(histogram).toHaveAttribute('d', /^M0 90 L/);
  const originalHistogram = await histogram.getAttribute('d');
  await low.fill('12');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
  await expect(histogram).not.toHaveAttribute('d', originalHistogram!);
  await expect(chip).toContainText('New images not published');

  await page.getByTestId('sync-action').click();
  const dialog = page.getByRole('dialog', { name: 'Sync to database' });
  await expect(dialog).toBeVisible();
  const publishImages = dialog.getByRole('checkbox', { name: 'Publish a new atlas version with these images' });
  await publishImages.check();
  await expect(publishImages).toBeChecked();
  await dialog.getByRole('button', { name: 'Sync to database', exact: true }).click();
  // Publish always offers the synthetic project's advisory checks, even after sync.
  await expect(page.getByTestId('checks-continue')).toBeVisible();
  await page.getByTestId('checks-continue').click();
  await expect.poll(() => uploads.length).toBe(1);
  const files = uploads[0];
  const published = JSON.parse(files.get('project.json')!.toString('utf8'));
  expect(published.areas[0].params.tone).toEqual(project.areas[0].params.tone);
  expect(published.grainatlas_ui.tone.a).toEqual(ui.tone.a);
  expect(published.grainatlas_ui).not.toHaveProperty('bundle_tone');
  expect(published.grainatlas_ui.spots.map((s: any) => [s.id, s.name])).toEqual(ui.spots.map((s: any) => [s.id, s.name]));
  expect(published.grainatlas_ui.grains.a.map((g: any) => [g.id, g.label])).toEqual(ui.grains.a.map((g: any) => [g.id, g.label]));
  expect(published.grainatlas_ui.labels.map((l: any) => [l.id, l.text])).toEqual(ui.labels.map((l: any) => [l.id, l.text]));
  expect(published.upbdb.mount_id).toBe(mountId);
  const tiles: string[] = atlas.files.map((f: any) => f.path).filter((name: string) => /\/0\/[^/]+\.webp$/.test(name));
  expect(tiles.length).toBeGreaterThan(0);
  for (const tile of tiles) {
    expect(files.has(tile)).toBe(true);
    expect(files.get(tile)!.equals(await readFile(path.join(generated, tile)))).toBe(false);
  }

  // v2 means lookup has refreshed after the uploaded project acknowledged pixels.
  await expect(chip).toContainText('In sync · v2');
  await expect(chip).not.toContainText('New images not published');
  await expect(low).toHaveValue('0');
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(histogram).toHaveAttribute('d', /^M0 90 L/);
  await expect(histogram).not.toHaveAttribute('d', originalHistogram!);
  const publishedHistogram = await histogram.getAttribute('d');
  const syncsBeforeSave = syncBodies.length;
  const saved = JSON.parse(await downloadText(page, () => page.getByTestId('save-project').click()));
  expect(saved.grainatlas_ui).not.toHaveProperty('bundle_tone');
  await expect.poll(() => syncBodies.length).toBe(syncsBeforeSave + 1);
  await expect(chip).toContainText(`In sync · v2 r${syncBodies.length + 1}`);
  await expect(chip).not.toContainText('New images not published');

  const beforeReload = bundleRequests.length;
  // The app drops the deep-link query once handled: open the same link again.
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await area.click();
  await expect(low).toHaveValue('0');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
  // Every z=0 tile was served from the uploaded bundle; the decoded image matches the bake.
  await expect.poll(() => tiles.every(tile => bundleRequests.slice(beforeReload).some(r => r.path === tile && r.version === 2))).toBe(true);
  await expect(histogram).toHaveAttribute('d', publishedHistogram!);
  expect(bundleRequests.slice(beforeReload)).toEqual(expect.arrayContaining([
    { path: 'atlas.json', version: 2 }, { path: 'project.json', version: 2 },
  ]));
  await expect(chip).toContainText('In sync · v2');
  await expect(chip).not.toContainText('New images not published');
  expect(uploads).toHaveLength(1);
  expect(unexpected).toEqual([]);
});
