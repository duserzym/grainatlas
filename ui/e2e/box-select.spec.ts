// Invented large bundle only. The supervisor runs these tests with a preview server.
import type { Page } from '@playwright/test';
import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import type { Grain } from '../src/lib/core/types';
import { generated, mockLargeServer, openEdit, root, savedProject } from './support/perf';

test.use({ viewport: { width: 1440, height: 900 } });
test.beforeAll(() => {
  test.setTimeout(600_000);
  if (!existsSync(path.join(generated, 'atlas.json'))) {
    execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'large_bundle'],
      { cwd: root, stdio: 'pipe', timeout: 600_000 });
  }
});

type Point = { x: number; y: number };
type Box = { start: Point; end: Point };
type Mock = Awaited<ReturnType<typeof mockLargeServer>>;
const areaId = 'a01';

async function tab(page: Page, name: 'Grains' | 'Spots') {
  await page.locator('.side .tabs').getByRole('button', { name: new RegExp(`^${name} `) }).click();
}

async function prepare(page: Page) {
  const mock = await mockLargeServer(page);
  await openEdit(page);
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(areaId);
  const layers = page.locator('details.layers');
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await page.getByLabel('not grains', { exact: true }).uncheck();
  await page.getByLabel('outlines', { exact: true }).check();
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = false; });
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  await tab(page, 'Grains');
  await page.getByTestId('tool-select').click();
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  return mock;
}

async function screenPoints(page: Page, points: number[][]) {
  return page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, points) => {
    const matrix = g.getScreenCTM();
    if (!matrix) throw new Error('Canvas micrometre transform is missing');
    return points.map(([x, y]) => {
      const p = new DOMPoint(x, y).matrixTransform(matrix);
      return { x: p.x, y: p.y };
    });
  }, points);
}

/** Gaps around a block in the fixture's 14-column grid; avoids spot/label hits at the start. */
async function block(page: Page, mock: Mock, columns: [number, number], rows: [number, number]): Promise<Box> {
  const grains = mock.ui.grains[areaId];
  const centres = grains.filter((_, i) => i % 14 >= columns[0] && i % 14 <= columns[1]
    && Math.floor(i / 14) >= rows[0] && Math.floor(i / 14) <= rows[1]).map(g => g.centroid_um);
  const dx = (grains[1].centroid_um[0] - grains[0].centroid_um[0]) * .3;
  const dy = (grains[14].centroid_um[1] - grains[0].centroid_um[1]) * .3;
  const [start, end] = await screenPoints(page, [
    [Math.min(...centres.map(p => p[0])) - dx, Math.min(...centres.map(p => p[1])) - dy],
    [Math.max(...centres.map(p => p[0])) + dx, Math.max(...centres.map(p => p[1])) + dy],
  ]);
  // Fail clearly if UI chrome would intercept either end of the intended gesture.
  for (const p of [start, end]) {
    expect(await page.evaluate(p => !!document.elementFromPoint(p.x, p.y)?.closest('[data-view]'), p)).toBe(true);
  }
  return { start, end };
}

async function dragBox(page: Page, box: Box, modifiers = ['Shift']) {
  for (const key of modifiers) await page.keyboard.down(key);
  await page.mouse.move(box.start.x, box.start.y);
  await page.mouse.down();
  await page.mouse.move(box.end.x, box.end.y, { steps: 10 });
  await page.mouse.up();
  for (const key of [...modifiers].reverse()) await page.keyboard.up(key);
}

function inside(p: Point, box: Box) {
  return p.x >= Math.min(box.start.x, box.end.x) && p.x <= Math.max(box.start.x, box.end.x)
    && p.y >= Math.min(box.start.y, box.end.y) && p.y <= Math.max(box.start.y, box.end.y);
}

async function grainHits(page: Page, mock: Mock, box: Box, artifacts = false) {
  const grains = mock.ui.grains[areaId].filter(g => artifacts || !g.artifact);
  const points = await screenPoints(page, grains.map(g => g.centroid_um));
  return grains.filter((_, i) => inside(points[i], box)).map(g => g.id);
}

