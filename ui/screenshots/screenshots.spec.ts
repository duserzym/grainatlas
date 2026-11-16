import { expect, test, type Browser, type Page } from '@playwright/test';
import { mkdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DEMO_PACKAGE = process.env.DEMO_PACKAGE
  ? path.resolve(process.env.DEMO_PACKAGE)
  : path.resolve(__dirname, '../../target/screenshot-package/Example_mount');
const OUTPUT = path.resolve(__dirname, '../../target/screenshots');
const SERVER = 'https://upbdb.example.org';
const VIEWPORTS = [{ width: 1440, height: 900 }, { width: 1280, height: 800 }];
type Theme = 'light' | 'dark';
type Point = { x: number; y: number };
type Grain = Point & { left: number; top: number; width: number; height: number };
const umSpace = '[data-testid="canvas-um-space"]';
const outlines = `${umSpace} polygon.grain:not(.artifact):not(.dup)`;

async function frames(page: Page) {
  await page.evaluate(() => new Promise<void>(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
}

async function settle(page: Page) {
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.locator('[data-live="busy"]')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: /^rendering…$/ })).toHaveCount(0);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await frames(page);
  await page.waitForTimeout(300);
  // Recheck after debounced live tone rendering has had time to start.
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.locator('[data-live="busy"]')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: /^rendering…$/ })).toHaveCount(0);
  await frames(page);
  await page.waitForTimeout(300);
}

async function screen(page: Page, name: string) {
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name, exact: true }).click();
  await settle(page);
}

async function fit(page: Page) {
  const view = page.locator('[data-view]');
  if (await view.count()) {
    await page.getByRole('button', { name: 'Fit', exact: true }).click();
    await settle(page);
    if (await page.locator(umSpace).count()) {
      // Canvas panels float over the viewer. Fit its entire image into the
      // uncovered rectangle, rather than leaving the right end under a table.
      const rect = await view.boundingBox();
      if (!rect) throw new Error('Canvas is not visible');
      const room = await workspace(page);
      await pan(page, room.x - (rect.x + rect.width / 2), room.y - (rect.y + rect.height / 2));
      await zoom(page, room, Math.min(room.width / rect.width, room.height / rect.height));
      await settle(page);
    }
  }
}

// All captures go through this helper. Detail preparations run again after each
// resize, so a lasso is cancelled before the next viewport is fitted.
async function shots(page: Page, name: string, theme: Theme,
  prepare?: () => Promise<void>, cleanup?: () => Promise<void>) {
  for (const viewport of VIEWPORTS) {
    await page.setViewportSize(viewport);
    await settle(page);
    await fit(page);
    if (prepare) await prepare();
    if (name !== 'outline') await page.mouse.move(10, 10);
    await settle(page);
    await expect(page.locator('.ga-root')).toHaveAttribute('data-theme', theme);
    await expect(page.getByRole('alert')).toHaveCount(0);
    // Browser directory uploads expose relative file names only. Catch an
    // accidentally displayed local path, including one introduced by a stub.
    const text = await page.locator('body').innerText();
    expect(text, 'A local absolute path is visible; inspect before capture').not.toMatch(
      /(?:\/Users\/|\/Volumes\/|\/home\/|[A-Z]:\\|file:\/\/)/,
    );
    const filename = `${name}-${viewport.width}x${viewport.height}-${theme}.png`;
    await page.screenshot({ path: path.join(OUTPUT, filename), fullPage: false });
    console.log(`Wrote target/screenshots/${filename}`);
    if (cleanup) await cleanup();
  }
}

async function initialise(page: Page, theme: Theme) {
  // Keep generated spot timestamps and the print header reproducible without
  // freezing timers, workers, animation frames or the application's polling.
  await page.clock.setFixedTime(new Date('2026-01-01T12:00:00Z'));
  // Install before goto on every page, including the separate Settings page.
  await page.addInitScript(t => localStorage.setItem('grainatlas.theme', t), theme);
  await page.addInitScript(() => {
    localStorage.setItem('grainatlas.settings', JSON.stringify({
      naming_pattern: '{sample}_{n}', n_per_sample: false, user: 'Example user',
    }));
  });
}

