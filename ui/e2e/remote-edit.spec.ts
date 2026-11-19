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

async function mockServer(page: Page, onLevelZero = () => {}, unregistered = false, beforeLevelZero: () => Promise<void> = async () => {},
  quiet: { standalone?: boolean; failures?: number; bulk?: boolean; beforeReply?: () => Promise<void>; unresponsive?: boolean; staleCopy?: boolean; images?: boolean; beforePut?: () => Promise<void> } = {}) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui;
  const syncBodies: any[] = [];
  const syncRequests: { key: string | undefined; body: string }[] = [];
  const replies = new Map<string, object>();
  let failures = quiet.failures ?? 0;
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
  if (quiet.staleCopy) await page.addInitScript(({ project, elements, server, mountId }) => {
    const older = structuredClone(project);
    older.upbdb = { mount_id: null };
    older.grainatlas_ui.spots = [];
    const key = `grainatlas.project.${project.package.name}`;
    localStorage.setItem(key, JSON.stringify(older));
    localStorage.setItem('grainatlas.drafts', JSON.stringify([project.uuid]));
    localStorage.setItem(`grainatlas.draft.${project.uuid}`, JSON.stringify({
      uuid: project.uuid, name: project.name, server, mount: mountId, base_revision: 1, base: elements,
      synced: true, server_can_edit: true, pixels_base: null, source_keys: [key],
      updated_at: new Date().toISOString(), changes: { spots: elements.spots.length, grains: 0, labels: 0 },
      pending: { ...elements, spots: [] },
    }));
  }, { project, elements, server, mountId });
  await page.addInitScript(({ server, standalone }) => {
    const w = window as any;
    // Exercise the real hosted bridge command route while keeping all image work in wasm.
    if (!standalone) w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__commands = [];
    w.__events = [];
    if (!standalone) w.__TAURI_INTERNALS__ = { invoke: async (cmd: string, args: any) => {
      if (cmd === 'bridge_take_command') return w.__commands.shift() ?? null;
      if (cmd === 'bridge_notify') { w.__events.push(args.event); return null; }
      if (cmd === 'bridge_token') return 'synthetic-token';
      return null;
    } };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: true, quality: 100, nasRoot: '' }));
    sessionStorage.setItem('grainatlas.upbdb.token', 'synthetic-token');
  }, { server, standalone: quiet.standalone ?? false });
  const uploads: Map<string, Buffer>[] = [];
  const bundleRequests: { path: string; version: number }[] = [];
  let atlasVersion = 1;
  const databaseRequests: string[] = [];
  const assets = new Map<string, Buffer>();
  for (const f of atlas.files) assets.set(f.sha256, await readFile(path.join(generated, f.path)));
  const imagePuts: string[] = [], imageStarts: string[] = [], imageStatuses: string[] = [], imageCommits: string[] = [];
  const imageJobs = new Map<string, any>(), imageReplies = new Map<string, object>();
  await page.route(`${server}/**`, async route => {
    const request = route.request(), url = new URL(request.url()), pathname = url.pathname;
    const json = (value: unknown, status = 200) => route.fulfill({ status, json: value, headers: { 'access-control-allow-origin': '*' } });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (pathname.startsWith('/v1/')) {
      databaseRequests.push(pathname);
      // Leave the intercepted request pending: DNS resolves but no headers arrive.
      if (quiet.unresponsive) return;
    }
    if (pathname === '/v1/assets/check') return quiet.images ? json({ have: [] }) : json({ code: 'not_found' }, 404);
    if (quiet.images && pathname === `/v1/mounts/${mountId}/atlas/uploads`) {
      const key = request.headers()['idempotency-key'];
      imageStarts.push(key);
      const body = request.postDataJSON();
      const id = `synthetic-upload-${key}`;
      imageJobs.set(id, body.atlas);
      return json({ upload_id: id, expires_at: '2099-01-01T00:00:00Z', missing: [...new Set<string>(body.atlas.files.map((f: any) => f.sha256))].filter(h => !assets.has(h)) }, 201);
    }
    const imageRoute = /^\/v1\/atlas\/uploads\/([^/]+)(?:\/(blobs|commit)(?:\/(.+))?)?$/.exec(pathname);
    if (quiet.images && imageRoute) {
      const [, id, action, hash] = imageRoute;
      const manifest = imageJobs.get(id);
      if (!manifest) return json({ code: 'not_found' }, 404);
      if (action === 'blobs') {
        imagePuts.push(hash);
        const bytes = request.postDataBuffer()!;
        const { createHash } = await import('node:crypto');
        expect(createHash('sha256').update(bytes).digest('hex')).toBe(hash);
        assets.set(hash, bytes);
        await quiet.beforePut?.();
        return json({ stored: true }, 201).catch(() => {}); // Reload may close a held request.
      }
      const missing = [...new Set<string>(manifest.files.map((f: any) => f.sha256))].filter(h => !assets.has(h));
      if (!action) { imageStatuses.push(id); return json({ state: 'open', missing, received: manifest.files.filter((f: any) => assets.has(f.sha256)).length, total: manifest.files.length }); }
      const key = request.headers()['idempotency-key'];
      if (imageReplies.has(key)) return json(imageReplies.get(key));
      if (missing.length) return json({ code: 'incomplete', missing }, 409);
      imageCommits.push(key);
      const files = new Map<string, Buffer>(manifest.files.map((f: any) => [f.path, assets.get(f.sha256)!]));
      files.set('atlas.json', Buffer.from(JSON.stringify(manifest)));
      uploads.push(files); atlasVersion++;
      const reply = { version: atlasVersion, created: true, warnings: [] };
      imageReplies.set(key, reply);
      return json(reply);
    }
    if (await mockRevisionRead(route)) return;
    const session = mockSessionRead(request.method(), pathname);
    if (session !== undefined) return json(session);
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
      if (relative.includes('/0/')) { onLevelZero(); await beforeLevelZero(); }
      const requestedVersion = Number(url.searchParams.get('version') ?? atlasVersion);
      // Image files carry the bundle version (immutable caching); atlas.json and the sync check's
      // project.json ask for the latest bundle.
      if (relative.endsWith('.webp')) expect(url.searchParams.has('version')).toBe(true);
      expect(requestedVersion).toBeGreaterThan(0);
      expect(requestedVersion).toBeLessThanOrEqual(atlasVersion);
      bundleRequests.push({ path: relative, version: requestedVersion });
      const fileHeaders = { 'access-control-allow-origin': '*', 'access-control-expose-headers': 'X-Atlas-Version', 'X-Atlas-Version': String(requestedVersion) };
      const uploaded = requestedVersion > 1 ? uploads[requestedVersion - 2] : undefined;
      if (uploaded) {
        const body = uploaded.get(relative);
        if (!body) throw new Error(`Unexpected uploaded bundle path ${relative}`);
        return route.fulfill({ headers: fileHeaders, body, contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
      return route.fulfill({ headers: fileHeaders, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
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
      const key = request.headers()['idempotency-key'];
      syncRequests.push({ key, body: request.postData()! });
      if (failures-- > 0) return json({ detail: 'Synthetic relay unavailable' }, 503);
      if (quiet.bulk && body.spots.delete.length && !body.confirm_bulk) return json({ code: 'bulk_change', message: 'Confirm this change',
        confirm: { retireGrains: 0, grainsTotal: 2, deleteSpots: body.spots.delete.length, spotsTotal: elements.spots.length } }, 422);
      if (key && replies.has(key)) return route.fulfill({ status: 200, json: replies.get(key), headers: { 'access-control-allow-origin': '*', 'Idempotent-Replayed': 'true' } });
      if (!body.dry_run) for (const kind of ['spots', 'grains', 'labels'] as const) {
        const rows = new Map<string, any>(elements[kind].map((e: any) => [e.id, e] as [string, any]));
        for (const id of body[kind].delete) rows.delete(id);
        for (const upsert of body[kind].upsert) rows.set(upsert.id, { ...rows.get(upsert.id), ...upsert, project_uuid: projectId });
        elements[kind] = [...rows.values()] as any;
      }
      if (!body.dry_run) revision++;
      const reply = { ok: true, mount_id: mountId, project_uuid: projectId, mode: body.mode, previous_revision: revision-1, revision,
        applied: { spots: { created: 0, updated: body.spots.upsert.length, deleted: body.spots.delete.length, unchanged: 3 }, grains: { created: 0, adopted: 0, updated: 0, retired: 0, unchanged: 2 }, labels: { created: 0, updated: 0, retired: 0, unchanged: 1 } },
        ids: { spots: {}, grains: {}, labels: {} }, acquisitions_linked: 0, complete: true, warnings: [], rebased_over: 0, not_in_snapshot: { spots: [], grains: [], labels: [] } };
      if (key) replies.set(key, reply);
      await quiet.beforeReply?.();
      return json(reply);
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return route.fulfill({ status: 404, json: { detail: 'Unexpected synthetic endpoint' } });
  });
  return { project, atlas, syncBodies, syncRequests, unexpected, uploads, bundleRequests, createBodies, databaseRequests, imagePuts, imageStarts, imageStatuses, imageCommits };
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
  // Register is also offered in View; finish the edit transition before opening it.
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  await expect(page.getByTestId('edit-copy')).toBeEnabled();
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
  const { project, atlas, syncBodies, unexpected, bundleRequests } = await mockServer(page, () => levelZeroRequests++);
  const ui = project.grainatlas_ui;
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  const nav = page.getByRole('navigation', { name: 'Screens' });
  const area = nav.getByRole('button', { name: 'Area', exact: true });
  const layout = nav.getByRole('button', { name: 'Layout', exact: true });
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await expect(area).toBeDisabled(); await expect(layout).toBeDisabled();
  // This small synthetic area fits on Canvas at z=0. Wait for its visible requests
  // before measuring Edit; background loading still must wait for Area or Layout.
  await expect.poll(() => levelZeroRequests).toBe(atlas.files.filter((f: any) => f.path.includes('/0/')).length);
  const initial = levelZeroRequests;
  await page.getByTestId('edit-copy').click();
  await expect(area).toBeEnabled(); await expect(layout).toBeEnabled();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  expect(levelZeroRequests).toBe(initial);
  await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
  await area.click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
  await expect(page.getByText('Published image from the database. Stitching and shading correction need the raw package.')).toBeVisible();
  expect(bundleRequests.filter(r => r.path.includes('/0/')).map(r => r.path).sort()).toEqual(
    atlas.files.map((f: any) => f.path).filter((name: string) => name.includes('/0/')).sort());
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
  // Print export is one PDF now; the on-screen page is the same sheet SVG.
  const svg = await page.getByTestId('page').evaluate(async (img: HTMLImageElement) => (await fetch(img.src)).text());
  const pdf = page.waitForEvent('download');
  await page.getByTestId('export-pdf').click();
  expect((await pdf).suggestedFilename()).toMatch(/_print_\d{4}-\d{2}-\d{2}\.pdf$/);
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
  await expect(low).toBeEnabled();
  await expect(low).toHaveValue('0');
  await expect(histogram).toHaveAttribute('d', /^M0 90 L/);
  const originalHistogram = await histogram.getAttribute('d');
  await low.fill('12');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
  await expect(histogram).not.toHaveAttribute('d', originalHistogram!);
  await expect(chip).toHaveText('Saved');
  await expect(chip).toHaveAttribute('title', /New images not published/);

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
  await expect(chip).toHaveText('Up to date');
  await expect(chip).toHaveAttribute('title', /v2/);
  await expect(chip).not.toHaveAttribute('title', /New images not published/);
  await expect(low).toBeEnabled();
  await expect(low).toHaveValue('0');
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(histogram).toHaveAttribute('d', /^M0 90 L/);
  await expect(histogram).not.toHaveAttribute('d', originalHistogram!);
  const publishedHistogram = await histogram.getAttribute('d');
  const syncsBeforeSave = syncBodies.length;
  const saved = JSON.parse(await downloadText(page, () => page.getByTestId('save-project').click()));
  expect(saved.grainatlas_ui).not.toHaveProperty('bundle_tone');
  await expect.poll(() => syncBodies.length).toBe(syncsBeforeSave + 1);
  await expect(chip).toHaveText('Up to date');
  await expect(chip).toHaveAttribute('title', new RegExp(`v2 r${syncBodies.length + 1}`));
  await expect(chip).not.toHaveAttribute('title', /New images not published/);

  const beforeReload = bundleRequests.length;
  // The app drops the deep-link query once handled: open the same link again.
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await area.click();
  await expect(low).toBeEnabled();
  await expect(low).toHaveValue('0');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
  // Every z=0 tile was served from the uploaded bundle (the bake changed their hashes, so the
  // cache cannot answer for them); the decoded image matches the bake.
  await expect.poll(() => tiles.every(tile => bundleRequests.slice(beforeReload).some(r => r.path === tile && r.version === 2))).toBe(true);
  await expect(histogram).toHaveAttribute('d', publishedHistogram!);
  expect(bundleRequests.slice(beforeReload)).toEqual(expect.arrayContaining([
    { path: 'atlas.json', version: 2 }, { path: 'project.json', version: 2 },
  ]));
  await expect(chip).toHaveText('Up to date');
  await expect(chip).toHaveAttribute('title', /v2/);
  await expect(chip).not.toHaveAttribute('title', /New images not published/);
  expect(uploads).toHaveLength(1);
  expect(unexpected).toEqual([]);
});


test('published pixels load without a modal, can stop and retry, and survive a reload in cache', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let levelZeroRequests = 0;
  const { atlas, unexpected } = await mockServer(page, () => levelZeroRequests++, false, () => gate);
  const link = `/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`;
  await page.goto(link);
  const nav = page.getByRole('navigation', { name: 'Screens' });
  const area = nav.getByRole('button', { name: 'Area', exact: true });
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  // Leave the Canvas's visible requests held; Edit must not request the rest.
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect.poll(() => levelZeroRequests).toBe(atlas.files.filter((f: any) => f.path.includes('/0/')).length);
  const beforeEdit = levelZeroRequests;
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  expect(levelZeroRequests).toBe(beforeEdit);
  await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
  await page.evaluate(() => {
    const w = window as any;
    w.__pixelsBusyShown = false;
    new MutationObserver(records => {
      if (document.querySelector('[data-testid=busy]') || records.some(record => [...record.addedNodes].some(node =>
        node instanceof Element && (node.matches('[data-testid=busy]') || node.querySelector('[data-testid=busy]'))))) w.__pixelsBusyShown = true;
    }).observe(document.body, { childList: true, subtree: true });
  });
  await area.click();
  const status = page.getByTestId('pixels-loading');
  await expect(status).toContainText('Loading full-resolution images:');
  await expect(page.getByTestId('area-view')).toBeVisible();
  await expect(page.getByTestId('low-pct')).toBeDisabled();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  // Both screens keep navigation available while the image load is in flight.
  await nav.getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect(page.locator('.canvas canvas').first()).toBeVisible();
  await expect(page.getByTestId('tool-spot')).toBeEnabled();
  await nav.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(status).toBeVisible();
  await area.click();
  await status.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(status).toContainText('Stopped.');
  await expect(status.getByRole('button', { name: 'Retry', exact: true })).toBeEnabled();
  release();
  await status.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  await expect(status).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__pixelsBusyShown)).toBe(false);
  const beforeReload = levelZeroRequests;
  await page.goto(link);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await area.click();
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  await expect(status).toHaveCount(0);
  expect(levelZeroRequests - beforeReload).toBe(0);
  expect(unexpected).toEqual([]);
});


