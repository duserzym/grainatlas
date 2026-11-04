import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';

// The three ways in: a package (smoke.spec.ts), one area folder alone, loose images with a typed scale.
const AREA = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X/001_HJ-GC1', import.meta.url));
const TOP_ROW = [0, 1, 2].map((i) => `${AREA}/001_HJ-GC1_000${i}.jpg`);

test('one area folder on its own opens as a one-area package with the sidecar calibration', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(AREA);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await expect(page.getByTestId('open-kind')).toContainText('One area folder');
  await expect(page.locator('.area h2')).toHaveText(/^HJ-GC1\s+001_HJ-GC1$/);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  expect(Number(await page.getByTestId('rms').textContent())).toBeLessThan(0.5);
});

test('loose images: the scale is asked once, each image is an area, or they form one grid', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('images-input').setInputFiles(TOP_ROW);
  await expect(page.getByTestId('scale-dialog')).toBeVisible();
  await page.getByTestId('um-per-px').fill('0.6614583');
  await page.getByTestId('scale-open').click();
  for (const i of [0, 1, 2]) await expect(page.getByTestId(`area-001_HJ-GC1_000${i}`)).toBeVisible();
  await expect(page.getByTestId('open-kind')).toContainText('0.6614583 µm per pixel');
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('to-canvas')).toBeEnabled();
  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.getByTestId('find-grains').click();
  await expect(page.locator('polygon.grain').first()).toBeVisible();

  // Same files again: the saved project knows its scale and reopens without asking.
  await page.getByRole('button', { name: 'Open', exact: true }).click();
  await page.getByTestId('images-input').setInputFiles(TOP_ROW);
  await expect(page.getByRole('status')).toContainText("Reopened 3 images with the project's scale");
  await expect(page.getByTestId('area-001_HJ-GC1_0002')).toBeVisible();
  // Change the arrangement: the dialog starts from the recorded scale; make the images a 3 × 1 grid.
  await page.getByRole('button', { name: 'Change…' }).click();
  await expect(page.getByTestId('um-per-px')).toHaveValue('0.6614583');
  await page.getByTestId('as-grid').check();
  await page.getByTestId('grid-cols').fill('3');
  await page.getByTestId('grid-overlap').fill('20');
  await page.getByTestId('scale-open').click();
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  expect(Number(await page.getByTestId('rms').textContent())).toBeLessThan(1);
});