// Read only geometry actually drawn by the app, in mosaic micrometres. No
// Svelte internals, development-only exports, source imports or image fixtures.
async function grains(page: Page): Promise<Grain[]> {
  return page.locator(outlines).evaluateAll(polygons => polygons.flatMap(node => {
    const polygon = node as SVGPolygonElement;
    const box = polygon.getBBox();
    if (box.width < 15 || box.height < 15) return [];
    const points = Array.from(polygon.points, p => ({ x: p.x, y: p.y }));
    let area = 0, x = 0, y = 0;
    for (let i = 0; i < points.length; i++) {
      const a = points[i], b = points[(i + 1) % points.length];
      const cross = a.x * b.y - b.x * a.y;
      area += cross; x += (a.x + b.x) * cross; y += (a.y + b.y) * cross;
    }
    if (Math.abs(area) < 400) return [];
    x /= 3 * area; y /= 3 * area;
    // A concave outline's centroid can fall outside it. Choose an interior
    // grid point nearest the centroid in that case.
    if (!polygon.isPointInFill(new DOMPoint(x, y))) {
      const candidates: Point[] = [];
      for (let row = 1; row < 10; row++) for (let col = 1; col < 10; col++) {
        const p = new DOMPoint(box.x + box.width * col / 10, box.y + box.height * row / 10);
        if (polygon.isPointInFill(p)) candidates.push({ x: p.x, y: p.y });
      }
      candidates.sort((a, b) => Math.hypot(a.x - x, a.y - y) - Math.hypot(b.x - x, b.y - y));
      if (!candidates.length) return [];
      ({ x, y } = candidates[0]);
    }
    return [{ x, y, left: box.x, top: box.y, width: box.width, height: box.height }];
  }));
}

function bounds(group: Grain[]) {
  const left = Math.min(...group.map(g => g.left));
  const top = Math.min(...group.map(g => g.top));
  const right = Math.max(...group.map(g => g.left + g.width));
  const bottom = Math.max(...group.map(g => g.top + g.height));
  return { x: (left + right) / 2, y: (top + bottom) / 2, width: right - left, height: bottom - top };
}

function cluster(all: Grain[], count: number): Grain[] {
  expect(all.length, `Need at least ${count} usable grain outlines`).toBeGreaterThanOrEqual(count);
  // Minimise the extent of nearest-neighbour groups, not the distance to the
  // image centre: Sample-A is a long strip with uneven grain density.
  const groups = all.map(seed => [...all].sort((a, b) =>
    Math.hypot(a.x - seed.x, a.y - seed.y) - Math.hypot(b.x - seed.x, b.y - seed.y)).slice(0, count));
  groups.sort((a, b) => {
    const A = bounds(a), B = bounds(b);
    return Math.max(A.width, A.height) - Math.max(B.width, B.height);
  });
  return groups[0];
}

async function toScreen(page: Page, point: Point): Promise<Point> {
  return page.locator(umSpace).evaluate((node, p) => {
    const matrix = (node as SVGGraphicsElement).getScreenCTM();
    if (!matrix) throw new Error('Canvas transform is not ready');
    const result = new DOMPoint(p.x, p.y).matrixTransform(matrix);
    return { x: result.x, y: result.y };
  }, point);
}

async function workspace(page: Page) {
  return page.locator('.canvas').evaluate(node => {
    const view = node.querySelector('[data-view]')!.getBoundingClientRect();
    const side = node.querySelector('.side')!.getBoundingClientRect();
    const toolbar = node.querySelector('.toolbar')!.getBoundingClientRect();
    const cards = Array.from(node.querySelectorAll('.floating')).map(el => el.getBoundingClientRect());
    const left = Math.max(view.left + 24, ...cards.map(r => r.right + 24));
    const right = side.left - 24;
    const top = toolbar.bottom + 24;
    const bottom = view.bottom - 65;
    return { x: (left + right) / 2, y: (top + bottom) / 2, width: right - left, height: bottom - top };
  });
}