async function openQuietEdit(page: Page) {
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('tool-spot')).toBeEnabled();
}
async function placeQuietSpot(page: Page, project: any) {
  const before = await page.getByTestId('spot').count();
  const layers = page.locator('details.layers');
  await layers.evaluate((element: HTMLDetailsElement) => { element.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await page.getByTestId('side-collapse').click();
  await page.getByTestId('tool-spot').click();
  await page.getByRole('button', { name: '+', exact: true }).click();
  // Inside the first grain but clear of its existing spots (a click on a spot selects it).
  const [gx] = project.grainatlas_ui.grains.a[0].centroid_um;
  const target = [gx, 150];
  const point = () => page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, point: number[]) => {
    const p = new DOMPoint(point[0], point[1]).matrixTransform(g.getScreenCTM()!);
    const bounds = g.closest('[data-view]')!.getBoundingClientRect();
    return { x: p.x, y: p.y, cx: (bounds.left + bounds.right) / 2, cy: (bounds.top + bounds.bottom) / 2 };
  }, target);
  let p = await point();
  await page.mouse.move(p.cx, p.cy); await page.mouse.down({ button: 'middle' });
  await page.mouse.move(p.cx + p.cx - p.x, p.cy + p.cy - p.y, { steps: 5 }); await page.mouse.up({ button: 'middle' });
  p = await point(); await page.mouse.click(p.x, p.y);
  await expect(page.getByTestId('spot')).toHaveCount(before + 1);
}

