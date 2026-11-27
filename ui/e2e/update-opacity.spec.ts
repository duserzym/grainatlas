import { expect, test } from '@playwright/test';
import { opaqueAndUnclipped } from './support/opaque-surface';

for (const theme of ['light', 'dark']) {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
    test.describe(`Opaque updater: ${theme}, ${viewport.width}`, () => {
      test.use({ viewport });
      for (const state of ['none', 'available', 'ready']) {
        test(`${state} menu and running-version notes cover content`, async ({ page }) => {
          await page.addInitScript(({ theme }) => {
            localStorage.setItem('grainatlas.theme', theme);
            localStorage.setItem('grainatlas.updates', JSON.stringify({ autoDownload: false }));
          }, { theme });
          await page.goto(`/?update-preview=${state}`);
          await page.getByTestId('check-updates').click();
          const panel = page.getByTestId('check-updates-popover');
          await expect(panel).toContainText(state === 'none' ? 'Up to date' : state === 'available' ? 'Update available' : 'is ready');
          const background = theme === 'light' ? 'rgb(255, 255, 255)' : 'rgb(22, 29, 43)';
          await opaqueAndUnclipped(panel, background);
          await panel.getByRole('link', { name: "What's new", exact: true }).click();
          await opaqueAndUnclipped(page.getByTestId('whats-new'), background);
        });
      }
    });
  }
}

for (const theme of ['light', 'dark']) {
  test(`Import menu covers the start page in ${theme}`, async ({ page }) => {
    await page.addInitScript(theme => localStorage.setItem('grainatlas.theme', theme), theme);
    await page.goto('/?update-preview=none');
    await page.getByRole('button', { name: 'Import', exact: true }).click();
    await opaqueAndUnclipped(page.getByRole('menu', { name: 'Import', exact: true }),
      theme === 'light' ? 'rgb(255, 255, 255)' : 'rgb(22, 29, 43)');
  });
}