async function expectGrains(page: Page, ids: string[]) {
  await expect.poll(() => page.locator('tr[data-grain-id].selected').evaluateAll(rows =>
    rows.map(row => row.getAttribute('data-grain-id')!).sort())).toEqual([...ids].sort());
  await expect(page.locator('polygon.grain.sel')).toHaveCount(ids.length);
  await expect(page.locator('[data-testid=spot].sel')).toHaveCount(0);
}

test('Select plain boxes replace and clear, Shift adds exact hits, Cmd/Ctrl toggles, Escape clears, Delete and undo use grain selection', async ({ page }) => {
  const mock = await prepare(page);
  const first = await block(page, mock, [1, 5], [0, 2]), second = await block(page, mock, [4, 8], [1, 3]);
  const a = await grainHits(page, mock, first), b = await grainHits(page, mock, second);
  expect(a.length).toBeGreaterThan(1);
  expect(b.some(id => a.includes(id))).toBe(true);
  expect(b.some(id => !a.includes(id))).toBe(true);
  await dragBox(page, first, []);
  await expectGrains(page, a);
  await expect(page.getByRole('button', { name: 'Merge', exact: true })).toBeEnabled();
  await expect(page.getByTestId('mark-artifact')).toBeEnabled();
  await dragBox(page, second, []);
  await expectGrains(page, b);
  const empty = { start: first.start, end: { x: first.start.x - 10, y: first.start.y - 10 } };
  expect(await grainHits(page, mock, empty)).toEqual([]);
  await dragBox(page, empty, []);
  await expectGrains(page, []);
  await dragBox(page, first, []);
  await expectGrains(page, a);
  const union = [...new Set([...a, ...b])];
  await dragBox(page, second);
  await expectGrains(page, union);
  await dragBox(page, first, ['Meta', 'Shift']);
  await expectGrains(page, union.filter(id => !a.includes(id)));
  await dragBox(page, first, ['Meta']);
  await expectGrains(page, union);
  await dragBox(page, first, ['Meta']);
  await expectGrains(page, union.filter(id => !a.includes(id)));
  // Ctrl is supported as well.
  await dragBox(page, first, ['Control', 'Shift']);
  await expectGrains(page, union);
  await dragBox(page, first, ['Control']);
  await expectGrains(page, union.filter(id => !a.includes(id)));
  await page.keyboard.press('Escape');
  await expectGrains(page, []);
  await dragBox(page, first);
  await page.keyboard.press('Delete');
  await expectGrains(page, []);
  for (const id of a) await expect(page.locator(`tr[data-grain-id="${id}"]`)).toHaveCount(0);
  await page.keyboard.press('Meta+z');
  for (const id of a) await expect(page.locator(`tr[data-grain-id="${id}"]`)).toHaveCount(1);
  expect(mock.unexpected).toEqual([]);
});

test('Spots tab plain boxes replace, Shift adds and Cmd toggles spot selection', async ({ page }) => {
  const mock = await prepare(page);
  await tab(page, 'Spots');
  const first = await block(page, mock, [1, 4], [0, 0]), second = await block(page, mock, [3, 6], [0, 0]);
  const spots = mock.ui.spots.filter(s => s.area_id === areaId);
  const points = await screenPoints(page, spots.map(s => [s.x_um, s.y_um]));
  const hits = (box: Box) => spots.filter((_, i) => inside(points[i], box)).map(s => s.id);
  const a = hits(first), b = hits(second);
  expect(a.length).toBeGreaterThan(1);
  const expectSpots = async (ids: string[]) => {
    await expect.poll(() => page.locator('tr[data-spot-id].selected').evaluateAll(rows =>
      rows.map(row => row.getAttribute('data-spot-id')!).sort())).toEqual([...ids].sort());
    await expect(page.locator('[data-testid=spot].sel')).toHaveCount(ids.length);
    await expect(page.locator('polygon.grain.sel')).toHaveCount(0);
  };
  await dragBox(page, first, []);
  await expectSpots(a);
  await dragBox(page, second, []);
  await expectSpots(b);
  await dragBox(page, first);
  const union = [...new Set([...a, ...b])];
  await expectSpots(union);
  await dragBox(page, first, ['Meta']);
  await expectSpots(union.filter(id => !a.includes(id)));
  await page.keyboard.press('Escape');
  await expectSpots([]);
  expect(mock.unexpected).toEqual([]);
});

