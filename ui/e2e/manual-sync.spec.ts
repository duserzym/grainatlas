// Synthetic records and mocked transports only. The supervisor runs this spec.
import { test, expect, mockRevisionRead, mockSessionRead } from './mock-session';
import type { Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
const server = 'https://manual-sync.example.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const mountId = '20000000-0000-4000-8000-000000000001';
const link = `/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`;

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' });
});

interface Options { automatic?: boolean; legacy?: boolean; desktop?: boolean; mountCount?: number }
async function setup(page: Page, options: Options = {}) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  let elements: Record<string, any[]> = {
    grains: project.grainatlas_ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', sample: project.areas[0].name, project_uuid: projectId, retired: false, merged_from: [] })),
    spots: project.grainatlas_ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, kind: 'spot', acquisition_ids: [] })),
    labels: project.grainatlas_ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  };
  let revision = 1, offline = false, refuseSync = false;
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const calls: { method: string; path: string; query: string; at: number }[] = [];
  const syncs: any[] = [], unexpected: string[] = [];
  await page.addInitScript(({ server, options }) => {
    if (!sessionStorage.getItem('manual-sync-fixture')) {
      sessionStorage.setItem('manual-sync-fixture', '1');
      localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, token: '', syncOnSave: false,
        syncAutomatically: options.automatic ?? false, syncPollSeconds: 120,
        ...(!options.legacy ? { syncModeVersion: 1 } : {}), quality: 100, nasRoot: '' }));
      sessionStorage.setItem('grainatlas.upbdb.token', 'synthetic-token');
    }
    if (!options.desktop) return;
    // Exercise the real native quit listener, retaining a browser-rendered UI and
    // replacing only native IPC. No production test hooks or app process are used.
    const w = window as any;
    const callbacks = new Map<number, (value: any) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    const files = new Map<string, string>();
    let sequence = 0;
    w.__nativeFiles = files;
    w.__finishedQuit = 0;
    w.__quitGuard = false;
    w.__emitNative = (event: string) => {
      for (const [id, item] of listeners) if (item.event === event) callbacks.get(item.handler)?.({ event, id, payload: null });
    };
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event: string, id: number) => listeners.delete(id) };
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback(callback: (value: any) => void) { const id = ++sequence; callbacks.set(id, callback); return id; },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke: async (command: string, args: any = {}) => {
        if (command === 'plugin:event|listen') { const id = ++sequence; listeners.set(id, args); return id; }
        if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return null; }
        if (command === 'plugin:app|version') return '0.1.14';
        if (command === 'startup') return { paths: [], stitch: false, segment: false, um_per_px: null, field_um: null, profile: null };
        if (command === 'take_opened') return [];
        if (command === 'enable_quit_guard') { w.__quitGuard = true; return null; }
        if (command === 'finish_quit') { w.__finishedQuit++; return null; }
        if (command === 'store_list') return { path: '/synthetic/local-outbox', keys: [...files.keys()] };
        if (command === 'store_read') return files.get(args.key) ?? null;
        if (command === 'store_write') { files.set(args.key, args.contents); return null; }
        if (command === 'store_remove') { files.delete(args.key); return null; }
        if (command === 'update_supported') return false;
        if (command === 'ownership') return '{}';
        if (command === 'check_project') return '[]';
        // blank/load_project/close_tab, window zoom and bridge-token fallback do
        // not need a native result for remote element-only editing.
        return null;
      },
    };
  }, { server, options });

  await page.route(`${server}/**`, async route => {
    const request = route.request(), url = new URL(request.url()), pathname = url.pathname;
    const headers = { 'access-control-allow-origin': '*', 'access-control-expose-headers': 'ETag, X-Next-Cursor, X-Total-Count, X-Atlas-Version', 'X-Atlas-Version': '1' };
    const json = (value: unknown, status = 200, extra = {}) => route.fulfill({ status, json: value, headers: { ...headers, ...extra } });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...headers, 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    calls.push({ method: request.method(), path: pathname, query: url.search, at: performance.now() });
    if (offline) return route.abort('connectionrefused');
    if (await mockRevisionRead(route, [{ project_uuid: projectId, mount_id: mountId, revision }])) return;
    const session = mockSessionRead(request.method(), pathname);
    if (session !== undefined) return json(session);
    if (pathname === '/v1/mounts') {
      expect(url.searchParams.get('limit')).toBe('50');
      const count = options.mountCount ?? 1, first = Number(url.searchParams.get('after') ?? 0);
      const rows = Array.from({ length: Math.min(50, count - first) }, (_, offset) => ({
        id: first + offset === 0 ? mountId : `20000000-0000-4000-8000-${String(first + offset + 1).padStart(12, '0')}`,
        label: `Synthetic mount ${first + offset + 1}`, mount_type: 'epoxy_1in', can_edit: true,
        grain_count: 2, spot_count: 3, sample_names: ['Synthetic'], sample_count: 1,
        role: 'write', content_revision: revision,
        atlas: { project_id: projectId, thumbnail: null, version: 1 },
      }));
      const etag = `"synthetic-page-${first}"`;
      const extra = { ETag: etag, 'X-Total-Count': String(count), ...(first + rows.length < count ? { 'X-Next-Cursor': String(first + rows.length) } : {}) };
      if (request.headers()['if-none-match'] === etag) return route.fulfill({ status: 304, headers: { ...headers, ...extra } });
      return json(rows, 200, extra);
    }
    if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'Synthetic mount 1', name: project.name,
      package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'synthetic-bundle', revision, hashes, can_edit: true,
      counts: { spots: elements.spots.length, grains: elements.grains.length, labels: elements.labels.length, grains_retired: 0, grains_without_atlas_id: 0 },
      current_project_uuid: projectId, project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
    if (pathname === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision, hashes, project_uuids: [projectId], current_project_uuid: projectId });
    if (pathname === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
    if (pathname === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision, since: revision, changes: [] });
    if (pathname === `/v1/mounts/${mountId}/sync` && request.method() === 'POST') {
      const body = request.postDataJSON(); syncs.push(body);
      if (refuseSync) return json({ detail: 'Synthetic database maintenance' }, 503);
      const applied: Record<string, object> = {};
      for (const kind of ['spots', 'grains', 'labels']) {
        const old = new Map(elements[kind].map(row => [row.id, row]));
        const created = body[kind].upsert.filter((row: any) => !old.has(row.id)).length;
        for (const id of body[kind].delete) old.delete(id);
        for (const row of body[kind].upsert) old.set(row.id, { ...old.get(row.id), ...row, project_uuid: projectId });
        if (!body.dry_run) elements[kind] = [...old.values()];
        applied[kind] = { created, updated: body[kind].upsert.length - created, deleted: body[kind].delete.length, retired: body[kind].delete.length, adopted: 0, unchanged: 0 };
      }
      const previous_revision = revision;
      if (!body.dry_run) revision++;
      return json({ ok: true, mode: body.mode, revision, previous_revision, applied, complete: true,
        not_in_snapshot: { spots: [], grains: [], labels: [] }, warnings: [], rebased_over: 0, dry_run: !!body.dry_run });
    }
    if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
      const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
      if (relative === 'atlas.json') return json(atlas);
      if (!atlas.files.some((file: any) => file.path === relative)) throw new Error(`Unexpected synthetic asset ${relative}`);
      return route.fulfill({ headers, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return json({ detail: 'Unexpected synthetic endpoint' }, 404);
  });
  return { project, calls, syncs, unexpected, offline(value = true) { offline = value; }, refuse(value = true) { refuseSync = value; } };
}

