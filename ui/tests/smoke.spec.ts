import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

test('open the fixture folder, stitch, view the canvas', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  const rms = Number(await page.getByTestId('rms').textContent());
  expect(rms).toBeLessThan(0.5);
  await page.getByTestId('to-canvas').click();
  await expect(page.locator('canvas')).toBeVisible();
});