async function pan(page: Page, dx: number, dy: number) {
  const target = await workspace(page);
  await page.mouse.move(target.x, target.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(target.x + dx, target.y + dy, { steps: 8 });
  await page.mouse.up({ button: 'middle' });
  await frames(page);
}

async function zoom(page: Page, centre: Point, factor: number) {
  let logFactor = Math.log(factor);
  await page.mouse.move(centre.x, centre.y);
  await page.keyboard.down('Control');
  try {
    while (Math.abs(logFactor) > 0.001) {
      const step = Math.max(-0.5, Math.min(0.5, logFactor));
      await page.mouse.wheel(0, -step / 0.01);
      await frames(page);
      logFactor -= step;
    }
  } finally { await page.keyboard.up('Control'); }
}

async function canvasScales(page: Page) {
  return page.locator(umSpace).evaluate(node => {
    const um = (node as SVGGraphicsElement).getScreenCTM()!;
    const px = (node.parentElement as unknown as SVGGraphicsElement).getScreenCTM()!;
    return { um: Math.hypot(um.a, um.b), px: Math.hypot(px.a, px.b) };
  });
}

async function frameCluster(page: Page, group: Grain[], zoomRange?: [number, number]) {
  const box = bounds(group);
  const room = await workspace(page);
  expect(room.width, 'Space between the info card and side panel').toBeGreaterThan(120);
  const scale = await canvasScales(page);
  let wanted = Math.min(room.width / (box.width * 1.4), room.height / (box.height * 1.4));
  if (zoomRange) {
    const umPerPx = scale.px / scale.um;
    wanted = Math.max(zoomRange[0] / umPerPx, Math.min(zoomRange[1] / umPerPx, wanted));
  }
  const centre = await toScreen(page, box);
  await pan(page, room.x - centre.x, room.y - centre.y);
  // Ctrl+wheel follows gestures.ts's pinch path on Chromium, independent of
  // whether a normal wheel event is classified as a mouse or a trackpad.
  await zoom(page, room, wanted / scale.um);
  await settle(page);
}

async function frameGrains(page: Page, all: Grain[]) {
  await page.mouse.move(10, 10);
  await settle(page); // Clear any hover card before measuring the usable view.
  const room = await workspace(page);
  const scale = await canvasScales(page);
  const umPerPx = scale.px / scale.um;
  const area = bounds(all);
  // Search the strip at readable native-pixel zooms. Prefer about 55 grains
  // in the uncovered canvas, with denser regions winning ties at higher zoom.
  let best = { centre: all[0] as Point, zoom: 0.4, count: 0, score: Infinity };
  for (const pixelZoom of [0.4, 0.375, 0.35, 0.325, 0.3, 0.275, 0.25]) {
    const halfWidth = room.width * umPerPx / pixelZoom / 2;
    const halfHeight = room.height * umPerPx / pixelZoom / 2;
    const maxOffset = Math.max(0, area.width / 2 - halfWidth);
    for (const seed of all) {
      const centre = {
        x: Math.max(area.x - maxOffset, Math.min(area.x + maxOffset, seed.x)),
        y: area.y,
      };
      const count = all.filter(g => Math.abs(g.x - centre.x) < halfWidth
        && Math.abs(g.y - centre.y) < halfHeight).length;
      const score = Math.abs(count - 55) + (0.4 - pixelZoom);
      if (score < best.score) best = { centre, zoom: pixelZoom, count, score };
    }
  }
  expect(best.count, 'Grains visible in the detail overview').toBeGreaterThanOrEqual(40);
  expect(best.count, 'Grains visible in the detail overview').toBeLessThanOrEqual(70);
  const centre = await toScreen(page, best.centre);
  await pan(page, room.x - centre.x, room.y - centre.y);
  await zoom(page, room, best.zoom / scale.px);
  await settle(page);
}

async function clickGrain(page: Page, grain: Grain) {
  const p = await toScreen(page, grain);
  const clear = await page.evaluate(p => {
    const el = document.elementFromPoint(p.x, p.y);
    return !!el?.closest('[data-view]') && !el.closest('[data-ui]');
  }, p);
  expect(clear, 'The grain centre must be visible and clear of panels').toBe(true);
  await page.mouse.click(p.x, p.y);
}

async function detailOutline(page: Page, group: Grain[]) {
  await page.getByTestId('tool-select').click();
  await frameCluster(page, group, [1.5, 2]);
  await clickGrain(page, group[0]);
  await expect(page.getByRole('dialog', { name: 'Grain', exact: true })).toBeVisible();
  // The selected grain's card now occupies space; frame again around it.
  await frameCluster(page, group, [1.5, 2]);
  await page.getByTestId('tool-outline').click();
  await page.locator('tr[data-grain-id].selected').evaluate(row => {
    row.scrollIntoView({ block: 'center' });
  });
  await frames(page);
  const g = group[0];
  const corners = [
    { x: g.left - g.width * 0.12, y: g.top + g.height * 0.65 },
    { x: g.left - g.width * 0.12, y: g.top - g.height * 0.12 },
    { x: g.left + g.width * 0.6, y: g.top - g.height * 0.12 },
  ];
  for (const corner of corners) {
    const p = await toScreen(page, corner);
    await page.mouse.click(p.x, p.y);
    await frames(page);
  }
  const end = await toScreen(page, { x: g.left + g.width * 1.12, y: g.top + g.height * 0.3 });
  await page.mouse.move(end.x, end.y, { steps: 5 });
  await expect(page.getByTestId('lasso')).toBeVisible();
}

async function settingsShot(browser: Browser, theme: Theme) {
  const context = await browser.newContext({
    viewport: VIEWPORTS[0], deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce',
  });
  const page = await context.newPage();
  page.setDefaultTimeout(120_000);
  page.setDefaultNavigationTimeout(120_000);
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await initialise(page, theme);
    await page.addInitScript(() => {
      let callback = 0;
      // Only the empty Settings page uses this. Never open pixels through it.
      Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
        transformCallback: () => ++callback,
        unregisterCallback: () => {},
        invoke: async (command: string) => {
          switch (command) {
            case 'startup': return { paths: [], stitch: false, segment: false, um_per_px: null, field_um: null, profile: null };
            case 'take_opened': return [];
            case 'store_list': return { path: 'Example app storage', keys: [] };
            case 'store_read': return null;
            case 'plugin:app|version': return '0.1.1';
            case 'update_supported': return false;
            case 'plugin:event|listen': return ++callback;
            default: return null;
          }
        },
      } });
      Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
        value: { unregisterListener: () => {} },
      });
    });
    await page.goto('http://localhost:4173/');
    await screen(page, 'Settings');
    await expect(page.getByTestId('server')).toBeVisible();
    await page.getByTestId('server').fill(SERVER);
    await page.getByTestId('server').blur();
    await expect(page.getByTestId('sign-in')).toBeEnabled();
    await shots(page, 'settings', theme, async () => {
      // A viewport capture centred on the UPbDB section keeps Server and
      // Sign in visible at both sizes without changing the application's CSS.
      await page.getByTestId('server').evaluate(input => {
        input.closest('.panel')!.scrollIntoView({ block: 'start' });
      });
      await expect(page.getByTestId('server')).toBeInViewport();
      await expect(page.getByTestId('sign-in')).toBeInViewport();
    });
    expect(errors, 'Settings desktop stub browser errors').toEqual([]);
  } finally { await context.close(); }
}