async function openEdit(page: Page, options: Options = {}) {
  const mock = await setup(page, options);
  await page.goto(link);
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('tool-spot')).toBeEnabled();
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date');
  await page.waitForLoadState('networkidle');
  return mock;
}
async function edit(page: Page, project: any) {
  const before = await page.getByTestId('spot').count();
  await page.locator('details.layers').evaluate((element: HTMLDetailsElement) => { element.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await page.getByTestId('side-collapse').click();
  await page.getByTestId('tool-spot').click();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  const target = [project.grainatlas_ui.grains.a[0].centroid_um[0], 150];
  const point = () => page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, point: number[]) => {
    const p = new DOMPoint(...point as [number, number]).matrixTransform(g.getScreenCTM()!);
    const bounds = g.closest('[data-view]')!.getBoundingClientRect();
    return { x: p.x, y: p.y, cx: (bounds.left + bounds.right) / 2, cy: (bounds.top + bounds.bottom) / 2 };
  }, target);
  let p = await point();
  await page.mouse.move(p.cx, p.cy); await page.mouse.down({ button: 'middle' });
  await page.mouse.move(p.cx + p.cx - p.x, p.cy + p.cy - p.y, { steps: 5 }); await page.mouse.up({ button: 'middle' });
  p = await point(); await page.mouse.click(p.x, p.y);
  await expect(page.getByTestId('spot')).toHaveCount(before + 1);
}
async function closeTab(page: Page) {
  await page.getByTestId('tab').filter({ has: page.locator('[data-testid="tab-dirty"]') }).first().getByRole('button', { name: /^Close / }).click();
  const dialog = page.getByTestId('close-tab-dialog');
  await expect(dialog).toContainText(/\d+ changes? not synced/);
  await expect(dialog).toContainText('saved locally in the outbox');
  return dialog;
}
async function storedDraft(page: Page, desktop = false) {
  return page.evaluate(async ({ projectId, desktop }) => {
    const key = `grainatlas.draft.${projectId}`;
    if (desktop) return JSON.parse(JSON.parse((window as any).__nativeFiles.get(key)).value);
    return new Promise<any>((resolve, reject) => {
      const open = indexedDB.open('grainatlas');
      open.onerror = () => reject(open.error);
      open.onsuccess = () => {
        const db = open.result, read = db.transaction('items').objectStore('items').get(key);
        read.onerror = () => { db.close(); reject(read.error); };
        read.onsuccess = () => { db.close(); resolve(read.result ? JSON.parse(read.result) : null); };
      };
    });
  }, { projectId, desktop });
}
async function quit(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as any).__quitGuard)).toBe(true);
  await page.evaluate(() => (window as any).__emitNative('grainatlas://quit-requested'));
  const dialog = page.getByRole('alertdialog', { name: 'Quit GrainAtlas?' });
  await expect(dialog).toContainText(/\d+ changes? not synced/);
  return dialog;
}

