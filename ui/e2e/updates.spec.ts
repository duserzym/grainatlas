import { test, expect } from '@playwright/test';

const installReason = 'Install grainatlas before updating. On Windows, run the setup file from the GitHub release page once. Later updates arrive in-app.';
const releases = 'https://github.com/duserzym/grainatlas/releases/latest';

for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  for (const theme of ['light', 'dark']) {
    test.describe(`Portable update instructions, ${viewport.width} × ${viewport.height}, ${theme}`, () => {
      test.use({ viewport });
      test('popover and settings explain the one-time Windows install', async ({ page }) => {
        await page.addInitScript(theme => localStorage.setItem('grainatlas.theme', theme), theme);
        await page.goto('/?update-preview=unsupported');
        await page.getByTestId('check-updates').click();
        const popover = page.getByTestId('check-updates-popover');
        await expect(popover.getByText(installReason, { exact: true })).toBeVisible();
        await expect(popover.getByRole('link', { name: 'Open download page' })).toHaveAttribute('href', releases);
        await expect(popover.getByRole('button', { name: /Download|Restart to update/ })).toHaveCount(0);
        const box = await popover.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.y).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
        expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
        await popover.getByRole('link', { name: 'Update settings…' }).click();
        const panel = page.getByTestId('updates-panel');
        await expect(panel.getByText(installReason, { exact: true })).toBeVisible();
        await expect(panel.getByRole('link', { name: 'Open download page' })).toHaveAttribute('href', releases);
        await expect(panel.getByRole('button', { name: /Download|Restart to update/ })).toHaveCount(0);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      });
    });
  }
}