test('an unanswered database goes offline within ten seconds; Save and deletion stay usable', async ({ page }) => {
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, unresponsive: true });
  const start = Date.now();
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('sync-chip')).toHaveText('Offline, will sync later', { timeout: 10000 });
  expect(Date.now() - start).toBeLessThan(10000);
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('tool-spot')).toBeEnabled();
  const download = page.waitForEvent('download', { timeout: 3000 });
  await page.getByTestId('save-project').click();
  await page.getByTestId('checks-continue').click();
  await download;
  expect(mock.databaseRequests).not.toContain('/v1/sessions');
  await expect(page.getByText(/Database checks skipped while saving/)).toBeVisible();
  const removed = mock.project.grainatlas_ui.spots[0];
  await page.locator(`tr[data-spot-id="${removed.id}"]`).click();
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('spot')).toHaveCount(mock.project.grainatlas_ui.spots.length - 1, { timeout: 1000 });
  expect(mock.syncRequests).toEqual([]);
});

test('Cancel stops an unanswered open-from-server request immediately', async ({ page }) => {
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true });
  const url = `${server}/atlas/projects/${projectId}/atlas.json`;
  await page.route(url, () => { /* Keep headers pending until Cancel aborts the request. */ });
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('busy')).toBeVisible();
  const failed = page.waitForEvent('requestfailed', { predicate: r => r.url() === url, timeout: 2000 });
  await page.getByTestId('busy').getByRole('button', { name: 'Cancel', exact: true }).click();
  await failed;
  await expect(page.getByTestId('busy')).toHaveCount(0, { timeout: 1000 });
  expect(mock.bundleRequests).toEqual([]);
});