for (const tool of ['delete', 'spot']) {
  test(`${tool} tool boxes select grains in the Grains tab`, async ({ page }) => {
    const mock = await prepare(page);
    await page.getByTestId(`tool-${tool}`).click();
    const box = await block(page, mock, [1, 5], [0, 2]);
    const expected = await grainHits(page, mock, box);
    await dragBox(page, box);
    await expectGrains(page, expected);
    await dragBox(page, box, ['Meta', 'Shift']);
    await expectGrains(page, []);
    await expect(page.getByTestId('spot')).toHaveCount(mock.ui.spots.length);
    expect(mock.unexpected).toEqual([]);
  });
}

test('selection gestures leave the saved spots byte-identical, including a drag starting on a spot', async ({ page }) => {
  const mock = await prepare(page);
  const before = JSON.stringify((await savedProject(page)).grainatlas_ui.spots);
  await tab(page, 'Grains');
  const box = await block(page, mock, [1, 5], [0, 2]);
  await dragBox(page, box);
  await dragBox(page, box, ['Meta', 'Shift']);
  await dragBox(page, box, ['Control']);
  await page.keyboard.press('Escape');
  // Starting on a registered spot must neither move it nor delete it.
  const spot = mock.ui.spots[2];
  const [start] = await screenPoints(page, [[spot.x_um, spot.y_um]]);
  const fromSpot = { start, end: box.end };
  await page.getByTestId('tool-delete').click();
  await dragBox(page, fromSpot);
  await expectGrains(page, await grainHits(page, mock, fromSpot));
  for (const modifiers of [['Meta'], ['Control'], ['Meta', 'Shift'], ['Control', 'Shift']]) {
    await page.keyboard.press('Escape');
    await dragBox(page, fromSpot, modifiers);
    await expectGrains(page, await grainHits(page, mock, fromSpot));
    await expect(page.getByTestId('spot')).toHaveCount(mock.ui.spots.length);
  }
  await page.keyboard.press('Escape');
  await tab(page, 'Spots');
  await page.getByTestId('tool-spot').click();
  // A Shift-click on an existing spot retains selection without placing a new spot.
  await page.keyboard.down('Shift');
  await page.mouse.click(start.x, start.y);
  await page.keyboard.up('Shift');
  await expect(page.locator('tr[data-spot-id].selected')).toHaveAttribute('data-spot-id', spot.id);
  await dragBox(page, fromSpot);
  await dragBox(page, fromSpot, ['Meta', 'Shift']);
  await dragBox(page, fromSpot, ['Meta']);
  await dragBox(page, fromSpot, ['Control']);
  await page.keyboard.press('Escape');
  const after = JSON.stringify((await savedProject(page)).grainatlas_ui.spots);
  expect(after).toBe(before);
  expect(mock.unexpected).toEqual([]);
});

test('artifacts are box-selected only while their layer is visible', async ({ page }) => {
  const mock = await prepare(page);
  const box = await block(page, mock, [3, 7], [0, 3]);
  const without = await grainHits(page, mock, box), withArtifacts = await grainHits(page, mock, box, true);
  expect(withArtifacts.length).toBeGreaterThan(without.length);
  await dragBox(page, box);
  await expectGrains(page, without);
  await page.keyboard.press('Escape');
  await page.locator('details.layers').evaluate((d: HTMLDetailsElement) => { d.open = true; });
  await page.getByLabel('not grains', { exact: true }).check();
  await page.locator('details.layers').evaluate((d: HTMLDetailsElement) => { d.open = false; });
  await dragBox(page, box);
  await expectGrains(page, withArtifacts);
  expect(mock.unexpected).toEqual([]);
});

for (const [tool, modifiers] of [['select', ['Space']], ['spot', []]] as const) {
  test(`${tool} tool ${modifiers.length ? 'Space' : 'plain'} drag on empty space pans without changing selection`, async ({ page }) => {
    const mock = await prepare(page);
    const selection = await block(page, mock, [1, 3], [0, 0]);
    const selected = await grainHits(page, mock, selection);
    await dragBox(page, selection);
    await expectGrains(page, selected);
    await page.getByTestId(`tool-${tool}`).click();
    const { start } = await block(page, mock, [1, 3], [1, 2]);
    const grain: Grain = mock.ui.grains[areaId][0];
    const [before] = await screenPoints(page, [grain.centroid_um]);
    await dragBox(page, { start, end: { x: start.x + 60, y: start.y + 35 } }, [...modifiers]);
    await expect.poll(async () => {
      const [after] = await screenPoints(page, [grain.centroid_um]);
      return { x: Math.round(after.x - before.x), y: Math.round(after.y - before.y) };
    }).toEqual({ x: 60, y: 35 });
    await expectGrains(page, selected);
    expect(mock.unexpected).toEqual([]);
  });
}

