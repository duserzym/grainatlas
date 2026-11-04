import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

// Regression: after stitching, tone, contrast and background-leveling changes must re-render
// the mosaic (they used to wait for a manual re-stitch, or change only the tile view).
const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

/** A fingerprint of what the image view's canvas shows. */
async function pixels(page: Page, scope: string): Promise<number> {
  return page.locator(`${scope} canvas`).first().evaluate((c: HTMLCanvasElement) => {
    const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
    let h = 0;
    for (let i = 0; i < d.length; i += 4) h = (h * 31 + d[i]) % 2147483647;
    return h;
  });
}

async function settled(page: Page, scope: string): Promise<number> {
  await expect(page.locator('.rendering')).toHaveCount(0);
  let last = -1;
  await expect.poll(async () => {
    const p = await pixels(page, scope);
    const same = p === last;
    last = p;
    return same;
  }, { intervals: [300] }).toBe(true);
  return last;
}

test('after stitching, correction and tone controls re-render the mosaic without a re-stitch', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  const view = '[data-testid=area-view]';
  const p0 = await settled(page, view);

  // Background leveling off: the view changes, and no stitch step runs.
  await page.getByTestId('background').uncheck();
  await expect.poll(() => pixels(page, view)).not.toBe(p0);
  const p1 = await settled(page, view);
  expect(p1).not.toBe(p0);
  await expect(page.locator('.stats .faint')).not.toContainText('stitch');
  await expect(page.locator('.stats .faint')).toContainText('photometric');

  // Contrast (stretch percentile) and CLAHE.
  await page.getByTestId('low-pct').fill('5');
  await expect.poll(() => pixels(page, view)).not.toBe(p1);
  const p2 = await settled(page, view);
  await page.getByTestId('clahe').check();
  await expect.poll(() => pixels(page, view)).not.toBe(p2);
  await settled(page, view);

  // The canvas shows the same, current pixels; a change made there-after shows up too.
  await page.getByTestId('to-canvas').click();
  const c0 = await settled(page, '.canvas');
  await page.getByRole('button', { name: 'Area', exact: true }).click();
  await page.getByTestId('clahe').uncheck();
  await settled(page, view);
  await page.getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect.poll(() => pixels(page, '.canvas')).not.toBe(c0);
});