test('an older copy waits for a choice and Bring them back prevents all spot deletions', async ({ page }) => {
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, staleCopy: true });
  await openQuietEdit(page);
  await expect(page.getByTestId('sync-chip')).toHaveText('Needs your attention');
  await expect(page.getByTestId('spot')).toHaveCount(0);
  await page.waitForTimeout(10000);
  expect(mock.syncRequests).toEqual([]);
  await page.getByTestId('sync-action').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading')).toHaveText('This copy is older than the shared project');
  await expect(dialog.getByRole('button').first()).toHaveText('Bring them back');
  await expect(dialog).toContainText(`${mock.project.grainatlas_ui.spots.length} spots and 0 grains`);
  await dialog.getByRole('button', { name: 'Bring them back', exact: true }).click();
  await expect(page.getByTestId('spot')).toHaveCount(mock.project.grainatlas_ui.spots.length);
  await placeQuietSpot(page, mock.project);
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 10000 });
  expect(mock.syncRequests.length).toBeGreaterThan(0);
  for (const body of mock.syncBodies) expect(body.spots.delete).toEqual([]);
  expect(mock.unexpected).toEqual([]);
});

test('quiet sync saves, syncs and settles with one idempotent write', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, beforeReply: () => gate });
  await openQuietEdit(page); await placeQuietSpot(page, mock.project);
  const chip = page.getByTestId('sync-chip');
  await expect(chip).toHaveText('Saved');
  await expect(chip).toHaveText('Syncing…', { timeout: 10000 });
  release(); await expect(chip).toHaveText('Up to date');
  expect(mock.syncRequests).toHaveLength(1);
  expect(mock.syncRequests[0].key).toMatch(/^[0-9a-f-]{36}$/);
  expect(mock.unexpected).toEqual([]);
});
test('quiet sync retries two 503 replies with the same key and body', async ({ page }) => {
  test.setTimeout(60000);
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, failures: 2 });
  await openQuietEdit(page); await placeQuietSpot(page, mock.project);
  await expect(page.getByTestId('sync-chip')).toHaveText('Offline, will sync later', { timeout: 10000 });
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 40000 });
  expect(mock.syncRequests).toHaveLength(3);
  expect(new Set(mock.syncRequests.map(r => r.key)).size).toBe(1);
  expect(new Set(mock.syncRequests.map(r => r.body)).size).toBe(1);
  expect(mock.syncRequests[0].key).toMatch(/^[0-9a-f-]{36}$/);
});
test('quiet sync leaves a bulk guard for a person and never confirms it', async ({ page }) => {
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, bulk: true });
  await openQuietEdit(page); await placeQuietSpot(page, mock.project);
  await page.getByTestId('side-expand').click();
  const removed = mock.project.grainatlas_ui.spots.slice(0, 2);
  await page.locator(`tr[data-spot-id="${removed[0].id}"]`).click();
  await page.locator(`tr[data-spot-id="${removed[1].id}"]`).click({ modifiers: ['Shift'] });
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('spot')).toHaveCount(2);
  await expect(page.getByTestId('sync-chip')).toHaveText('Needs your attention', { timeout: 10000 });
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button').first()).toHaveText('Keep them (recommended)');
  await expect(dialog.getByText(/^What happens to your spots:/)).toBeVisible();
  await page.evaluate(() => { window.dispatchEvent(new Event('online')); window.dispatchEvent(new Event('focus')); });
  expect(mock.syncRequests).toHaveLength(1);
  expect(mock.syncBodies[0].spots.delete.sort()).toEqual(removed.map((s: any) => s.id).sort());
  await dialog.getByRole('button', { name: 'Keep them (recommended)', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('spot')).toHaveCount(4);
  for (const spot of removed) await expect(page.locator(`tr[data-spot-id="${spot.id}"]`)).toBeVisible();
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 10000 });
  expect(mock.syncRequests).toHaveLength(2);
  for (const kind of ['spots', 'grains', 'labels']) expect(mock.syncBodies[1][kind].delete).toEqual([]);
  // The wire body sends grain retirement as grains.delete (checked above).
  expect(mock.syncBodies.every(body => !Object.hasOwn(body, 'confirm_bulk'))).toBe(true);
  expect(mock.unexpected).toEqual([]);
});
test('quiet sync recovers the exact pending request after reloading mid-reply', async ({ page }) => {
  let release!: () => void; let held = false;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, beforeReply: async () => { held = true; await gate; } });
  await openQuietEdit(page); await placeQuietSpot(page, mock.project);
  await expect.poll(() => held).toBe(true);
  expect(mock.syncRequests).toHaveLength(1);
  const first = mock.syncRequests[0];
  // A direct open returns to View; explicitly resume Edit so the read-only gate stays respected.
  page.on('dialog', dialog => { void dialog.accept(); });
  await openQuietEdit(page);
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 10000 });
  release();
  expect(mock.syncRequests).toHaveLength(2);
  expect(mock.syncRequests[1]).toEqual(first);
});