for (const viewport of [{ width: 1280, height: 800 }, { width: 1440, height: 900 }]) {
  test(`held outline repair remains reachable after reopen with Show and undo at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const mock = await setup(page);
    await page.addInitScript(({ project, server, mountId }) => {
      if (sessionStorage.getItem('synthetic-held-repair-seeded')) return;
      sessionStorage.setItem('synthetic-held-repair-seeded', '1');
      const copy = structuredClone(project), ui = copy.grainatlas_ui;
      const original = structuredClone(ui.grains.a[0]);
      const replacement = { ...original, id: '30000000-0000-4000-8000-000000000099', number: 99, label: 'a/99' };
      ui.grains.a[0] = { ...original, polygon_um: original.polygon_um.map(([x, y]: number[]) => [x + 10000, y]), centroid_um: [original.centroid_um[0] + 10000, original.centroid_um[1]] };
      ui.grains.a.push(replacement);
      const base = {
        grains: project.grainatlas_ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', project_uuid: project.uuid })),
        spots: project.grainatlas_ui.spots.map((s: any) => ({ ...s, project_uuid: project.uuid })),
        labels: project.grainatlas_ui.labels.map((l: any) => ({ ...l, project_uuid: project.uuid })),
      };
      const key = `grainatlas.project.${project.package.name}`;
      localStorage.setItem(key, JSON.stringify(copy));
      localStorage.setItem('grainatlas.drafts', JSON.stringify([project.uuid]));
      localStorage.setItem(`grainatlas.draft.${project.uuid}`, JSON.stringify({
        uuid: project.uuid, name: project.name, server, mount: mountId, base_revision: 1, base,
        synced: true, server_can_edit: true, pixels_base: null, pixels_changed: false, source_keys: [key],
        updated_at: new Date().toISOString(), changes: { spots: 0, grains: 2, labels: 0 },
        pending: { grains: ui.grains.a.map((g: any) => ({ ...g, area_id: 'a' })), spots: ui.spots, labels: ui.labels },
        held_back: [{ kind: 'grain', id: original.id, op: 'upsert', code: 'spot_outside_grain',
          reason: 'Synthetic registered spot lies outside the edited outline.',
          related: ui.spots.filter((s: any) => s.grain_id === original.id).map((s: any) => ({ kind: 'spot', id: s.id, label: s.name })) }],
      }));
    }, { project: mock.project, server, mountId });
    await page.goto(link); await page.getByTestId('edit-copy').click();
    await expect(page.getByTestId('sync-chip')).toHaveText('Needs your attention');
    await page.getByTestId('review-issues').focus(); await page.keyboard.press('Enter');
    await expect(page.getByTestId('sync-details')).toBeFocused();
    await expect(page.getByTestId('editing-issue-count')).toContainText(/\d+ issues? to review/);
    const held = page.getByTestId('held-items');
    await expect(held.getByTestId('held-reason')).toContainText('Synthetic registered spot');
    await held.getByRole('button', { name: 'Show spot', exact: true }).first().click();
    await expect(page.getByTestId('canvas-um-space')).toBeVisible();
    await expect(page.getByRole('application', { name: 'Mosaic canvas' })).toBeFocused();
    await expect(page.getByTestId('sync-details')).toBeHidden();
    await expect(page.getByTestId('review-issues')).toBeVisible();
    await expect(page.getByTestId('sync-action')).toBeVisible();
    await page.getByTestId('review-issues').click();
    await held.getByTestId('held-keep').click();
    await expect.poll(async () => (await storedDraft(page))?.pending?.grains.find((g: any) => g.id === mock.project.grainatlas_ui.grains.a[0].id)?.polygon_um[0][0]).toBe(mock.project.grainatlas_ui.grains.a[0].polygon_um[0][0]);
    await page.getByRole('button', { name: 'Undo', exact: true }).click();
    // Canvas actions leave the review panel. Return from the persistent header.
    await page.getByTestId('review-issues').click();
    const repair = held.getByTestId('spot-link-repair').first();
    await expect(repair).toBeVisible();
    await repair.getByRole('combobox').selectOption('30000000-0000-4000-8000-000000000099');
    await repair.getByRole('button', { name: 'Show grain', exact: true }).click();
    await expect(page.getByRole('application', { name: 'Mosaic canvas' })).toBeFocused();
    await page.getByTestId('review-issues').focus(); await page.keyboard.press('Enter');
    await expect(repair.getByRole('combobox')).toHaveValue('30000000-0000-4000-8000-000000000099');
    await repair.getByRole('button', { name: 'Assign same spot', exact: true }).click();
    await expect.poll(async () => (await storedDraft(page))?.pending?.spots.some((s: any) => s.grain_id === '30000000-0000-4000-8000-000000000099')).toBe(true);
    expect(mock.syncs).toHaveLength(0);
    // Reopen the same project in a new document to exercise persisted recovery.
    await page.goto(link);
    await page.getByTestId('edit-copy').click({ timeout: 20000 }); await page.getByTestId('sync-chip').click();
    await expect(page.getByTestId('held-reason')).toContainText('Synthetic registered spot');
    await expect(page.getByTestId('spot-link-audit')).toBeVisible();
    expect(mock.syncs).toHaveLength(0); expect(mock.unexpected).toEqual([]);
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('review-issues')).toBeFocused();
    await page.keyboard.press('Enter');
    await page.getByTestId('sync-details').getByRole('button', { name: 'Back to canvas', exact: true }).click();
    await expect(page.getByRole('application', { name: 'Mosaic canvas' })).toBeFocused();
    mock.refuse(); await page.getByTestId('sync-action').click();
    const failure = page.getByRole('alert').filter({ hasText: 'The database had a problem. Try again later.' });
    await failure.getByRole('button', { name: 'Review issues', exact: true }).click({ timeout: 20000 });
    await expect(page.getByTestId('sync-details')).toBeFocused();
    await expect(page.getByTestId('sync-action')).toBeEnabled();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });

  test(`manual edits persist without upload, polling or wake requests for five minutes at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const mock = await openEdit(page);
    await page.clock.install();
    await edit(page, mock.project);
    await expect(page.getByTestId('sync-chip')).toHaveText('1 change not synced');
    await expect(page.getByTestId('tab-dirty')).toHaveAttribute('aria-label', 'not synced');
    const before = mock.calls.length;
    await page.evaluate(() => { window.dispatchEvent(new Event('online')); window.dispatchEvent(new Event('focus')); document.dispatchEvent(new Event('visibilitychange')); });
    await page.clock.runFor(300_000);
    expect(mock.calls).toHaveLength(before);
    expect(mock.syncs).toHaveLength(0);
    await expect.poll(async () => (await storedDraft(page))?.pending?.spots.length).toBe(mock.project.grainatlas_ui.spots.length + 1);
    await page.getByTestId('sync-action').click();
    await expect.poll(() => mock.syncs.length).toBe(1);
    await expect(page.getByTestId('sync-chip')).toHaveText('Up to date');
    expect(mock.unexpected).toEqual([]);
  });
}

