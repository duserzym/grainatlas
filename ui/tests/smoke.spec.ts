import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

test('open the fixture, stitch, find grains, place three spots, export CSV, print one page', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  expect(Number(await page.getByTestId('rms').textContent())).toBeLessThan(0.5);
  await page.getByTestId('to-canvas').click();
  await expect(page.locator('canvas')).toBeVisible();

  // Grains.
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.getByTestId('find-grains').click();
  await expect(page.locator('polygon.grain').first()).toBeVisible();
  expect(await page.locator('polygon.grain').count()).toBeGreaterThan(10);

  // Three spots with the mouse alone: pick the tool, click three grains.
  await page.getByRole('button', { name: /^Spots/ }).click();
  await page.getByTestId('tool-spot').click();
  for (let i = 0; i < 3; i++) {
    const box = (await page.locator('polygon.grain:not(.artifact)').nth(i * 3).boundingBox())!;
    await page.keyboard.down('Alt'); // place exactly where clicked (no snapping)
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.up('Alt');
  }
  await expect(page.getByTestId('spot')).toHaveCount(3);
  const names = await page.locator('.list tbody tr td:first-child').allTextContents();
  expect(names).toHaveLength(3);
  expect(names[0]).toMatch(/^HJ-GC1-\d+-1$/);
  expect(names[2]).toMatch(/-3$/);

  // Drag the first spot with the select tool.
  await page.getByTestId('tool-select').click();
  const c = (await page.getByTestId('spot').first().locator('circle').first().boundingBox())!;
  await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2);
  await page.mouse.down();
  await page.mouse.move(c.x + c.width / 2 + 25, c.y + c.height / 2 + 10, { steps: 4 });
  await page.mouse.up();
  const c2 = (await page.getByTestId('spot').first().locator('circle').first().boundingBox())!;
  expect(c2.x - c.x).toBeGreaterThan(15);

  // Export CSV.
  const dl = page.waitForEvent('download');
  await page.getByTestId('export-csv').click();
  const file = await (await dl).path();
  const csv = readFileSync(file, 'utf8').trim().split('\n');
  expect(csv[0]).toContain('id,name,sample,area,grain,grain_id,x_um,y_um');
  expect(csv).toHaveLength(4);
  expect(csv[1]).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7/); // UUIDv7

  // Print one page of grains on Letter and export the PDF.
  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByTestId('style-layout').check();
  await page.getByTestId('paper').selectOption('letter');
  await page.getByTestId('scale').fill('10');
  await page.getByTestId('layout').click();
  await expect(page.getByTestId('page')).toBeVisible();
  const pdfDl = page.waitForEvent('download');
  await page.getByTestId('export-pdf').click();
  const pdf = readFileSync(await (await pdfDl).path());
  expect(pdf.subarray(0, 5).toString()).toBe('%PDF-');
  expect(pdf.toString('latin1')).toContain('/MediaBox [0 0 612 792');
});
