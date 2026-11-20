import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const current = JSON.parse(readFileSync(new URL('../../tauri/src-tauri/tauri.conf.json', import.meta.url), 'utf8')).version;
const bundledNotes = readFileSync(new URL(`../../release-notes/${current}.md`, import.meta.url), 'utf8').trim();
const newNotes = '- Faster stitching.\n- Check grain outlines before printing.\n- Print sheets fit A4 and Letter.\n- Keep neighbouring grains together.\n- <b>Notes stay plain text.</b>';

const installReason = 'grainatlas cannot replace itself in this folder. Download the new version from GitHub.';
const releases = 'https://github.com/duserzym/grainatlas/releases/latest';

for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  for (const theme of ['light', 'dark']) {
    test.describe(`Portable update instructions, ${viewport.width} × ${viewport.height}, ${theme}`, () => {
      test.use({ viewport });
      test('popover and settings explain unsupported installation folders', async ({ page }) => {
        await page.addInitScript(theme => localStorage.setItem('grainatlas.theme', theme), theme);
        await page.goto('/?update-preview=unsupported');
        for (const control of [page.getByTestId('check-updates'), page.getByTestId('version-chip')]) {
          await expect(control).toBeVisible();
          const bounds = await control.boundingBox();
          expect(bounds).not.toBeNull();
          expect(bounds!.x).toBeGreaterThanOrEqual(0);
          expect(bounds!.y).toBeGreaterThanOrEqual(0);
          expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
          expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(100);
        }
        await expect(page.getByTestId('version-chip')).toHaveText(/^v\d+\.\d+\.\d+/);
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

      test('upgrade card fits, dismisses once, restores focus and reopens', async ({ page }) => {
        await page.addInitScript(theme => {
          localStorage.setItem('grainatlas.theme', theme);
          if (!localStorage.getItem('grainatlas.updates')) localStorage.setItem('grainatlas.updates', JSON.stringify({ lastSeenVersion: '0.0.0', channel: 'beta' }));
        }, theme);
        await page.goto('/?update-preview=ready');
        const card = page.getByTestId('whats-new');
        await expect(card.getByText(`What's new in v${current}`, { exact: true })).toBeVisible();
        await expect(card.locator('p')).toHaveText(bundledNotes);
        const box = await card.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.y).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
        expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
        await expect(card.getByRole('button', { name: "Dismiss what's new" })).toBeFocused();
        await page.keyboard.press('Escape');
        await expect(card).toHaveCount(0);
        await expect(page.getByTestId('check-updates')).toBeFocused();
        await page.reload();
        await expect(page.getByTestId('version-chip')).toBeVisible();
        await expect(card).toHaveCount(0);
        expect(await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.updates')!))).not.toHaveProperty('channel');
        await page.getByTestId('check-updates').click();
        await page.getByRole('link', { name: "What's new", exact: true }).click();
        await expect(card.locator('p')).toHaveText(bundledNotes);
        await card.getByRole('button', { name: "Dismiss what's new" }).click();
        await expect(card).toHaveCount(0);
        await expect(page.getByTestId('check-updates')).toBeFocused();
      });

      test('first install records the version without showing a card', async ({ page }) => {
        await page.goto('/?update-preview=idle');
        await expect(page.getByTestId('version-chip')).toBeVisible();
        await expect(page.getByTestId('whats-new')).toHaveCount(0);
        expect(await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.updates')!).lastSeenVersion)).toBe(current);
      });

      test('header, popover and settings use canonical state text', async ({ page }) => {
        await page.addInitScript(theme => localStorage.setItem('grainatlas.theme', theme), theme);
        for (const state of ['idle', 'checking', 'none', 'available', 'downloading', 'ready', 'installing', 'error']) {
          await page.goto(`/?update-preview=${state}`);
          const chip = page.getByTestId('version-chip');
          await expect(chip).toBeVisible();
          const version = (await chip.innerText()).trim();
          const text: Record<string, string> = {
            idle: `Current version: ${version}`,
            checking: 'Checking for updates…',
            none: `Up to date (${version})`,
            available: 'Update available: v0.2.0',
            downloading: 'Downloading v0.2.0… 43%',
            ready: 'v0.2.0 is ready',
            installing: 'Installing v0.2.0 and restarting…',
            error: 'Updates are not configured for this build yet.',
          };
          const check = page.getByTestId('check-updates');
          await expect(check).toHaveAttribute('title', `Check for updates · ${version} · ${text[state]}`);
          if (['checking', 'downloading', 'installing'].includes(state)) {
            await expect(check).toHaveAttribute('data-busy', 'true');
            await expect(check.locator('svg')).toHaveClass(/(?:^|\s)rotating(?:\s|$)/);
          }
          if (['available', 'ready'].includes(state)) await expect(check).toHaveAttribute('data-badge', 'true');
          await check.click();
          const popover = page.getByTestId('check-updates-popover');
          // Opening idle starts a manual check; the preview completes with no update.
          const expected = state === 'idle' ? text.none : text[state];
          await expect(popover.getByRole('status')).toHaveText(expected);
          if (['available', 'ready'].includes(state)) {
            const notes = popover.getByTestId('update-notes');
            await expect(notes).toHaveText(newNotes);
            await expect(notes.locator('b')).toHaveCount(0);
            await expect(notes).toHaveCSS('-webkit-line-clamp', '4');
            expect(await notes.evaluate(el => el.clientHeight < el.scrollHeight)).toBe(true);
            await expect(popover.getByRole('link', { name: 'Release notes', exact: true })).toHaveAttribute('href', 'https://github.com/duserzym/grainatlas/releases/tag/v0.2.0');
          }
          if (state === 'available') await expect(popover.getByText(`Current version: ${version}`, { exact: true })).toBeVisible();
          if (state === 'ready') await expect(popover.getByRole('button', { name: 'Restart to update' })).toBeVisible();
          if (state === 'error') {
            const details = popover.getByText('Details: Updates are not configured for this build yet', { exact: true });
            await expect(details).toBeVisible();
            await expect(details).toHaveCSS('user-select', 'text');
          }
          await popover.getByRole('link', { name: 'Update settings…' }).click();
          const panel = page.getByTestId('updates-panel');
          await expect(panel.getByRole('status')).toHaveText(expected);
          await expect(panel.getByRole('combobox', { name: 'Channel' })).toHaveCount(0);
          if (['available', 'ready'].includes(state)) {
            await expect(panel.getByTestId('update-notes')).toHaveText(newNotes);
            await expect(panel.getByTestId('update-notes')).toHaveCSS('-webkit-line-clamp', '4');
            await expect(panel.getByRole('link', { name: 'Release notes', exact: true })).toHaveAttribute('href', 'https://github.com/duserzym/grainatlas/releases/tag/v0.2.0');
          } else {
            await expect(panel.getByTestId('update-notes')).toHaveCount(0);
          }
          if (state === 'available') await expect(panel.getByText(`Current version: ${version}`, { exact: true })).toBeVisible();
          if (state === 'error') {
            const details = panel.getByText('Details: Updates are not configured for this build yet', { exact: true });
            await expect(details).toBeVisible();
            await expect(details).toHaveCSS('user-select', 'text');
          }
        }
      });
    });
  }
}
