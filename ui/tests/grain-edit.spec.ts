import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

test('the Delete tool and the grain card remove a grain; undo restores it', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.getByTestId('find-grains').click();
  const real = page.locator('polygon.grain:not(.artifact)');
  await expect(real.first()).toBeVisible();
  const n = await real.count();

  // Delete tool: one click on a grain removes it.
  await page.getByTestId('tool-delete').click();
  const box = (await real.nth(2).boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(real).toHaveCount(n - 1);
  await page.keyboard.press('Meta+z');
  await expect(real).toHaveCount(n);

  // Grain card: select a grain, then Delete grain.
  await page.getByTestId('tool-select').click();
  await page.locator('.list tbody tr').first().click();
  await page.getByTestId('card-delete-grain').click();
  await expect(real).toHaveCount(n - 1);
});
