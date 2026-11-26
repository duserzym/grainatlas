// Invented bundle and registered spot only. The supervisor runs the browser tests.
import { test, expect } from './mock-session';
import type { Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type { Grain, Spot } from '../src/lib/core/types';
import { contains } from '../src/lib/polygon-edit';
import { root, generated, mockImageServer, projectURL, saveImageProject } from './support/image-edit';

test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'], { cwd: root, stdio: 'pipe' });
});

type Saved = { grainatlas_ui: { grains: Record<string, Grain[]>; spots: Spot[] } };
async function saved(page: Page): Promise<Saved> { return JSON.parse(await saveImageProject(page)); }
async function screen(page: Page, points: number[][]) {
  return page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, points) => {
    const matrix = g.getScreenCTM();
    if (!matrix) throw new Error('Missing micrometre transform');
    return points.map(([x, y]) => {
      const p = new DOMPoint(x, y).matrixTransform(matrix);
      return { x: p.x, y: p.y };
    });
  }, points);
}
async function prepare(page: Page) {
  const source = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  // Grain 13 sits near the middle of the canvas, away from its chrome.
  const parent: Grain = source.grainatlas_ui.grains.a[12];
  source.grainatlas_ui.spots = [{
    id: '50000000-0000-4000-8000-000000000001', name: 'registered-spot-keep-name',
    area_id: 'a', grain_id: parent.id, grain_number: parent.number, sample: 'A1', n: 1,
    x_um: parent.centroid_um[0] - 20, y_um: parent.centroid_um[1] - 20,
    diameter_um: 10, kind: 'spot', comment: '', created: '', created_by: '',
  }];
  const unexpected = await mockImageServer(page, { saved: JSON.stringify(source), syncOnSave: false });
  await page.goto(projectURL);
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  const layers = page.locator('details.layers');
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await page.getByLabel('outlines', { exact: true }).check();
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = false; });
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await page.locator('.side .tabs').getByRole('tab', { name: /^Grains / }).click();
  // A larger view makes the curve clearly visible even on a 14-inch laptop.
  await page.getByTitle('Zoom in (+)', { exact: true }).click();
  await page.getByTestId('tool-split').click();
  const before = await saved(page);
  return { parent: before.grainatlas_ui.grains.a.find(g => g.id === parent.id)!, before, unexpected };
}

async function draw(page: Page, parent: Grain, shift = false) {
  const [cx, cy] = parent.centroid_um;
  const path = Array.from({ length: 41 }, (_, i) => {
    const t = i / 40;
    return [cx - 50 + 100 * t, cy + 16 * Math.sin(2 * Math.PI * t)];
  });
  const points = await screen(page, path);
  for (const p of [points[0], points.at(-1)!]) {
    expect(await page.evaluate(p => !!document.elementFromPoint(p.x, p.y)?.closest('[data-view]'), p)).toBe(true);
  }
  await page.mouse.move(points[0].x, points[0].y);
  await page.mouse.down();
  // Start curved, then press Shift without moving: the live preview must become
  // the press-to-pointer segment immediately, rather than starting a box.
  for (const p of points.slice(1, 20)) await page.mouse.move(p.x, p.y, { steps: 2 });
  const preview = page.locator('polyline.splitline');
  await expect(preview).toHaveCount(1);
  expect((await preview.getAttribute('points'))!.split(' ').length).toBeGreaterThan(2);
  if (shift) {
    await page.keyboard.down('Shift');
    await expect.poll(async () => (await preview.getAttribute('points'))!.split(' ').length).toBe(2);
  }
  for (const p of points.slice(20)) await page.mouse.move(p.x, p.y, { steps: 2 });
  if (shift) expect((await preview.getAttribute('points'))!.split(' ').length).toBe(2);
  await page.mouse.up();
  if (shift) await page.keyboard.up('Shift');
  await expect(preview).toHaveCount(0);
}

