import { test, expect, type Page } from '@playwright/test';
import { mockRevisionRead } from './mock-session';

const server = 'https://database.example.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const ownerId = '30000000-0000-4000-8000-000000000001';
const mountId = (i: number) => `20000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`;
function mount(i: number) {
  return { id: mountId(i), label: `Invented mount ${String(i + 1).padStart(3, '0')}`, mount_type: 'grain_tape', creator: { id: ownerId, name: 'Robin Example' }, lab: { handle: 'invented-lab', display_name: 'Invented Lab' }, role: 'write', can_edit: true, sample_names: ['Example A', 'Example B', 'Example C', 'Example D'], sample_count: 4, grain_count: 12, spot_count: 20, mineral: ['zircon'], session_dates: ['2026-01-01'], created_at: '2026-01-01T00:00:00Z', imaged_at: '2026-01-02T00:00:00Z', last_changed_at: '2026-01-03T00:00:00Z', content_revision: 4, atlas: { project_id: projectId, thumbnail: null, version: 1 } };
}
interface Options { count?: number; signedOut?: boolean; superuser?: boolean; local?: boolean; unlinked?: boolean; hosted?: boolean; legacyLabs?: boolean; holdRevisions?: boolean; separate?: boolean }
async function setup(page: Page, options: Options = {}) {
  const calls: { path: string; query: Record<string, string>; headers: Record<string, string>; method: string }[] = [];
  let offline = false;
  let accountId = 'viewer';
  let holdRevalidation = false;
  let releaseRevalidation!: () => void;
  const gate = new Promise<void>(resolve => { releaseRevalidation = resolve; });
  let releaseRevisions!: () => void;
  const revisionGate = new Promise<void>(resolve => { releaseRevisions = resolve; });
  await page.addInitScript(({ server, options, projectId, mountId }) => {
    if (sessionStorage.getItem('grainatlas.e2e.initialized')) return;
    sessionStorage.setItem('grainatlas.e2e.initialized', '1');
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: false }));
    if (!options.signedOut) sessionStorage.setItem('grainatlas.upbdb.token', 'invented-token');
    if (options.hosted) {
      const w = window as any;
      w.__LAB_SHELL__ = { module: 'grainatlas' };
      w.__scope = { viewAs: { username: 'robin', name: 'Robin Example' }, onBehalf: false };
      w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => cmd === 'bridge_token' ? 'invented-token' : cmd === 'bridge_oversight' ? w.__scope : null };
    }
    if (options.unlinked || options.separate) projectId = '10000000-0000-4000-8000-000000000002';
    if (options.local || options.unlinked) {
      localStorage.setItem('grainatlas.recent', JSON.stringify([{ id: projectId, name: 'Invented local project', when: '2026-01-01T00:00:00Z', areas: 1, spots: 20 }]));
      localStorage.setItem('grainatlas.drafts', JSON.stringify([projectId]));
      localStorage.setItem(`grainatlas.draft.${projectId}`, JSON.stringify({ uuid: projectId, name: 'Invented local project', server, mount: options.unlinked ? null : mountId, base_revision: 2, base: { grains: [], spots: [], labels: [] }, synced: true, changes: { grains: 1, spots: 0, labels: 0 }, updated_at: '2026-01-01T00:00:00Z' }));
    }
  }, { server, options, projectId, mountId: mountId(0) });
  await page.route(/^https:\/\/(?:database|other)\.example\.test\//, async route => {
    const r = route.request(), u = new URL(r.url());
    if (r.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    calls.push({ path: u.pathname, query: Object.fromEntries(u.searchParams), headers: r.headers(), method: r.method() });
    if (offline) return route.abort('connectionrefused');
    if (holdRevalidation && u.pathname === '/v1/mounts') await gate;
    const headers = { 'access-control-allow-origin': '*', 'access-control-expose-headers': 'ETag, X-Next-Cursor, X-Total-Count', ETag: `"${u.pathname}-${u.search}"` };
    const json = (body: unknown, status = 200) => route.fulfill({ status, json: body, headers });
    if (u.pathname === '/v1/accounts/me') return json({ id: accountId, username: accountId, display_name: 'Casey Example', can_manage_people: !!options.superuser, groups: [{ name: 'invented-team', lab_id: 'lab-id' }] });
    if (u.pathname === '/v1/server') return json({ dataset_id: u.origin === server ? 'invented-dataset' : 'other-dataset' });
    if (u.pathname === '/v1/labs') return json([
      { id: 'lab-id', handle: 'invented-lab', display_name: 'Invented Lab', ...(!options.legacyLabs ? { my_role: 'member' } : {}) },
      { id: 'second-lab', handle: 'second-lab', ...(!options.legacyLabs ? { my_role: 'owner' } : {}) },
      ...(options.superuser ? [{ id: 'other-lab', handle: 'other-lab', ...(!options.legacyLabs ? { my_role: null } : {}) }] : []),
    ]);
    if (r.method() === 'POST' && u.pathname === '/v1/atlas/projects/revisions' && options.holdRevisions) await revisionGate;
    if (await mockRevisionRead(route, [{ project_uuid: projectId, mount_id: mountId(0), revision: 4 }])) return;
    if (/^\/v1\/atlas\/projects\/[^/]+\/link$/.test(u.pathname)) {
      if (r.headers()['x-upbdb-oversight'] !== 'on-behalf') return json({ code: 'oversight_confirm', owner: 'Robin Example' }, 403);
      expect(r.postDataJSON()).toEqual({ mount_id: mountId(0), make_current: false });
      return json({ project_uuid: u.pathname.split('/')[4], mount_id: mountId(0), revision: 4 });
    }
    if (u.pathname === '/v1/mounts') {
      expect(u.searchParams.get('limit')).toBe('50');
      const count = accountId === 'viewer' && u.origin === server ? options.count ?? 12 : 0, from = Number(u.searchParams.get('after') ?? 0), end = Math.min(count, from + 50);
      return route.fulfill({ json: Array.from({ length: end - from }, (_, i) => mount(from + i)), headers: { ...headers, 'X-Total-Count': String(count), ...(end < count ? { 'X-Next-Cursor': String(end) } : {}) } });
    }
    return json({ detail: 'Unexpected invented request' }, 404);
  });
  return { calls, releaseRevisions, goOffline() { offline = true; }, goOnline() { offline = false; }, account(id: string) { accountId = id; }, hold() { holdRevalidation = true; }, release() { holdRevalidation = false; releaseRevalidation(); } };
}
async function fits(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const tile = page.getByTestId('mount-tile').first();
  if (await tile.count()) { const box = await tile.boundingBox(); expect(box!.x + box!.width).toBeLessThanOrEqual((page.viewportSize()?.width ?? 0) + 1); }
}
async function waitForMountCache(page: Page) {
  await page.waitForFunction(() => new Promise<boolean>(resolve => {
    const open = indexedDB.open('grainatlas-mirror');
    open.onerror = () => resolve(false);
    open.onsuccess = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains('items')) { db.close(); resolve(false); return; }
      const read = db.transaction('items').objectStore('items').get('mirror/invented-dataset/viewer/start-screen.json');
      read.onsuccess = () => { db.close(); resolve(Object.values(JSON.parse(read.result ?? '{}').pages ?? {}).some((page: any) => page.rows?.length)); };
      read.onerror = () => { db.close(); resolve(false); };
    };
  }));
}
for (const viewport of [{ width: 1280, height: 800 }, { width: 1440, height: 900 }]) {
  for (const theme of ['light', 'dark']) {
    test.describe(`${viewport.width} × ${viewport.height}, ${theme}`, () => {
      test.use({ viewport });
      test.beforeEach(async ({ page }) => { await page.addInitScript(theme => localStorage.setItem('grainatlas.theme', theme), theme); });
      for (const count of [0, 1, 12, 300]) {
        test(`${count} mounts, bounded first page and cursor paging`, async ({ page }) => {
          const mock = await setup(page, { count }); await page.goto('/');
          await expect(page.getByTestId('mount-tile')).toHaveCount(Math.min(50, count));
          await expect(page.getByRole('heading', { name: 'Mounts', exact: true })).toBeVisible();
          if (count === 0) await expect(page.getByText('No mounts match these filters.')).toBeVisible();
          if (count === 300) {
            await page.getByRole('button', { name: 'Show more', exact: true }).click();
            await expect(page.getByTestId('mount-tile')).toHaveCount(100);
            expect(mock.calls.filter(c => c.path === '/v1/mounts')).toHaveLength(2);
            expect(mock.calls.filter(c => c.path === '/v1/mounts')[1].query.after).toBe('50');
          }
          if (count) await expect(page.getByTestId('mount-tile').first().getByRole('button', { name: mount(0).label, exact: true })).toBeEnabled();
          await expect(page.locator('button.primary:visible')).toHaveCount(1);
          await fits(page);
        });
      }
      test('S fits the requested fully visible rows and every density has one primary action', async ({ page }) => {
        await setup(page, { count: 60 });
        // The existing preference must select S on first render, not just after a click.
        await page.addInitScript(() => localStorage.setItem('grainatlas.layout', JSON.stringify({ tileSize: 'S' })));
        await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(50);
        await expect(page.getByRole('radio', { name: 'S', exact: true })).toHaveAttribute('aria-checked', 'true');
        await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toHaveCount(0);
        const minimum = viewport.width === 1440 ? 11 : 9;
        const fullyVisibleRows = () => page.getByTestId('mount-tile').evaluateAll(rows => rows.filter(row => {
          const box = row.getBoundingClientRect();
          const clip = document.querySelector('.open')!.getBoundingClientRect();
          return box.top >= clip.top && box.bottom <= Math.min(innerHeight, clip.bottom) && box.left >= 0 && box.right <= innerWidth;
        }).length);
        await expect.poll(fullyVisibleRows).toBeGreaterThanOrEqual(minimum);
        const first = page.getByTestId('mount-tile').first();
        const rowBox = (await first.boundingBox())!;
        expect(rowBox.height).toBeGreaterThanOrEqual(48); expect(rowBox.height).toBeLessThanOrEqual(56);
        await expect(first.locator('.samples')).toHaveText('Example A +3');
        await expect(first.locator('.samples')).toHaveAttribute('title', 'Example A, Example B, Example C, Example D');
        await expect(first.locator('.changed')).toHaveAttribute('title', mount(0).last_changed_at);
        await expect(first.locator('.changed')).toHaveText(/(?:just now|\d+ (?:min|h|d|mo|y) ago)/);
        for (const size of ['S', 'M', 'L']) {
          await page.getByRole('radio', { name: size, exact: true }).click();
          await expect(page.locator('button.primary:visible')).toHaveCount(1);
          await expect(page.locator('button.primary:visible')).toHaveText('Open project…');
          await expect(first.getByRole('button', { name: mount(0).label, exact: true })).toBeEnabled();
          expect(await first.locator('button button, button a, button input, button select').count()).toBe(0);
          await fits(page);
        }
        await expect(first.locator('.preview')).toHaveCSS('height', '150px');
        await page.getByRole('radio', { name: 'S', exact: true }).click();
        await page.reload();
        await expect(page.getByRole('radio', { name: 'S', exact: true })).toHaveAttribute('aria-checked', 'true');
        await expect(page.getByTestId('mount-tile')).toHaveCount(50);
        const drag = await page.evaluateHandle(() => {
          const transfer = new DataTransfer(); transfer.items.add(new File(['invented'], 'invented.png', { type: 'image/png' })); return transfer;
        });
        const region = page.getByRole('region', { name: 'Open files', exact: true });
        await region.dispatchEvent('dragenter', { dataTransfer: drag });
        await first.dispatchEvent('dragenter', { dataTransfer: drag });
        await expect(page.locator('.drop-overlay')).toBeVisible();
        await expect(page.locator('.drop-overlay')).toHaveCSS('pointer-events', 'none');
        await first.dispatchEvent('dragleave', { dataTransfer: drag });
        await expect(page.locator('.drop-overlay')).toBeVisible();
        await region.dispatchEvent('dragleave', { dataTransfer: drag });
        await expect(page.locator('.drop-overlay')).toHaveCount(0);
        await drag.dispose();
      });
      test('column headers use the existing sort and direction fields', async ({ page }) => {
        const mock = await setup(page, { count: 60 }); await page.goto('/');
        await page.getByRole('radio', { name: 'S', exact: true }).click();
        const name = page.getByRole('columnheader').filter({ has: page.getByRole('button', { name: /^Name/ }) });
        const changed = page.getByRole('columnheader').filter({ has: page.getByRole('button', { name: /^Last change/ }) });
        await expect(changed).toHaveAttribute('aria-sort', 'descending');
        await name.getByRole('button').click();
        await expect(name).toHaveAttribute('aria-sort', 'ascending');
        await expect(page.getByLabel('Sort mounts', { exact: true })).toHaveValue('label');
        await expect(page.getByLabel('Sort direction', { exact: true })).toHaveValue('ascending');
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/mounts').at(-1)?.query.sort).toBe('label');
        await name.getByRole('button').click();
        await expect(name).toHaveAttribute('aria-sort', 'descending');
        await expect(page.getByLabel('Sort direction', { exact: true })).toHaveValue('descending');
        await page.getByLabel('Sort mounts', { exact: true }).selectOption('last_changed');
        await expect(changed).toHaveAttribute('aria-sort', 'descending');
        await expect(name).toHaveAttribute('aria-sort', 'none');
        await expect(page.getByRole('columnheader', { name: 'Grains', exact: true }).getByRole('button')).toHaveCount(0);
        await fits(page);
      });
      test('Import contains exactly the original actions and preserves their targets', async ({ page }) => {
        await setup(page, { count: 1 }); await page.goto('/');
        const trigger = page.getByRole('button', { name: 'Import', exact: true });
        await expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
        await expect(trigger).toHaveAttribute('aria-expanded', 'false');
        await trigger.focus(); await trigger.press('ArrowDown');
        const menu = page.getByRole('menu', { name: 'Import', exact: true });
        await expect(menu.getByRole('menuitem')).toHaveText(['Open package…', 'Import pick list…', 'Choose images…']);
        await expect(menu.getByRole('menuitem').first()).toBeFocused();
        await page.keyboard.press('ArrowUp'); await expect(menu.getByRole('menuitem').last()).toBeFocused();
        await page.keyboard.press('Home'); await expect(menu.getByRole('menuitem').first()).toBeFocused();
        await page.keyboard.press('End'); await expect(menu.getByRole('menuitem').last()).toBeFocused();
        await page.keyboard.press('Escape'); await expect(trigger).toBeFocused();
        await expect(trigger).toHaveAttribute('aria-expanded', 'false');
        for (const [name, input] of [['Open package…', 'package-input'], ['Choose images…', 'images-input']]) {
          await trigger.click();
          const choosing = page.waitForEvent('filechooser');
          await menu.getByRole('menuitem', { name, exact: true }).click();
          const chooser = await choosing;
          expect(await chooser.element().getAttribute('data-testid')).toBe(input);
          await expect(menu).toHaveCount(0);
        }
        await trigger.click(); await menu.getByRole('menuitem', { name: 'Import pick list…', exact: true }).click();
        await expect(page.getByTestId('pick-import-dialog')).toBeVisible();
        await page.getByTestId('pick-import-dialog').getByRole('button', { name: 'Close', exact: true }).click();
        await fits(page);
      });
      test('secondary mount actions are outside the open button and reachable by keyboard', async ({ page }) => {
        await setup(page, { count: 60 }); await page.goto('/');
        await page.getByRole('radio', { name: 'S', exact: true }).click();
        const row = page.getByTestId('mount-tile').first();
        const open = row.getByRole('button', { name: mount(0).label, exact: true });
        await open.focus(); await page.keyboard.press('Tab');
        const more = row.getByRole('button', { name: `More actions for ${mount(0).label}`, exact: true });
        await expect(more).toBeFocused(); await more.press('Enter');
        const link = row.getByRole('menuitem', { name: 'Link a local project…', exact: true });
        await expect(link).toBeFocused();
        expect(await open.locator('[role="menuitem"], button').count()).toBe(0);
        await page.keyboard.press('Escape'); await expect(more).toBeFocused();
        await more.press('ArrowDown'); await link.press('Enter');
        await expect(page.getByRole('dialog', { name: 'Link a local project', exact: true })).toBeVisible();
        await expect(page.getByRole('dialog')).toContainText('No unlinked local projects. Open a local project first.');
      });
      test('signed out shows local projects only', async ({ page }) => {
        const mock = await setup(page, { signedOut: true, unlinked: true }); await page.goto('/');
        await expect(page.getByText('Sign in to see your mounts')).toBeVisible();
        await expect(page.locator('.recent-list').getByRole('heading')).toHaveText('Recent projects');
        await expect.poll(() => page.locator('.recent-list').evaluate(el => !!(el.compareDocumentPosition(document.querySelector('[data-testid="mount-browser"]')!) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
        expect((await page.locator('.recent-list').boundingBox())!.x).toBeLessThan((await page.getByTestId('mount-browser').boundingBox())!.x);
        await expect(page.getByTestId('recent-row')).toHaveCount(1); await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        expect(mock.calls).toHaveLength(0); await fits(page);
      });
      test('offline cached metadata and local actions survive reload', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await waitForMountCache(page);
        mock.goOffline(); await page.getByRole('button', { name: 'Refresh', exact: true }).click();
        await expect(page.getByTestId('offline-cache')).toContainText('Offline copy · as of');
        const tile = page.getByTestId('mount-tile');
        await expect(tile.getByTestId('tile-status').locator('p')).toHaveText(['Local copy on this computer', '1 change not yet uploaded']);
        await expect(tile.getByText(/Offline copy · as of/)).toHaveCount(0);
        await expect(page.getByTestId('offline-cache')).toHaveAttribute('title', /.+/);
        await expect(page.getByText('Failed to fetch', { exact: true })).toHaveCount(0);
        const open = tile.getByRole('button', { name: mount(0).label, exact: true });
        await expect(open).toBeEnabled(); await expect(open).not.toHaveClass(/primary/); await fits(page);
        await page.reload(); await expect(page.getByTestId('offline-cache')).toContainText('Offline copy · as of');
      });
      test('badges use one batched revision lookup', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        const tile = page.getByTestId('mount-tile');
        const lines = tile.getByTestId('tile-status').locator('p');
        await expect(lines).toHaveText(['Database · local copy not yet synced', '1 change not yet uploaded', 'Newer in database']);
        for (let i = 1; i < 3; i++) {
          const previous = (await lines.nth(i - 1).boundingBox())!, current = (await lines.nth(i).boundingBox())!;
          expect(current.y).toBeGreaterThanOrEqual(previous.y + previous.height);
        }
        await expect(tile.getByRole('button', { name: mount(0).label, exact: true })).toBeEnabled();
        await expect(tile.getByText('Shared · Write', { exact: true })).toBeVisible();
        expect(mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions')).toHaveLength(1); await fits(page);
      });
      test('database leads by default, local choice persists, Settings shares the choice', async ({ page }) => {
        await setup(page, { count: 1, local: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        const mounts = page.getByTestId('mount-browser'), local = page.locator('.recent-list');
        await expect(local.getByRole('heading')).toHaveText('On this computer only');
        await expect.poll(() => mounts.evaluate(el => !!(el.compareDocumentPosition(document.querySelector('.recent-list')!) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
        expect((await mounts.boundingBox())!.x).toBeLessThan((await local.boundingBox())!.x);
        await expect(page.getByTestId('recent-row')).toHaveCount(0);
        await page.getByRole('radio', { name: 'Local first', exact: true }).click();
        await expect.poll(() => local.evaluate(el => !!(el.compareDocumentPosition(document.querySelector('[data-testid="mount-browser"]')!) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
        expect((await local.boundingBox())!.x).toBeLessThan((await mounts.boundingBox())!.x);
        await page.reload();
        await expect(page.getByRole('radio', { name: 'Local first', exact: true })).toHaveAttribute('aria-checked', 'true');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        expect((await local.boundingBox())!.x).toBeLessThan((await mounts.boundingBox())!.x);
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Settings', exact: true }).click();
        await expect(page.getByRole('combobox', { name: 'Start screen' })).toHaveValue('local');
        await page.getByRole('combobox', { name: 'Start screen' }).selectOption('database');
        expect(await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.layout')!).startView)).toBe('database');
        await fits(page);
      });
      test('unlinked local projects stay in the local column beside database mounts', async ({ page }) => {
        await setup(page, { count: 1, unlinked: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await expect(page.getByTestId('recent-row')).toHaveCount(1);
        await expect(page.locator('.recent-list').getByRole('heading')).toHaveText('On this computer only');
        await expect(page.getByTestId('recent-row')).toContainText('Invented local project'); await fits(page);
      });
      test('offline without a local bundle leads with local projects and explains the disabled Open', async ({ page }) => {
        const mock = await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        mock.goOffline(); await page.getByRole('button', { name: 'Refresh', exact: true }).click();
        await expect(page.getByTestId('offline-cache')).toContainText('Offline copy');
        const tile = page.getByTestId('mount-tile');
        await expect(tile.getByTestId('tile-status').locator('p')).toHaveText(['Not on this computer']);
        await expect(tile.getByText(/Offline copy · as of/)).toHaveCount(0);
        await expect(page.getByText('Failed to fetch', { exact: true })).toHaveCount(0);
        const open = tile.getByRole('button', { name: mount(0).label, exact: true });
        await expect(open).toBeDisabled(); await expect(open).not.toHaveClass(/primary/); await expect(open).toHaveAttribute('title', 'Not on this computer; needs a connection');
        const mounts = page.getByTestId('mount-browser'), local = page.locator('.recent-list');
        await expect.poll(() => local.evaluate(el => !!(el.compareDocumentPosition(document.querySelector('[data-testid="mount-browser"]')!) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
        expect((await local.boundingBox())!.x).toBeLessThan((await mounts.boundingBox())!.x);
        await expect(page.getByRole('radio', { name: 'Database first', exact: true })).toHaveAttribute('aria-checked', 'true'); await fits(page);
      });
      test('offline cached bundle has a read-only status and enabled row open action', async ({ page }) => {
        const mock = await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await waitForMountCache(page);
        // Synthetic bundle metadata; the unit tests exercise the read-only open itself.
        await page.evaluate(async projectId => {
          const db = await new Promise<IDBDatabase>((resolve, reject) => {
            const request = indexedDB.open('grainatlas-mirror', 1);
            request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
          });
          const record = JSON.stringify({ atlas: { project_id: projectId }, project: { uuid: projectId }, elements: null, revision: 4, at: new Date().toISOString() });
          const prefix = 'mirror/invented-dataset/viewer/bundles/';
          await new Promise<void>((resolve, reject) => {
            const tx = db.transaction('items', 'readwrite'), items = tx.objectStore('items');
            items.put(record, `${prefix}${projectId}.json`);
            items.put(JSON.stringify([{ project: projectId, bytes: new TextEncoder().encode(record).length }]), `${prefix}index.json`);
            tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
          });
          db.close();
        }, projectId);
        mock.goOffline(); await page.reload();
        await expect(page.getByTestId('offline-cache')).toContainText('Offline copy · as of');
        const tile = page.getByTestId('mount-tile');
        await expect(tile.getByTestId('tile-status').locator('p')).toHaveText(['Offline copy available (read-only)']);
        await expect(tile.getByText(/Offline copy · as of/)).toHaveCount(0);
        await expect(page.getByTestId('offline-cache')).toHaveAttribute('title', /.+/);
        await expect(page.getByText('Failed to fetch', { exact: true })).toHaveCount(0);
        const open = tile.getByRole('button', { name: mount(0).label, exact: true });
        await expect(open).toBeEnabled(); await expect(open).toHaveAttribute('title', 'Open offline copy'); await expect(open).not.toHaveClass(/primary/); await fits(page);
      });
      test('database HTTP errors stay visible outside offline mode', async ({ page }) => {
        await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.route(`${server}/v1/mounts?**`, route => route.fulfill({ status: 503, json: { detail: 'Synthetic database unavailable' }, headers: { 'access-control-allow-origin': '*' } }));
        await page.getByRole('button', { name: 'Refresh', exact: true }).click();
        await expect(page.getByRole('status').filter({ hasText: 'Database 503:' })).toContainText('Synthetic database unavailable');
        await expect(page.getByTestId('offline-cache')).toHaveCount(0); await fits(page);
      });
      test('superuser defaults to own lab and can choose All labs', async ({ page }) => {
        const mock = await setup(page, { superuser: true, count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        expect(mock.calls.find(c => c.path === '/v1/mounts')?.query.lab).toBe('invented-lab');
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('radio', { name: 'second-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('radio', { name: 'other-lab', exact: true })).toHaveCount(0);
        await page.getByRole('radio', { name: 'All labs', exact: true }).click();
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/mounts').at(-1)?.query.lab).toBeUndefined(); await fits(page);
      });
      test('ordinary accounts get a chip per lab from /labs', async ({ page }) => {
        await setup(page, { count: 1 }); await page.goto('/');
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('radio', { name: 'invented-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('radio', { name: 'second-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('radio', { name: 'All labs', exact: true })).toHaveCount(0);
      });
      test('older servers give superusers Mine and All labs without team-lab chips', async ({ page }) => {
        const mock = await setup(page, { count: 1, superuser: true, legacyLabs: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('radio', { name: 'Mine', exact: true })).toHaveAttribute('aria-checked', 'true');
        await expect(scope.getByRole('radio', { name: 'All labs', exact: true })).toBeVisible();
        await expect(scope.getByRole('radio', { name: 'invented-lab', exact: true })).toHaveCount(0);
        expect(mock.calls.find(c => c.path === '/v1/mounts')?.query.mine).toBe('true');
      });
      test('unlinked project asks before linking on behalf and preserves project UUID', async ({ page }) => {
        const mock = await setup(page, { count: 1, unlinked: true }); await page.goto('/');
        await page.getByRole('button', { name: 'Link to a mount', exact: true }).click();
        await expect(page.getByTestId('mount-select')).toContainText('Invented mount 001 · Robin Example');
        let prompts = 0;
        page.on('dialog', async dialog => { prompts++; expect(dialog.message()).toBe("This mount is Robin Example's. Sync on their behalf?"); await dialog.accept(); });
        await page.getByTestId('mount-select').selectOption(mountId(0));
        await expect(page.getByRole('dialog')).toHaveCount(0);
        const writes = mock.calls.filter(c => c.path.endsWith('/link'));
        expect(writes).toHaveLength(2); expect(writes.every(c => c.path === '/v1/atlas/projects/10000000-0000-4000-8000-000000000002/link')).toBe(true); expect(writes[0].headers['x-upbdb-oversight']).toBeUndefined(); expect(writes[1].headers['x-upbdb-oversight']).toBe('on-behalf'); expect(prompts).toBe(1); await fits(page);
      });
      test('sign-out keeps the mirror hidden and same-account sign-in shows it before revalidation', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('sign-out').click();
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByText('Sign in to see your mounts')).toBeVisible();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0); await expect(page.getByTestId('recent-row')).toHaveCount(1); await fits(page);
        await page.reload();
        await expect(page.getByText('Sign in to see your mounts')).toBeVisible();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        await expect(page.getByTestId('recent-row')).toHaveCount(1);
        const savedDraft = await page.evaluate(async projectId => {
          const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('grainatlas', 1); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
          const r = db.transaction('items').objectStore('items').get(`grainatlas.draft.${projectId}`);
          const value = await new Promise<string>((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
          db.close(); return JSON.parse(value);
        }, projectId);
        expect(savedDraft.changes).toMatchObject({ grains: 1 });
        const readMirror = () => page.evaluate(async () => {
          const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('grainatlas-mirror', 1); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
          const r = db.transaction('items').objectStore('items').get('mirror/invented-dataset/viewer/start-screen.json');
          const value = await new Promise<string>((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
          db.close(); return value;
        });
        await expect.poll(readMirror).toContain('Invented mount 001');
        // Even a different valid account at this server cannot read the retained folder.
        mock.account('another-viewer');
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('token').fill('another-token'); await page.getByTestId('token').press('Tab');
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        await expect(page.getByText('No mounts match these filters.')).toBeVisible();
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/accounts/me').length).toBeGreaterThan(1);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('sign-out').click();
        mock.account('viewer'); mock.hold();
        await page.getByTestId('token').fill('new-token-same-account'); await page.getByTestId('token').press('Tab');
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toBeVisible();
        mock.release();
        await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toHaveCount(0);
        await expect.poll(readMirror).toContain('Invented mount 001');
      });
      test('switching servers hides the same account mirror until returning', async ({ page }) => {
        await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('server').fill('https://other.example.test'); await page.getByTestId('server').press('Tab');
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByText('No mounts match these filters.')).toBeVisible();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('server').fill(server); await page.getByTestId('server').press('Tab');
        await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
      });
      test('combined server filters, sorting and removable chips', async ({ page }) => {
        const mock = await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.getByRole('radio', { name: 'Mine', exact: true }).click();
        await page.getByRole('button', { name: 'Filters…', exact: true }).click();
        await page.getByLabel('Mineral', { exact: true }).fill('zircon');
        await page.getByLabel('Mineral', { exact: true }).press('Tab');
        await page.getByLabel('Sort mounts', { exact: true }).selectOption('label');
        await page.getByLabel('Sort direction', { exact: true }).selectOption('ascending');
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/mounts').at(-1)?.query).toMatchObject({ mine: 'true', mineral: 'zircon', sort: 'label' });
        await page.getByRole('button', { name: 'mineral: zircon ×', exact: true }).click();
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/mounts').at(-1)?.query.mineral).toBeUndefined(); await fits(page);
      });
      test('tile size and splitter persist and Reset layout restores defaults', async ({ page }) => {
        await setup(page, { count: 12 }); await page.goto('/');
        await page.getByRole('radio', { name: 'L', exact: true }).click();
        const splitter = page.getByRole('slider', { name: 'Recent projects width' });
        await splitter.focus(); await splitter.press('ArrowRight');
        expect(await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.layout')!))).toMatchObject({ recentWidth: 250, tileSize: 'L' });
        await page.reload(); await expect(splitter).toHaveAttribute('aria-valuenow', '250');
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByRole('button', { name: 'Reset layout', exact: true }).click();
        expect(await page.evaluate(() => localStorage.getItem('grainatlas.layout'))).toBeNull(); await fits(page);
      });
      test('hosted view-as banner disables every start-screen server write', async ({ page }) => {
        const mock = await setup(page, { count: 1, hosted: true, unlinked: true }); await page.goto('/');
        await expect(page.getByTestId('view-as-banner')).toHaveText('Viewing as Robin Example (read-only)');
        await expect(page.getByRole('button', { name: 'New mount…', exact: true })).toBeDisabled();
        await expect(page.getByRole('button', { name: 'Link to a mount', exact: true })).toBeDisabled();
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        expect(mock.calls.filter(c => c.path === '/v1/mounts').every(c => c.query.as === 'robin')).toBe(true);
        expect(mock.calls.filter(c => c.method !== 'GET' && c.path !== '/v1/atlas/projects/revisions')).toHaveLength(0); await fits(page);
      });
    });
  }
}


test('screen switches and refresh reuse the signed-in server identity', async ({ page }) => {
  const mock = await setup(page, { count: 1 }); await page.goto('/');
  await expect(page.getByTestId('mount-tile')).toHaveCount(1);
  const nav = page.getByRole('navigation', { name: 'Screens' });
  for (let i = 0; i < 3; i++) {
    await nav.getByRole('button', { name: 'Settings', exact: true }).click();
    await nav.getByRole('button', { name: 'Open', exact: true }).click();
    await expect(page.getByTestId('mount-tile')).toHaveCount(1);
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toHaveCount(0);
  }
  expect(mock.calls.filter(c => c.path === '/v1/accounts/me')).toHaveLength(1);
  expect(mock.calls.filter(c => c.path === '/v1/server')).toHaveLength(1);
});

test('revision badges reuse a fresh answer across screens and reloads, then revalidate with ETag', async ({ page }) => {
  await page.clock.install();
  const mock = await setup(page, { count: 1, local: true });
  await page.goto('/');
  const badge = page.getByTestId('mount-tile').getByText(/Newer in database/);
  await expect(badge).toBeVisible();
  const revisions = () => mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions');
  const nav = page.getByRole('navigation', { name: 'Screens' });
  for (let i = 0; i < 2; i++) {
    await nav.getByRole('button', { name: 'Settings', exact: true }).click();
    await nav.getByRole('button', { name: 'Open', exact: true }).click();
    await expect(badge).toBeVisible();
    await page.getByRole('button', { name: 'Refresh', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toHaveCount(0);
  }
  expect(revisions()).toHaveLength(1);
  // Observe the durable write before deliberately replacing the document.
  await page.waitForFunction(() => new Promise<boolean>(resolve => {
    const open = indexedDB.open('grainatlas-mirror');
    open.onerror = () => resolve(false);
    open.onsuccess = () => {
      const db = open.result;
      const read = db.transaction('items').objectStore('items').get('mirror/invented-dataset/viewer/start-screen.json');
      read.onsuccess = () => {
        db.close();
        resolve(!!JSON.parse(read.result ?? '{}').revisions?.batches?.length);
      };
      read.onerror = () => { db.close(); resolve(false); };
    };
  }));
  await page.reload();
  await expect(badge).toBeVisible();
  expect(revisions()).toHaveLength(1);
  await nav.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.clock.fastForward(61_000);
  expect(revisions()).toHaveLength(1);
  const revalidated = page.waitForResponse(r => new URL(r.url()).pathname === '/v1/atlas/projects/revisions');
  await nav.getByRole('button', { name: 'Open', exact: true }).click();
  expect((await revalidated).status()).toBe(304);
  await expect(badge).toBeVisible();
  expect(revisions()).toHaveLength(2);
  expect(revisions()[1].headers['if-none-match']).toBeTruthy();
});

for (const options of [{ count: 1 }, { count: 1, unlinked: true }, { count: 1, local: true, signedOut: true }]) {
  test(`no revision lookup without a signed-in linked recent project: ${JSON.stringify(options)}`, async ({ page }) => {
    const mock = await setup(page, options); await page.goto('/');
    if (options.signedOut) await expect(page.getByText('Sign in to see your mounts')).toBeVisible();
    else {
      await expect(page.getByTestId('mount-tile')).toHaveCount(1);
      await expect(page.getByRole('status').filter({ hasText: 'Loading mounts' })).toHaveCount(0);
    }
    expect(mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions')).toHaveLength(0);
  });
}

test('a stalled revision lookup never holds the mount list, Refresh or navigation', async ({ page }) => {
  const mock = await setup(page, { count: 1, local: true, holdRevisions: true });
  await page.goto('/');
  await expect.poll(() => mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions').length).toBe(1);
  await expect(page.getByTestId('mount-tile')).toHaveCount(1);
  const refresh = page.getByRole('button', { name: 'Refresh', exact: true });
  await expect(refresh).toBeEnabled(); await refresh.click();
  await expect(refresh).toBeEnabled();
  const nav = page.getByRole('navigation', { name: 'Screens' });
  await nav.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByTestId('mount-browser')).toHaveCount(0);
  mock.releaseRevisions();
  await nav.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.getByTestId('mount-tile').getByText(/Newer in database/)).toBeVisible();
  expect(mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions')).toHaveLength(1);
});


test('a separate local copy remains a keyboard-reachable secondary action', async ({ page }) => {
  await setup(page, { count: 1, local: true, separate: true }); await page.goto('/');
  await expect(page.getByTestId('mount-tile')).toHaveCount(1);
  await page.getByRole('radio', { name: 'S', exact: true }).click();
  const row = page.getByTestId('mount-tile');
  const more = row.getByRole('button', { name: `More actions for ${mount(0).label}`, exact: true });
  await more.focus(); await more.press('ArrowDown');
  const other = row.getByRole('menuitem', { name: 'Open other local copy', exact: true });
  await expect(other).toBeFocused(); await expect(other).toBeEnabled(); await other.press('Enter');
  await expect(page.getByTestId('recent-note')).toContainText('Opening by path needs the desktop app.');
  await expect(page.getByTestId('recent-row')).toContainText('Invented local project');
});