async function mockSync(page: Page, areaId: string) {
  const mount = '11111111-1111-4111-8111-111111111111';
  const label = '22222222-2222-4222-8222-222222222222';
  const hashes = { spots: '0'.repeat(64), grains: '1'.repeat(64), labels: '2'.repeat(64) };
  let linked = false;
  const unexpected: string[] = [];
  await page.route(`${SERVER}/**`, async route => {
    const request = route.request();
    const url = new URL(request.url());
    const endpoint = url.pathname;
    const method = request.method();
    const json = (body: unknown, status = 200) => route.fulfill({
      status, json: body, headers: { 'access-control-allow-origin': '*' },
    });
    if (method === 'OPTIONS') {
      await route.fulfill({ status: 204, headers: {
        'access-control-allow-origin': '*', 'access-control-allow-headers': 'authorization,content-type',
        'access-control-allow-methods': 'GET,POST,OPTIONS',
      } });
      return;
    }
    if (method === 'GET' && endpoint === '/v1/atlas/projects') {
      await json([{ project_uuid: null, mount_id: mount, mount_label: 'Example mount', matched_by: 'package',
        atlas_version: null, revision: 3, spot_count: 0, grain_count: 0, can_edit: true }]);
      return;
    }
    const project = endpoint.match(/^\/v1\/atlas\/projects\/([^/]+)(?:\/(link|elements))?$/);
    if (project) {
      const uuid = decodeURIComponent(project[1]); // The actual local project's identity.
      const status = {
        project_uuid: uuid, mount_id: mount, mount_label: 'Example mount', name: 'Example mount',
        package_name: 'Example_mount', atlas_version: null, atlas_bundle_id: null, revision: 3,
        counts: { spots: 0, grains: 0, labels: 1, grains_retired: 0, grains_without_atlas_id: 0 },
        hashes, changed_at: '2026-01-01T12:00:00Z', changed_by: 'Example user', can_edit: true,
        current_project_uuid: uuid,
        project_uuids: [{ project_uuid: uuid, name: 'Example mount', package_name: 'Example_mount', is_current: true }],
      };
      if (method === 'POST' && project[2] === 'link') {
        expect(request.postDataJSON()).toEqual({ mount_id: mount, make_current: false });
        linked = true;
        await json(status); return;
      }
      if (method === 'GET' && !project[2]) {
        await json(linked ? status : { detail: 'Not registered' }, linked ? 200 : 404); return;
      }
      if (method === 'GET' && project[2] === 'elements' && linked) {
        await json({ mount_id: mount, revision: 3, hashes, project_uuids: [uuid], current_project_uuid: uuid,
          spots: [], grains: [], labels: [{ id: label, project_uuid: uuid, area_id: areaId,
            x_um: 100, y_um: 100, text: 'Example note' }] });
        return;
      }
    }
    if (method === 'GET' && /^\/atlas\/projects\/[^/]+\/project\.json$/.test(endpoint)) {
      await json({ detail: 'No published pixels in this example' }, 404); return;
    }
    if (method === 'GET' && endpoint === `/v1/mounts/${mount}/results`) {
      await json({ mount_id: mount, spots: [] }); return;
    }
    if (method === 'GET' && endpoint === '/v1/sessions') { await json([]); return; }
    if (method === 'GET' && endpoint === '/v1/mounts') {
      await json([{ id: mount, label: 'Example mount', mount_type: 'grain', grain_count: 0, spot_count: 0, can_edit: true }]);
      return;
    }
    unexpected.push(`${method} ${endpoint}`);
    await json({ detail: 'Unimplemented screenshot mock endpoint' }, 501);
  });
  return unexpected;
}

