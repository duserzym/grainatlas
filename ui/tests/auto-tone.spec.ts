import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));
const VIEW = '[data-testid=area-view]';

async function pixels(page: Page): Promise<number> {
  return page.locator(`${VIEW} canvas`).first().evaluate((c: HTMLCanvasElement) => {
    const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
    let h = 0;
    for (let i = 0; i < d.length; i += 4) h = (h * 31 + d[i]) % 2147483647;
    return h;
  });
}

async function settled(page: Page): Promise<number> {
  await expect(page.locator(VIEW)).toHaveAttribute('data-live', 'idle');
  let last = -1;
  await expect.poll(async () => {
    const p = await pixels(page);
    const same = p === last;
    last = p;
    return same;
  }, { intervals: [300] }).toBe(true);
  return last;
}

test('Auto, Before / After, Reset, and per-grain normalisation after finding grains', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await expect(page.getByTestId('before-after')).toBeDisabled();
  const p0 = await settled(page);

  await page.getByTestId('auto-tone').click();
  await expect(page.getByTestId('clahe')).toBeChecked();
  await expect(page.getByTestId('background')).toBeChecked();
  const pAuto = await settled(page);
  expect(pAuto).not.toBe(p0);

  // Before / After swaps the two parameter sets; manual controls stay editable.
  await page.getByTestId('before-after').click();
  await expect(page.getByTestId('clahe')).not.toBeChecked();
  expect(await settled(page)).toBe(p0);
  await page.getByTestId('before-after').click();
  await expect(page.getByTestId('clahe')).toBeChecked();
  expect(await settled(page)).toBe(pAuto);
  await page.getByTestId('low-pct').fill('2');
  expect(await settled(page)).not.toBe(pAuto);

  // Reset: the defaults again.
  await page.getByTestId('reset-tone').click();
  await expect(page.getByTestId('clahe')).not.toBeChecked();
  await expect(page.getByTestId('low-pct')).toHaveValue('0.1');
  expect(await settled(page)).toBe(p0);

  // Per-grain normalisation needs grains.
  await expect(page.getByTestId('grain-norm')).toBeDisabled();
  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.getByTestId('find-grains').click();
  await expect(page.locator('polygon.grain').first()).toBeVisible();
  await page.getByRole('button', { name: 'Area', exact: true }).click();
  const pG = await settled(page);
  await page.getByTestId('grain-norm').check();
  await expect.poll(() => pixels(page)).not.toBe(pG);
  const pN = await settled(page);
  await page.getByTestId('grain-norm-strength').fill('0.2');
  expect(await settled(page)).not.toBe(pN);
  // Stored with the project.
  const saved = await page.evaluate(() => {
    const k = Object.keys(localStorage).find((x) => x.startsWith('grainatlas.project.'))!;
    return JSON.parse(localStorage.getItem(k)!).areas[0].params.tone.grain_norm;
  });
  expect(saved).toMatchObject({ enabled: true, strength: 0.2 });
});