test('existing automatic preferences migrate once and a later explicit opt-in survives reload', async ({ page }) => {
  const mock = await setup(page, { automatic: true, legacy: true });
  await page.goto('/');
  const settings = page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Settings', exact: true });
  await settings.click();
  const automatic = page.getByTestId('sync-automatically');
  await expect(automatic).not.toBeChecked();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.upbdb')!).syncModeVersion)).toBe(1);
  await automatic.check();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.upbdb')!).syncAutomatically)).toBe(true);
  await page.reload();
  await settings.click();
  await expect(page.getByTestId('sync-automatically')).toBeChecked();
  expect(mock.unexpected).toEqual([]);
});

test('3,000 mounts stay paged and the manual start screen does not repeat requests over five minutes', async ({ page }) => {
  const mock = await setup(page, { mountCount: 3000 });
  const start = performance.now();
  await page.goto('/');
  await expect(page.getByTestId('mount-tile')).toHaveCount(50);
  await page.waitForLoadState('networkidle');
  const firstLoadMs = performance.now() - start, before = mock.calls.length;
  await page.clock.install();
  await page.evaluate(() => { window.dispatchEvent(new Event('online')); window.dispatchEvent(new Event('focus')); });
  await page.clock.runFor(300_000);
  expect(mock.calls).toHaveLength(before);
  expect(mock.calls.filter(call => call.path === '/v1/mounts')).toHaveLength(1);
  await page.getByRole('button', { name: 'Show more', exact: true }).click();
  await expect(page.getByTestId('mount-tile')).toHaveCount(100);
  expect(mock.calls.filter(call => call.path === '/v1/mounts')).toHaveLength(2);
  expect(mock.calls.filter(call => call.path === '/v1/mounts')[1].query).toContain('after=50');
  await page.getByRole('button', { name: 'Refresh', exact: true }).first().click();
  await expect.poll(() => mock.calls.filter(call => call.path === '/v1/mounts').length).toBe(3);
  console.info(JSON.stringify({ firstLoadMs: Math.round(firstLoadMs), rowsAvailable: 3000, firstPageRows: 50, idleMinutes: 5, idleRequests: mock.calls.length - before - 2 }));
  expect(mock.unexpected).toEqual([]);
});