test('Select plain drag starting on a grain draws a replacement box', async ({ page }) => {
  const mock = await prepare(page);
  const box = await block(page, mock, [1, 5], [2, 4]);
  // Start on an unoccupied grain rather than the gap around the block.
  const [centre] = await screenPoints(page, [mock.ui.grains[areaId][29].centroid_um]);
  const start = { x: centre.x - .5, y: centre.y - .5 };
  const fromGrain = { start, end: box.end };
  const expected = await grainHits(page, mock, fromGrain);
  expect(expected.length).toBeGreaterThan(1);
  await dragBox(page, fromGrain, []);
  await expectGrains(page, expected);
  expect(mock.unexpected).toEqual([]);
});

test('Delete plain click on a spot still deletes only that spot', async ({ page }) => {
  const mock = await prepare(page);
  const before = (await savedProject(page)).grainatlas_ui.spots;
  const spot = mock.ui.spots[2];
  const [start] = await screenPoints(page, [[spot.x_um, spot.y_um]]);
  await page.getByTestId('tool-delete').click();
  await page.mouse.click(start.x, start.y);
  await expect(page.getByTestId('spot')).toHaveCount(mock.ui.spots.length - 1);
  expect((await savedProject(page)).grainatlas_ui.spots).toEqual(before.filter((s: { id: string }) => s.id !== spot.id));
  expect(mock.unexpected).toEqual([]);
});

test('Select plain drag on a selected spot still moves the selected spots together', async ({ page }) => {
  const mock = await prepare(page);
  const before = (await savedProject(page)).grainatlas_ui.spots;
  await tab(page, 'Spots');
  const selected = [mock.ui.spots[2], mock.ui.spots[4]];
  const [start, other] = await screenPoints(page, selected.map(s => [s.x_um, s.y_um]));
  await page.mouse.click(start.x, start.y);
  await page.keyboard.down('Shift');
  await page.mouse.click(other.x, other.y);
  await page.keyboard.up('Shift');
  await expect(page.locator('tr[data-spot-id].selected')).toHaveCount(2);
  // Alt disables snapping; the plain drag still starts on a selected spot.
  await dragBox(page, { start, end: { x: start.x + 20, y: start.y + 20 } }, ['Alt']);
  const after = (await savedProject(page)).grainatlas_ui.spots;
  const delta = selected.map(s => {
    const moved = after.find((q: { id: string }) => q.id === s.id);
    expect(moved.diameter_um).toBe(s.diameter_um);
    return [moved.x_um - s.x_um, moved.y_um - s.y_um];
  });
  expect(delta[0][0]).toBeGreaterThan(0);
  expect(delta[0][1]).toBeGreaterThan(0);
  expect(delta[1][0]).toBeCloseTo(delta[0][0], 6);
  expect(delta[1][1]).toBeCloseTo(delta[0][1], 6);
  expect(after.filter((s: { id: string }) => !selected.some(q => q.id === s.id)))
    .toEqual(before.filter((s: { id: string }) => !selected.some(q => q.id === s.id)));
  await tab(page, 'Spots');
  await expect(page.locator('tr[data-spot-id].selected')).toHaveCount(2);
  expect(mock.unexpected).toEqual([]);
});

test('Shift-click without a drag still adds a grain to the click selection', async ({ page }) => {
  const mock = await prepare(page);
  // The first row carries spots; use unoccupied grains in the third row.
  const grains = mock.ui.grains[areaId].slice(29, 31);
  const points = await screenPoints(page, grains.map(g => g.centroid_um));
  await page.mouse.click(points[0].x, points[0].y);
  await page.keyboard.down('Shift');
  await page.mouse.click(points[1].x, points[1].y);
  await page.keyboard.up('Shift');
  await expectGrains(page, grains.map(g => g.id));
  expect(mock.unexpected).toEqual([]);
});
