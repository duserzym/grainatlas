// Screenshots for README.md and the site: one pass through the workflow on the bundled
// fixture (tests/fixtures/tm4000/Sweden_XS_150X, CL zircon, 150X). Run with `make screenshots`,
// which writes PNGs to target/screenshots and WebP copies to docs/images.
import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));
const OUT = fileURLToPath(new URL('../../target/screenshots/', import.meta.url));
const shot = async (page: Page, name: string) => {
  await page.waitForTimeout(600); // let the mosaic tiles settle
  await page.screenshot({ path: `${OUT}${name}.png` });
};

test('workflow screenshots', async ({ page }) => {
  await page.goto('./');
  await shot(page, '01-open');

  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('auto-tone').click();
  await page.waitForTimeout(2500);
  await shot(page, '02-stitch-tone');

  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.locator('summary', { hasText: 'Segmentation' }).click().catch(() => {});
  await page.getByTestId('polarity').selectOption('both').catch(() => {});
  await page.getByTestId('find-grains').click();
  const real = page.locator('polygon.grain:not(.artifact)');
  await expect(real.first()).toBeVisible();
  await shot(page, '03-grains');

  // Hand editing: zoom in, then an outline drawn click by click around the grain nearest the middle.
  const zoomIn = page.locator('button[title="Zoom in (+)"]');
  await zoomIn.click();
  await zoomIn.click();
  await page.waitForTimeout(500);
  const mid = { x: 560, y: 540 }; // middle of the canvas, left of the side panel
  let c = (await real.first().boundingBox())!;
  let best = Infinity;
  for (const g of await real.all()) {
    const b = await g.boundingBox();
    if (!b) continue;
    const d = Math.hypot(b.x + b.width / 2 - mid.x, b.y + b.height / 2 - mid.y);
    if (d < best) [best, c] = [d, b];
  }
  await page.getByTestId('tool-outline').click();
  const [cx, cy, rx, ry] = [c.x + c.width / 2, c.y + c.height / 2, c.width / 2 + 14, c.height / 2 + 14];
  for (let i = 0; i < 9; i++) {
    const t = (i / 12) * 2 * Math.PI;
    await page.mouse.click(cx + rx * Math.cos(t), cy + ry * Math.sin(t));
  }
  await page.mouse.move(cx + rx * Math.cos((9.5 / 12) * 2 * Math.PI), cy + ry * Math.sin((9.5 / 12) * 2 * Math.PI));
  await expect(page.getByTestId('lasso')).toBeVisible();
  await shot(page, '04-outline');
  await page.keyboard.press('Escape');
  await page.locator('button[title="Fit (0)"]').click();
  await page.waitForTimeout(400);

  // Spots: sample-n names, one per grain.
  await page.getByRole('button', { name: /^Spots/ }).click();
  await page.getByTestId('tool-spot').click();
  for (const i of [0, 2, 4, 6, 8, 10]) {
    const b = (await real.nth(i).boundingBox())!;
    await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
  }
  await expect(page.getByTestId('spot')).toHaveCount(6);
  await page.getByTestId('tool-select').click();
  await page.locator('button[title="Fit (0)"]').click();
  await page.waitForTimeout(400);
  const s0 = (await page.getByTestId('spot').nth(2).locator('circle').first().boundingBox())!;
  await page.mouse.click(s0.x + s0.width / 2, s0.y + s0.height / 2);
  await shot(page, '05-spots');

  // Print sheets: tabloid at 1 mm = 8 um with the registration sidecar.
  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('page')).toBeVisible();
  await shot(page, '06-sheets');

  // Dark theme, grains and spots on the canvas.
  await page.getByRole('button', { name: 'Canvas' }).click();
  await page.getByTestId('theme-dark').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.keyboard.press('Escape');
  await shot(page, '07-dark');
});