test('start-screen Refresh immediately revalidates recent project revisions', async ({ page }) => {
  const mock = await setup(page);
  await page.addInitScript(({ projectId, mountId, server }) => {
    localStorage.setItem('grainatlas.recent', JSON.stringify([{ id: projectId, name: 'Synthetic local', when: '2026-01-01T00:00:00Z', areas: 1, spots: 0 }]));
    localStorage.setItem('grainatlas.drafts', JSON.stringify([projectId]));
    localStorage.setItem(`grainatlas.draft.${projectId}`, JSON.stringify({ uuid: projectId, name: 'Synthetic local', server, mount: mountId,
      base_revision: 1, base: { grains: [], spots: [], labels: [] }, synced: true, pixels_base: null,
      updated_at: '2026-01-01T00:00:00Z', changes: { spots: 0, grains: 0, labels: 0 } }));
  }, { projectId, mountId, server });
  await page.goto('/');
  const revisions = () => mock.calls.filter(call => call.path === '/v1/atlas/projects/revisions').length;
  await expect.poll(revisions).toBe(1);
  await page.getByRole('button', { name: 'Refresh', exact: true }).first().click();
  await expect.poll(revisions).toBe(2);
  expect(mock.unexpected).toEqual([]);
});

test('Sync automatically restores edit scheduling and configurable background checks', async ({ page }) => {
  const mock = await openEdit(page);
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByTestId('sync-automatically').check();
  const interval = page.getByLabel('Background check interval (seconds)');
  await interval.fill('30'); await interval.blur();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Canvas', exact: true }).click();
  await page.clock.install();
  await edit(page, mock.project);
  await page.clock.runFor(3001);
  await expect.poll(() => mock.syncs.length).toBe(1);
  await expect(page.getByTestId('sync-chip')).toHaveText('Up to date');
  const before = mock.calls.filter(call => call.path === `/v1/atlas/projects/${projectId}`).length;
  await page.clock.runFor(30_001);
  await expect.poll(() => mock.calls.filter(call => call.path === `/v1/atlas/projects/${projectId}`).length).toBeGreaterThan(before);
  expect(mock.unexpected).toEqual([]);
});