async function quietTone(page: Page) {
  await openQuietEdit(page);
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Area', exact: true }).click();
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  await page.getByTestId('low-pct').fill('12');
  await expect(page.getByTestId('area-view')).toHaveAttribute('data-live', 'idle');
}

test('quiet images upload only changed tiles after idle and settle without a modal', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, images: true, beforePut: () => gate });
  await quietTone(page);
  const chip = page.getByTestId('sync-chip');
  await expect(chip).toHaveText('Saved');
  await expect(chip).toHaveText('Syncing images…', { timeout: 20000 });
  await expect(chip).toHaveAttribute('title', /Syncing images….*files/);
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  release();
  await expect(chip).toHaveText('Up to date');
  expect(mock.imageStarts).toHaveLength(1); expect(mock.imageCommits).toHaveLength(1);
  const manifest = JSON.parse(mock.uploads[0].get('atlas.json')!.toString());
  const previous = new Map(mock.atlas.files.map((f: any) => [f.path, f.sha256]));
  const changed = [...new Set<string>(manifest.files.filter((f: any) => previous.get(f.path) !== f.sha256).map((f: any) => f.sha256))];
  expect([...mock.imagePuts].sort()).toEqual(changed.sort());
  expect(mock.syncRequests).toHaveLength(0); expect(mock.unexpected).toEqual([]);
});
test('quiet images resume a persisted upload after reload while a PUT is held and commit once', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true, images: true, beforePut: () => gate });
  await quietTone(page);
  await expect.poll(() => mock.imagePuts.length).toBeGreaterThan(0);
  // The first PUT starts only after the job ID has committed to the app's draft storage.
  page.on('dialog', dialog => { void dialog.accept(); });
  await openQuietEdit(page);
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Area', exact: true }).click();
  await expect(page.getByTestId('low-pct')).toHaveValue('12');
  release();
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 40000 });
  expect(mock.imageStarts).toHaveLength(1);
  expect(mock.imageStatuses.length).toBeGreaterThan(0);
  expect(mock.imageCommits).toHaveLength(1); expect(mock.uploads).toHaveLength(1);
  expect(mock.unexpected).toEqual([]);
});
test('quiet images fall back to one zip publish when the new routes are unavailable', async ({ page }) => {
  const mock = await mockServer(page, undefined, false, undefined, { standalone: true });
  await quietTone(page);
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date', { timeout: 30000 });
  expect(mock.uploads).toHaveLength(1); expect(mock.imagePuts).toHaveLength(0);
  expect(mock.unexpected).toEqual([]);
});