for (const theme of ['light', 'dark'] as const) {
  test(`documentation screenshots (${theme})`, async ({ page, browser }) => {
    expect((await stat(DEMO_PACKAGE)).isDirectory(), 'Place the neutralised package as described in docs/screenshots.md').toBe(true);
    await mkdir(OUTPUT, { recursive: true });
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await initialise(page, theme);
    await page.goto('/');
    await page.setInputFiles('[data-testid=package-input]', DEMO_PACKAGE);
    await expect(page.getByTestId('area-view')).toBeVisible();
    await settle(page);
    const areas = page.locator('button.arearow[data-testid^="area-"]');
    await expect(areas).toHaveCount(2);
    await areas.filter({ hasText: 'Sample-A' }).click();
    await expect(areas.filter({ hasText: 'Sample-B' })).toBeVisible();
    await expect(page.getByTestId('stitch')).toHaveText('Stitch');
    await expect(page.getByTestId('area-view').locator('.tile image')).toHaveCount(20);
    // Open.svelte deliberately navigates to Area after reading the package;
    // this is the actual post-open view with areas and tile/navigation overview.
    await shots(page, 'open', theme);

    await page.getByRole('button', { name: 'Stitch all areas', exact: true }).click();
    await expect(areas.filter({ hasText: 'stitched' })).toHaveCount(2);
    await settle(page);
    await page.getByTestId('auto-tone').click();
    await settle(page);
    await page.getByRole('checkbox', { name: 'residuals', exact: true }).check();
    await shots(page, 'stitch', theme);

    await screen(page, 'Canvas');
    await page.getByRole('button', { name: /^Grains\s+\d+$/ }).click();
    await page.getByText('Segmentation', { exact: true }).click();
    await page.getByTestId('signal').selectOption('cl');
    await page.getByTestId('polarity').selectOption('both');
    await page.getByTestId('find-grains').click();
    await settle(page);
    const foundNotice = page.getByRole('status').filter({ hasText: /^Found \d+ grains? in Sample-A/ });
    if (await foundNotice.count()) {
      await foundNotice.getByRole('button', { name: 'Dismiss', exact: true }).click();
    }
    await page.getByText('Segmentation', { exact: true }).click();
    await page.getByText('Spot entry', { exact: true }).click();
    await page.getByText('Layers', { exact: true }).click();
    await page.getByRole('checkbox', { name: 'numbers', exact: true }).check();
    await page.getByText('Layers', { exact: true }).click();
    await expect(page.locator('tr[data-grain-id]').first()).toBeVisible();
    await expect(page.locator(`${umSpace} text.gnum`).first()).toBeVisible();
    const all = await grains(page);
    await shots(page, 'grains', theme, () => frameGrains(page, all));
    const detail = cluster(all, 3);
    await shots(page, 'outline', theme, () => detailOutline(page, detail), async () => {
      await page.keyboard.press('Escape');
      await expect(page.getByTestId('lasso')).toHaveCount(0);
    });

    await page.getByRole('button', { name: /^Spots\s+\d+$/ }).click();
    if (await page.getByTestId('sample').inputValue() !== 'Sample-A') {
      await page.getByTestId('sample').fill('Sample-A');
      await page.getByTestId('sample').blur();
    }
    await page.getByTestId('area-pattern').fill('{sample}_{n}');
    await page.getByTestId('area-pattern').blur();
    const spotGrains = cluster(all, 8);
    await frameCluster(page, spotGrains);
    await page.getByTestId('tool-spot').click();
    for (const [index, grain] of spotGrains.entries()) {
      // Alt is the UI's free-placement modifier. Keep the proven interior
      // point even if a concave outline has its snap centroid outside it.
      await page.keyboard.down('Alt');
      try { await clickGrain(page, grain); }
      finally { await page.keyboard.up('Alt'); }
      await expect(page.getByTestId('panel-name')).toHaveValue(`Sample-A_${index + 1}`);
      await expect(page.getByRole('dialog', { name: `Spot Sample-A_${index + 1}`, exact: true })
        .getByRole('cell', { name: 'on a grain', exact: true })).toBeVisible();
    }
    await expect(page.locator('tr[data-spot-id]')).toHaveCount(8);
    await expect(page.locator('tr[data-spot-id].selected')).toHaveCount(1);
    await shots(page, 'spots', theme, async () => {
      await frameCluster(page, spotGrains);
      await page.mouse.move(10, 10);
      await expect(page.locator('td.spot-name')).toHaveText(
        Array.from({ length: 8 }, (_, i) => `Sample-A_${i + 1}`));
    });

    await screen(page, 'Layout');
    await page.getByTestId('style-sheets').check();
    await page.getByRole('textbox', { name: 'Lab and operator', exact: true }).fill('');
    await page.getByTestId('sheets').click();
    await expect(page.getByTestId('page')).toBeVisible();
    await expect(page.getByTestId('sheet-stats')).toBeVisible();
    await shots(page, 'print', theme, async () => {
      await page.getByTestId('page').evaluate(async node => { await (node as HTMLImageElement).decode(); });
      await page.getByTestId('sheet-stats').scrollIntoViewIfNeeded();
    });

    await settingsShot(browser, theme);

    await screen(page, 'Canvas');
    const areaId = await page.getByRole('combobox', { name: 'Area', exact: true }).inputValue();
    const unexpected = await mockSync(page, areaId);
    await screen(page, 'Settings');
    await page.getByTestId('server').fill(SERVER);
    await page.getByTestId('server').blur();
    await page.getByTestId('token').fill('example-screenshot-token');
    await page.getByTestId('token').blur();
    await screen(page, 'Canvas');
    // The 30 s background check may already have found the offered mount.
    // Otherwise Retry explicitly checks after the initial no-server failure.
    await expect(page.getByTestId('sync-chip')).toHaveAttribute('aria-busy', 'false');
    if ((await page.getByTestId('sync-chip').innerText()).includes('Offline')) {
      await page.getByTestId('sync-action').click();
    }
    await expect(page.getByTestId('sync-chip')).toContainText('Not in the database');
    await page.getByTestId('sync-chip').click();
    await page.getByTestId('sync-link-offer').getByRole('button', { name: 'Link', exact: true }).click();
    await expect(page.getByTestId('sync-chip')).toContainText('Database has more');
    await expect(page.getByTestId('sync-chip')).toHaveAttribute('aria-busy', 'false');
    await shots(page, 'sync', theme, async () => {
      await frameCluster(page, spotGrains);
      if (await page.getByTestId('sync-chip').getAttribute('aria-expanded') !== 'true') {
        await page.getByTestId('sync-chip').click();
      }
      await expect(page.getByTestId('sync-details')).toBeVisible();
      await expect(page.getByTestId('sync-details')).toContainText('r3');
      await expect(page.getByTestId('sync-details')).toContainText('Example user');
      await expect(page.getByTestId('sync-details')).toContainText('Example note');
    });
    expect(unexpected, 'Unimplemented mock requests').toEqual([]);
    expect(errors, 'Browser errors during capture').toEqual([]);
  });
}
