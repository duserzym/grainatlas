import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

test('CL-dark grains, then print sheets with the print_sheet registration', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();

  // Brighter or darker than the epoxy (CL): at least as many grains as bright only.
  await page.getByTestId('find-grains').click();
  await expect(page.locator('polygon.grain').first()).toBeVisible();
  const real = page.locator('polygon.grain:not(.artifact)');
  const bright = await real.count();
  await page.locator('summary', { hasText: 'Segmentation' }).click();
  await page.getByTestId('polarity').selectOption('both');
  await page.getByTestId('find-grains').click();
  await expect.poll(() => real.count()).toBeGreaterThanOrEqual(bright);

  // Artifacts are flagged with a reason, shown dimmed, and one click keeps one as a grain.
  const flagged = page.getByTestId('grain-artifact');
  await expect(flagged.first()).toBeVisible();
  const nArt = await flagged.count();
  const nReal = await real.count();
  await page.getByTestId('artifact-list').locator('summary').click();
  await page.getByTestId('keep-artifact').first().click();
  await expect(flagged).toHaveCount(nArt - 1);
  await expect(real).toHaveCount(nReal + 1);
  // And a grain can be marked as not a grain.
  await page.locator('.list tbody tr').first().click();
  await page.getByTestId('mark-artifact').click();
  await expect(flagged).toHaveCount(nArt);
  await expect(real).toHaveCount(nReal);

  // Sheets: tabloid landscape at 1 mm = 8 um, background stripped where safe.
  await page.getByRole('button', { name: 'Layout' }).click();
  await expect(page.getByTestId('style-sheets')).toBeChecked();
  await page.getByTestId('strip-bg').check();
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('page')).toBeVisible();
  await expect(page.getByTestId('sheet-stats')).toContainText('1 mm = 8 µm');

  const pdfDl = page.waitForEvent('download');
  await page.getByTestId('export-pdf').click();
  const pdf = readFileSync(await (await pdfDl).path());
  expect(pdf.subarray(0, 5).toString()).toBe('%PDF-');
  expect(pdf.toString('latin1')).toContain('/MediaBox [0 0 1224 792');

  const regDl = page.waitForEvent('download');
  await page.getByTestId('export-registration').click();
  const reg = JSON.parse(readFileSync(await (await regDl).path(), 'utf8'));
  expect(reg.frame).toBe('print_sheet');
  expect(reg.um_per_mm).toBe(8);
  expect(reg.bands.length).toBeGreaterThan(0);
  const b = reg.bands[0];
  expect((b.sheet_rect_mm[2] - b.sheet_rect_mm[0]) * 8).toBeCloseTo(b.mosaic_rect_um[2] - b.mosaic_rect_um[0], 1);
});