for (const choice of ['Cancel', 'Close without syncing', 'Sync and close']) {
  test(`tab close: ${choice}`, async ({ page }) => {
    const mock = await openEdit(page); await edit(page, mock.project);
    const dialog = await closeTab(page);
    await dialog.getByRole('button', { name: choice, exact: true }).click();
    await expect(dialog).toHaveCount(0);
    if (choice === 'Cancel') {
      await expect(page.getByTestId('sync-chip')).toHaveText('1 change not synced');
      expect(mock.syncs).toHaveLength(0);
    } else {
      await expect(page.getByTestId('sync-chip')).toHaveCount(0);
      expect(mock.syncs).toHaveLength(choice === 'Sync and close' ? 1 : 0);
      const draft = await storedDraft(page);
      if (choice === 'Close without syncing') expect(draft.pending.spots).toHaveLength(mock.project.grainatlas_ui.spots.length + 1);
      else expect(draft.pending).toBeNull();
    }
    expect(mock.unexpected).toEqual([]);
  });
}

for (const failure of ['offline', 'refused'] as const) {
  test(`tab Sync and close keeps the tab and outbox when ${failure}`, async ({ page }) => {
    const mock = await openEdit(page); await edit(page, mock.project);
    if (failure === 'offline') mock.offline(); else mock.refuse();
    const dialog = await closeTab(page);
    await dialog.getByRole('button', { name: 'Sync and close', exact: true }).click();
    await expect(page.getByTestId('sync-chip')).toBeVisible();
    await expect(page.getByRole('alert').filter({ hasText: failure === 'refused' ? /503.*Synthetic database maintenance/ : /could not|offline|failed|reach/i })).toBeVisible();
    await expect.poll(async () => !!(await storedDraft(page))?.pending).toBe(true);
    const reminder = await closeTab(page);
    if (failure === 'offline') await expect(reminder).toContainText('Sync next time you are online');
    await reminder.getByRole('button', { name: 'Cancel', exact: true }).click();
  });
}