async function splitAndUndo(page: Page, shift: boolean, undoKey: string) {
  const { parent, before, unexpected } = await prepare(page);
  await draw(page, parent, shift);
  await expect(page.locator('polygon.grain.sel')).toHaveCount(2);
  const after = await saved(page), grains = after.grainatlas_ui.grains.a;
  expect(grains).toHaveLength(before.grainatlas_ui.grains.a.length + 1);
  expect(grains.some(g => g.id === parent.id)).toBe(false);
  const children = grains.filter(g => g.split_from === parent.id).sort((a, b) => b.area_um2 - a.area_um2);
  expect(children.map(g => g.label)).toEqual([`${parent.label}a`, `${parent.label}b`]);
  expect(new Set(children.map(g => g.id)).size).toBe(2);
  for (const g of children) {
    expect(g).toMatchObject({ number: parent.number, split_from: parent.id });
    expect(g.sample ?? null).toEqual(parent.sample ?? null);
  }
  const original = before.grainatlas_ui.spots[0], spot = after.grainatlas_ui.spots.find(s => s.id === original.id)!;
  const containing = children.find(g => contains(g.polygon_um, [spot.x_um, spot.y_um]))!;
  expect(containing).toBeDefined();
  expect(spot).toEqual({ ...original, grain_id: containing.id, grain_number: containing.number });
  // The saved cut follows the drag, except while Shift holds the straight chord.
  const [x0, y0, x1, y1] = parent.bbox_um, cy = parent.centroid_um[1];
  const cutVertices = children.flatMap(g => g.polygon_um).filter(([x, y]) => x >= x0 - .01 && x <= x1 + .01 && y > y0 + .01 && y < y1 - .01);
  expect(cutVertices.length).toBeGreaterThan(0);
  if (shift) expect(cutVertices.every(([, y]) => Math.abs(y - cy) < .01)).toBe(true);
  else expect(cutVertices.some(([, y]) => Math.abs(y - cy) > 5)).toBe(true);
  await page.keyboard.press(undoKey);
  const restored = await saved(page);
  expect(restored.grainatlas_ui.grains.a).toEqual(before.grainatlas_ui.grains.a);
  expect(restored.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
  expect(unexpected).toEqual([]);
}

test('freehand split preserves registered spot identity and undo restores its parent', async ({ page }) => {
  await splitAndUndo(page, false, 'Meta+z');
});
test('Shift pressed during a drag makes a straight cut', async ({ page }) => {
  await splitAndUndo(page, true, 'Meta+z');
});
test('Escape and blur cancel split strokes without changing registered data', async ({ page }) => {
  const { parent, before, unexpected } = await prepare(page);
  const [a, b] = await screen(page, [[parent.bbox_um[0] - 10, parent.centroid_um[1]], [parent.bbox_um[2] + 10, parent.centroid_um[1]]]);
  for (const cancel of ['escape', 'blur']) {
    await page.mouse.move(a.x, a.y); await page.mouse.down();
    await page.mouse.move(b.x, b.y, { steps: 20 });
    await expect(page.locator('polyline.splitline')).toHaveCount(1);
    if (cancel === 'escape') await page.keyboard.press('Escape');
    else await page.evaluate(() => window.dispatchEvent(new Event('blur')));
    await expect(page.locator('polyline.splitline')).toHaveCount(0);
    await page.mouse.up();
    const after = await saved(page);
    expect(after.grainatlas_ui.grains).toEqual(before.grainatlas_ui.grains);
    expect(after.grainatlas_ui.spots).toEqual(before.grainatlas_ui.spots);
  }
  expect(unexpected).toEqual([]);
});

test.describe('Windows Edge / WebView2 input', () => {
  test.use({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0' });
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'platform', { configurable: true, get: () => 'Win32' });
      if ('userAgentData' in navigator) Object.defineProperty(navigator.userAgentData, 'platform', { configurable: true, get: () => 'Windows' });
    });
  });
  test('freehand split and Ctrl undo preserve the registered spot', async ({ page }) => {
    await splitAndUndo(page, false, 'Control+z');
  });
  test('Shift has the same straight-cut behavior on Windows', async ({ page }) => {
    await splitAndUndo(page, true, 'Control+z');
  });
});
