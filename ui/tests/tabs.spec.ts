import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

// Several projects in one window: each tab keeps its own package, mosaics and edits; opening
// something while a project is open makes a new tab; closing a tab with unsaved changes asks.
test('tabs: open two projects, switch, reorder, close with the unsaved-changes dialog', async ({ page }) => {
  await page.goto('./');
  const tabs = page.getByTestId('tab');
  await expect(tabs).toHaveCount(1);
  await expect(tabs.first()).toContainText('New tab');

  // Tab 1: open and stitch.
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await expect(tabs.first()).toContainText('Sweden_XS_150X');
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await expect(page.getByTestId('tab-dirty')).toHaveCount(1);

  // Opening again from the Open screen of a tab with a project: a second tab, not a replace.
  await page.getByRole('button', { name: 'Open', exact: true }).click();
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  await expect(tabs).toHaveCount(2);
  await expect(page.getByTestId('area-001_HJ-GC1')).toBeVisible();
  // The new tab has nothing stitched yet; the first one keeps its mosaic.
  await expect(page.getByTestId('rms')).toHaveCount(0);
  await tabs.first().click();
  await expect(page.getByRole('button', { name: 'Area', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Area', exact: true }).click();
  await expect(page.getByTestId('rms')).toBeVisible();

  // + makes an empty third tab on the Open screen.
  await page.getByTestId('tab-add').click();
  await expect(tabs).toHaveCount(3);
  await expect(tabs.nth(1)).toContainText('New tab');
  await expect(page.getByTestId('package-input')).toBeAttached();

  // Drag the new tab to the end.
  const a = (await tabs.nth(1).boundingBox())!;
  const b = (await tabs.nth(2).boundingBox())!;
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width - 4, b.y + b.height / 2, { steps: 6 });
  await page.mouse.up();
  await expect(tabs.nth(2)).toContainText('New tab');

  // Close the empty tab: no question.
  await tabs.nth(2).getByRole('button', { name: /^Close/ }).click();
  await expect(tabs).toHaveCount(2);

  // Close the stitched one: the dialog asks; Cancel keeps it, Close without saving drops it.
  await tabs.first().getByRole('button', { name: /^Close/ }).click();
  await expect(page.getByTestId('close-tab-dialog')).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(tabs).toHaveCount(2);
  await tabs.first().getByRole('button', { name: /^Close/ }).click();
  await page.getByTestId('close-tab-discard').click();
  await expect(tabs).toHaveCount(1);
  await expect(page.getByTestId('rms')).toHaveCount(0);
});