test('browser tab close uses the native beforeunload warning and dismiss retains edits', async ({ page }) => {
  const mock = await openEdit(page); await edit(page, mock.project);
  const originalUrl = page.url();
  const originalSpots = await page.getByTestId('spot').count();
  await expect.poll(async () => (await storedDraft(page))?.pending?.spots.length).toBe(mock.project.grainatlas_ui.spots.length + 1);
  const originalDraft = await storedDraft(page);
  const warning = page.waitForEvent('dialog');
  // Handle rejection immediately: dismissal can reject goto before dismiss resolves.
  const navigation = page.goto('/?leaving=1').then(
    () => ({ cancelled: false as const }),
    error => ({ cancelled: true as const, error }),
  );
  const dialog = await warning;
  expect(dialog.type()).toBe('beforeunload');
  await dialog.dismiss();
  const result = await navigation;
  expect(result.cancelled).toBe(true);
  if (!result.cancelled) throw new Error('Navigation unexpectedly completed after dismissing beforeunload');
  expect(result.error.message).toMatch(/net::ERR_ABORTED/);
  await expect(page).toHaveURL(originalUrl);
  await expect(page.getByTestId('spot')).toHaveCount(originalSpots);
  await expect(page.getByTestId('sync-chip')).toHaveText('1 change not synced');
  expect((await storedDraft(page)).pending).toEqual(originalDraft.pending);
  expect(mock.syncs).toHaveLength(0);
  expect(mock.unexpected).toEqual([]);
});

for (const choice of ['Cancel', 'Close without syncing', 'Sync and close']) {
  test(`desktop quit reminder: ${choice}`, async ({ page }) => {
    const mock = await openEdit(page, { desktop: true }); await edit(page, mock.project);
    const dialog = await quit(page);
    await expect(dialog).toContainText(mock.project.name);
    await expect(dialog).toContainText('saved locally in the outbox');
    await dialog.getByRole('button', { name: choice, exact: true }).click();
    if (choice === 'Cancel') {
      await expect(dialog).toHaveCount(0);
      expect(await page.evaluate(() => (window as any).__finishedQuit)).toBe(0);
      expect(mock.syncs).toHaveLength(0);
    } else {
      await expect.poll(() => page.evaluate(() => (window as any).__finishedQuit)).toBe(1);
      expect(await page.evaluate(() => {
        const event = new Event('beforeunload', { cancelable: true });
        window.dispatchEvent(event);
        return event.defaultPrevented;
      })).toBe(false); // The native choice must not produce a second browser warning.
      expect(mock.syncs).toHaveLength(choice === 'Sync and close' ? 1 : 0);
      if (choice === 'Close without syncing') expect((await storedDraft(page, true)).pending.spots).toHaveLength(mock.project.grainatlas_ui.spots.length + 1);
    }
    expect(mock.unexpected).toEqual([]);
  });
}

test('desktop Sync and close keeps the app open when sync fails', async ({ page }) => {
  const mock = await openEdit(page, { desktop: true }); await edit(page, mock.project); mock.refuse();
  const dialog = await quit(page);
  await dialog.getByRole('button', { name: 'Sync and close', exact: true }).click();
  const alert = page.getByRole('alert').filter({ hasText: /503.*Synthetic database maintenance/ });
  await expect(alert).toBeVisible();
  await alert.getByRole('button', { name: 'Review issues', exact: true }).click();
  await expect(page.getByTestId('sync-details')).toBeFocused();
  await expect(page.getByTestId('sync-action')).toBeEnabled();
  expect(await page.evaluate(() => (window as any).__finishedQuit)).toBe(0);
  await expect(page.getByTestId('sync-chip')).toBeVisible();
  expect((await storedDraft(page, true)).pending).not.toBeNull();
});
