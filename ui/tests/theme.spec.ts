import { expect, test } from '@playwright/test';

const root = '.ga-root';

test('light by default; the switch persists; system follows the computer', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('./');
  // Nothing stored: light even when the system is dark.
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'light');
  await expect(page.getByTestId('theme-light')).toHaveAttribute('aria-pressed', 'true');

  await page.getByTestId('theme-dark').click();
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'dark');
  await page.reload();
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'dark');

  await page.getByTestId('theme-system').click();
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'light');

  // Settings has the same switch, with labels.
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.locator('.settings').getByTestId('theme-dark').click();
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'dark');
});

test('an older stored settings object with theme "dark" no longer forces dark', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('grainatlas.settings', JSON.stringify({ theme: 'dark', user: 'x' })));
  await page.goto('./');
  await expect(page.locator(root)).toHaveAttribute('data-theme', 'light');
});
