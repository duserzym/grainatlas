import { test, expect, type Page } from '@playwright/test';
import { mockRevisionRead } from './mock-session';

const server = 'https://database.example.test';
const projectId = '10000000-0000-4000-8000-000000000001';
const ownerId = '30000000-0000-4000-8000-000000000001';
const mountId = (i: number) => `20000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`;
function mount(i: number) {
  return { id: mountId(i), label: `Invented mount ${String(i + 1).padStart(3, '0')}`, mount_type: 'grain_tape', creator: { id: ownerId, name: 'Robin Example' }, lab: { handle: 'invented-lab', display_name: 'Invented Lab' }, role: 'write', can_edit: true, sample_names: ['Example A', 'Example B', 'Example C', 'Example D'], sample_count: 4, grain_count: 12, spot_count: 20, mineral: ['zircon'], session_dates: ['2026-01-01'], created_at: '2026-01-01T00:00:00Z', imaged_at: '2026-01-02T00:00:00Z', last_changed_at: '2026-01-03T00:00:00Z', content_revision: 4, atlas: { project_id: projectId, thumbnail: null, version: 1 } };
}
interface Options { count?: number; signedOut?: boolean; superuser?: boolean; local?: boolean; unlinked?: boolean; hosted?: boolean; legacyLabs?: boolean; holdRevisions?: boolean }
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
    if (u.pathname === `/v1/atlas/projects/${projectId}/link`) {
      if (r.headers()['x-upbdb-oversight'] !== 'on-behalf') return json({ code: 'oversight_confirm', owner: 'Robin Example' }, 403);
      expect(r.postDataJSON()).toEqual({ mount_id: mountId(0), make_current: false });
      return json({ project_uuid: projectId, mount_id: mountId(0), revision: 4 });
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
          await fits(page);
        });
      }
      test('signed out shows local projects only', async ({ page }) => {
        const mock = await setup(page, { signedOut: true, unlinked: true }); await page.goto('/');
        await expect(page.getByText('Sign in to see your mounts')).toBeVisible();
        await expect(page.getByTestId('recent-row')).toHaveCount(1); await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        expect(mock.calls).toHaveLength(0); await fits(page);
      });
      test('offline cached metadata and local actions survive reload', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.waitForFunction(async () => {
          const databases = await indexedDB.databases(); return databases.length > 0;
        });
        mock.goOffline(); await page.getByRole('button', { name: 'Refresh', exact: true }).click();
        await expect(page.getByTestId('offline-cache')).toContainText('Offline, as of');
        await expect(page.getByRole('button', { name: 'Open local', exact: true })).toBeEnabled(); await fits(page);
        await page.reload(); await expect(page.getByTestId('offline-cache')).toContainText('Offline, as of');
      });
      test('badges use one batched revision lookup', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        const tile = page.getByTestId('mount-tile');
        for (const badge of ['On this computer', 'Local changes', 'Newer on server', 'Shared · Write']) await expect(tile.getByText(badge, { exact: true })).toBeVisible();
        expect(mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions')).toHaveLength(1); await fits(page);
      });
      test('superuser defaults to own lab and can choose All labs', async ({ page }) => {
        const mock = await setup(page, { superuser: true, count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        expect(mock.calls.find(c => c.path === '/v1/mounts')?.query.lab).toBe('invented-lab');
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('button', { name: 'second-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('button', { name: 'other-lab', exact: true })).toHaveCount(0);
        await page.getByRole('button', { name: 'All labs', exact: true }).click();
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/mounts').at(-1)?.query.lab).toBeUndefined(); await fits(page);
      });
      test('ordinary accounts get a chip per lab from /labs', async ({ page }) => {
        await setup(page, { count: 1 }); await page.goto('/');
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('button', { name: 'invented-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('button', { name: 'second-lab', exact: true })).toBeVisible();
        await expect(scope.getByRole('button', { name: 'All labs', exact: true })).toHaveCount(0);
      });
      test('older servers give superusers Mine and All labs without team-lab chips', async ({ page }) => {
        const mock = await setup(page, { count: 1, superuser: true, legacyLabs: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        const scope = page.getByLabel('Mount scope', { exact: true });
        await expect(scope.getByRole('button', { name: 'Mine', exact: true })).toHaveClass(/primary/);
        await expect(scope.getByRole('button', { name: 'All labs', exact: true })).toBeVisible();
        await expect(scope.getByRole('button', { name: 'invented-lab', exact: true })).toHaveCount(0);
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
        expect(writes).toHaveLength(2); expect(writes[0].headers['x-upbdb-oversight']).toBeUndefined(); expect(writes[1].headers['x-upbdb-oversight']).toBe('on-behalf'); expect(prompts).toBe(1); await fits(page);
      });
      test('sign-out keeps the mirror hidden and same-account sign-in shows it before revalidation', async ({ page }) => {
        const mock = await setup(page, { count: 1, local: true }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('sign-out').click();
        await page.getByRole('button', { name: 'Open', exact: true }).click();
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
        await page.getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        await expect(page.getByText('No mounts match these filters.')).toBeVisible();
        await expect.poll(() => mock.calls.filter(c => c.path === '/v1/accounts/me').length).toBeGreaterThan(1);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('sign-out').click();
        mock.account('viewer'); mock.hold();
        await page.getByTestId('token').fill('new-token-same-account'); await page.getByTestId('token').press('Tab');
        await page.getByRole('button', { name: 'Open', exact: true }).click();
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
        await page.getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByText('No mounts match these filters.')).toBeVisible();
        await expect(page.getByTestId('mount-tile')).toHaveCount(0);
        await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
        await page.getByTestId('server').fill(server); await page.getByTestId('server').press('Tab');
        await page.getByRole('button', { name: 'Open', exact: true }).click();
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
      });
      test('combined server filters, sorting and removable chips', async ({ page }) => {
        const mock = await setup(page, { count: 1 }); await page.goto('/');
        await expect(page.getByTestId('mount-tile')).toHaveCount(1);
        await page.getByRole('button', { name: 'Mine', exact: true }).click();
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
        await page.getByRole('button', { name: 'L', exact: true }).click();
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
  const badge = page.getByTestId('mount-tile').getByText('Newer on server', { exact: true });
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
  await expect(page.getByTestId('mount-tile').getByText('Newer on server', { exact: true })).toBeVisible();
  expect(mock.calls.filter(c => c.path === '/v1/atlas/projects/revisions')).toHaveLength(1);
});
