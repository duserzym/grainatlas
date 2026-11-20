import type { Page } from '@playwright/test';
import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mockTimsServer, mountId, projectId, server } from './support/tims';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' });
});
async function prepare(page: Page) {
  const mock = await mockTimsServer(page, generated);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  await page.getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect(page.getByTestId('tool-pluck')).toBeVisible();
  const layers = page.locator('details.layers');
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = false; });
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  return mock;
}
async function point(page: Page, x: number, y: number) {
  return page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, [x, y]) => {
    const matrix = g.getScreenCTM(); if (!matrix) throw new Error('Missing canvas transform');
    const p = new DOMPoint(x, y).matrixTransform(matrix); return { x: p.x, y: p.y };
  }, [x, y]);
}
async function grain(page: Page, x = 80) { const p = await point(page, x, 95); await page.mouse.click(p.x, p.y); }
async function close(page: Page) { await page.getByRole('dialog', { name: /(?:Pluck for TIMS|Edit TIMS pluck)/ }).getByRole('button', { name: 'Cancel', exact: true }).click(); }
async function chooseCapsule(page: Page) {
  const panel = page.getByTestId('pluck-panel');
  await panel.getByLabel('Capsule set', { exact: true }).selectOption('40000000-0000-4000-8000-000000000001');
  await panel.getByLabel('Batch', { exact: true }).selectOption('batch-1');
  return panel;
}
async function fits(page: Page) {
  const dialog = page.getByRole('dialog', { name: /(?:Pluck for TIMS|Edit TIMS pluck)/ });
  const box = await dialog.boundingBox(); expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0); expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(1280); expect(box!.y + box!.height).toBeLessThanOrEqual(800);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
}

test('Pluck tool adds a capsule badge, edits a pluck and retires it with history confirmation', async ({ page }) => {
  const mock = await prepare(page);
  await page.getByTestId('tool-pluck').click(); await grain(page);
  const panel = await chooseCapsule(page); await fits(page);
  await panel.getByLabel('Fraction name', { exact: true }).fill('z3');
  await panel.getByLabel('Capsule number', { exact: true }).selectOption('3');
  await panel.getByRole('button', { name: 'Pluck', exact: true }).click();
  await expect(page.getByTestId('tims-badge')).toHaveText('K3 · z3');
  await expect.poll(() => mock.fractions.length).toBe(1);
  await panel.getByLabel('Notes', { exact: true }).fill('Capsule checked');
  await panel.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => mock.fractions[0].notes).toBe('Capsule checked');
  expect(mock.fractions).toHaveLength(1); await close(page);
  await grain(page); await expect(page.getByRole('dialog', { name: 'Edit TIMS pluck' })).toBeVisible();
  await panel.getByLabel('Fraction name', { exact: true }).fill('z4'); await panel.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByTestId('tims-badge')).toHaveText('K3 · z4'); await close(page);
  const layers = page.locator('details.layers'); await layers.evaluate((d: HTMLDetailsElement) => { d.open = true; }); await page.getByLabel('info panel', { exact: true }).check(); await layers.evaluate((d: HTMLDetailsElement) => { d.open = false; });
  await page.getByTestId('tool-select').click(); const p = await point(page, 80, 95); await page.mouse.move(p.x, p.y);
  await expect(page.getByTestId('tims-section')).toBeVisible();
  page.once('dialog', dialog => { expect(dialog.message()).toContain('database history'); void dialog.accept(); });
  await page.getByTestId('tims-section').getByRole('button', { name: 'Remove pluck', exact: true }).click();
  await expect(page.getByTestId('tims-badge')).toHaveCount(0);
  await expect.poll(() => mock.fractions[0].retired_at).toBeTruthy(); expect(mock.unexpected).toEqual([]);
});

test('right-click a spot, require a fragment note, and display a duplicate-name refusal verbatim', async ({ page }) => {
  const mock = await prepare(page);
  const p = await point(page, 75, 120); await page.mouse.click(p.x, p.y, { button: 'right' });
  await page.getByRole('menuitem', { name: 'Pluck for TIMS…' }).click();
  const panel = page.getByTestId('pluck-panel'); await panel.getByLabel('Fragment', { exact: true }).check();
  await expect(panel.getByRole('button', { name: 'Pluck', exact: true })).toBeDisabled();
  await expect(panel.getByTestId('pluck-reason')).toHaveText('Add a fragment note');
  await panel.getByLabel('Fragment note', { exact: true }).fill('Tip removed');
  await panel.getByLabel('Fraction name', { exact: true }).fill('z1'); await panel.getByRole('button', { name: 'Pluck', exact: true }).click();
  await expect(page.getByTestId('tims-badge')).toContainText('z1†'); await close(page);
  expect(mock.fractions[0].sources[0].atlas_spot_id).toBe(mock.ui.spots[0].id);
  await page.getByTestId('tool-pluck').click(); await grain(page, 240);
  await panel.getByLabel('Fraction name', { exact: true }).fill('z2'); await panel.getByRole('button', { name: 'Pluck', exact: true }).click(); await close(page);
  await grain(page, 240); await panel.getByLabel('Fraction name', { exact: true }).fill('z1'); await panel.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveText('That fraction name is already used for this sample');
  expect(mock.fractions[1].name).toBe('z2');
  await expect(page.getByTestId('tims-badge').filter({ hasText: 'z2' })).toHaveClass(/failed/);
  await panel.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveCount(0); expect(mock.unexpected).toEqual([]);
});

test('offline pluck shows a dashed badge and retries the same key when online', async ({ page }) => {
  const mock = await prepare(page);
  await page.getByTestId('tool-pluck').click(); await grain(page);
  const panel = await chooseCapsule(page);
  mock.setOffline(true);
  await panel.getByLabel('Fraction name', { exact: true }).fill('z5'); await panel.getByRole('button', { name: 'Pluck', exact: true }).click();
  await expect(panel.getByRole('status')).toHaveText('Saved on this computer; sends when online');
  await expect(page.getByTestId('tims-badge')).toHaveClass(/pending/);
  const key = mock.writes.find(w => w.path === '/v1/fractions')?.key; expect(key).toBeTruthy();
  await close(page);
  mock.setOffline(false); await page.evaluate(() => window.dispatchEvent(new Event('online')));
  await expect.poll(() => mock.fractions.length).toBe(1);
  await expect(page.getByTestId('tims-badge')).not.toHaveClass(/pending/);
  expect(mock.writes.filter(w => w.path === '/v1/fractions').every(w => w.key === key)).toBe(true);
  expect(mock.unexpected).toEqual([]);
});
