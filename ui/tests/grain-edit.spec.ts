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

test('outline a grain by hand (freehand and click by click), paint with the brush, undo', async ({ page }) => {
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

  // Freehand: a loop a little outside a grain redraws it (same count, drawn by hand).
  await page.getByTestId('tool-outline').click();
  const b = (await real.nth(3).boundingBox())!;
  const [cx, cy, rx, ry] = [b.x + b.width / 2, b.y + b.height / 2, b.width / 2 + 6, b.height / 2 + 6];
  await page.mouse.move(cx + rx, cy);
  await page.mouse.down();
  for (let i = 1; i <= 40; i++) await page.mouse.move(cx + rx * Math.cos((i / 40) * 2 * Math.PI), cy + ry * Math.sin((i / 40) * 2 * Math.PI));
  await page.mouse.up();
  await expect(page.getByText('drawn by hand')).toBeVisible();
  await expect(real).toHaveCount(n);

  // Click by click around another grain, Enter closes.
  const c = (await real.nth(6).boundingBox())!;
  for (const [x, y] of [[c.x - 5, c.y - 5], [c.x + c.width + 5, c.y - 5], [c.x + c.width + 5, c.y + c.height + 5], [c.x - 5, c.y + c.height + 5]]) await page.mouse.click(x, y);
  await expect(page.getByTestId('lasso')).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('lasso')).toHaveCount(0);
  await expect(real).toHaveCount(n);

  // Brush: painting over a grain grows it; with ⌥ it erases; undo restores.
  await page.getByTestId('tool-brush').click();
  const d = (await real.nth(10).boundingBox())!;
  await page.mouse.move(d.x + d.width / 2, d.y + d.height / 2);
  await page.mouse.down();
  await page.mouse.move(d.x + d.width / 2 + 8, d.y + d.height / 2, { steps: 4 });
  await page.mouse.up();
  await expect(page.locator('.toast')).toContainText(/Grew the grain/);
  await page.keyboard.down('Alt');
  await page.mouse.move(d.x + d.width - 2, d.y + 2);
  await page.mouse.down();
  await page.mouse.move(d.x + d.width - 2, d.y + d.height / 2, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up('Alt');
  await expect(page.locator('.toast')).toContainText(/Cut the grain/);
  await expect(real).toHaveCount(n);
  // Two undos: the brushed grain is back to its detected outline.
  await page.keyboard.press('Meta+z');
  await page.keyboard.press('Meta+z');
  await expect(page.getByText('drawn by hand')).toHaveCount(0);
  await expect(real).toHaveCount(n);
});

test('spots are named sample-n; Next n starts a sample anywhere; a spot n can be overwritten', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  await page.getByRole('button', { name: /^Grains/ }).click();
  await page.getByTestId('find-grains').click();
  const real = page.locator('polygon.grain:not(.artifact)');
  await expect(real.first()).toBeVisible();
  // No grain numbers on the canvas.
  await expect(page.locator('text.gnum')).toHaveCount(0);
  await page.getByRole('button', { name: /^Spots/ }).click();
  const sample = await page.locator('input[aria-label="Sample"], .field:has-text("Sample") input').first().inputValue().catch(() => '');
  const place = async (i: number) => {
    const b = (await real.nth(i).boundingBox())!;
    await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
  };
  await page.getByTestId('tool-spot').click();
  await place(1);
  const names = page.locator('.list tbody tr td:first-child');
  await expect(names.first()).toHaveText(/^.+-1$/);
  // Others took 2..30 in between: start this sample again at 31; it counts on from there.
  await page.getByTestId('next-n').fill('31');
  await place(2);
  await place(3);
  await expect(names).toHaveText([/-1$/, /-31$/, /-32$/]);
  // Overwrite one spot's number: the name follows.
  await names.nth(1).click();
  await page.getByTestId('panel-n').fill('7');
  await page.getByTestId('panel-n').press('Enter');
  await expect(names.nth(1)).toHaveText(/-7$/);
  if (sample) await expect(names.first()).toHaveText(`${sample}-1`);
});
