// Invented print-edit bundle only. The supervisor runs these tests with a preview server.
import { test, expect } from './mock-session';
import type { Page, Locator } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { root, mockImageServer, projectURL } from './support/image-edit';

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'], { cwd: root, stdio: 'pipe' });
});
async function open(page: Page) {
  const unexpected = await mockImageServer(page, { syncOnSave: false });
  await page.goto(projectURL);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  const nav = page.getByRole('navigation', { name: 'Screens' });
  await nav.getByRole('button', { name: 'Area', exact: true }).click();
  await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
  await nav.getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  // Add grain stays available; Click detection uses progressively loaded pixels.
  await expect(page.getByRole('button', { name: 'Add grain', exact: true })).toBeEnabled();
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  return unexpected;
}
async function active(control: Locator) {
  const role = await control.getAttribute('role');
  await expect(control).toHaveAttribute(role === 'radio' || role === 'menuitemradio' ? 'aria-checked' : 'aria-pressed', 'true');
}
const tools = [
  ['tool-select', 'v'], ['tool-spot', 's'], ['tool-line', 'l'], ['tool-raster', 'r'], ['tool-measure', 'm'], ['tool-label', 't'],
  ['tool-delete', 'd'], ['Add grain', 'g'], ['tool-split', 'k'], ['tool-outline', 'o'], ['tool-brush', 'b'], ['tool-pluck', 'p'],
] as const;
const extra = new Set(['tool-outline', 'tool-brush', 'tool-pluck']);
async function reach(page: Page, id: string, image: boolean) {
  if (!image && extra.has(id)) await page.getByRole('button', { name: 'More tools', exact: true }).click();
  return id === 'Add grain' ? page.getByRole('button', { name: id, exact: true }) : page.getByTestId(id);
}
async function fits(page: Page) {
  const toolbar = page.getByTestId('canvas-toolbar');
  await expect(toolbar).toBeInViewport({ ratio: 1 });
  expect(await toolbar.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && document.documentElement.scrollHeight <= innerHeight)).toBe(true);
  // All toolbar activators share a single horizontal row, including the inline extra tools.
  const rows = await toolbar.locator('button').evaluateAll(buttons => buttons.map(b => b.getBoundingClientRect().top));
  expect(Math.max(...rows) - Math.min(...rows)).toBeLessThan(5);
  for (const control of await toolbar.locator('button').all()) await expect(control).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole('group', { name: 'Zoom', exact: true })).toBeInViewport({ ratio: 1 });
}
for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  for (const image of [false, true]) {
    test(`${image ? 'Edit image' : 'View'} controls and shortcuts fit ${viewport.width} x ${viewport.height}`, async ({ page }) => {
      await page.setViewportSize(viewport);
      const unexpected = await open(page);
      if (image) await page.getByTestId('edit-image').click();
      await expect(page.getByTestId(image ? 'edit-image' : 'canvas-view')).toHaveAttribute('aria-checked', 'true');
      for (const [id, shortcut] of tools) {
        // Check activation by control. A menu selection closes and restores focus.
        let control = await reach(page, id, image);
        await expect(control).toBeEnabled(); await control.click();
        if (!image && extra.has(id)) {
          await expect(page.getByRole('button', { name: 'More tools', exact: true })).toBeFocused();
          control = await reach(page, id, image);
        }
        await active(control);
        if (!image && extra.has(id)) await page.keyboard.press('Escape');
        // Change tools first, then invoke the shortcut with the More menu closed.
        await page.getByTestId(id === 'tool-select' ? 'tool-measure' : 'tool-select').click();
        await page.keyboard.press(shortcut);
        control = await reach(page, id, image); await active(control);
        if (!image && extra.has(id)) await page.keyboard.press('Escape');
      }
      await page.getByTestId('tool-select').click();
      const toolbar = page.getByTestId('canvas-toolbar');
      // Both history actions remain reachable even when the history is empty.
      for (const name of ['Undo', 'Redo']) await expect(toolbar.getByRole('button', { name, exact: true })).toBeInViewport({ ratio: 1 });
      await fits(page);
      const zoom = page.getByRole('group', { name: 'Zoom', exact: true });
      const percentage = page.getByTestId('zoom-percentage');
      await zoom.getByRole('button', { name: 'Fit', exact: true }).click();
      const fitted = await percentage.innerText();
      await zoom.getByRole('button', { name: 'Zoom in', exact: true }).click();
      await expect(percentage).not.toHaveText(fitted);
      const enlarged = await percentage.innerText();
      await zoom.getByRole('button', { name: 'Zoom out', exact: true }).click();
      await expect(percentage).not.toHaveText(enlarged);
      await expect(percentage).toHaveText(fitted);
      await zoom.getByRole('button', { name: '1:1', exact: true }).click();
      await expect(percentage).toHaveText('100%');
      await zoom.getByRole('button', { name: 'Fit', exact: true }).click();
      await expect(percentage).toHaveText(fitted);
      for (const name of ['Rotate left 15°', 'Rotate right 15°']) await expect(zoom.getByRole('button', { name, exact: true })).toBeEnabled();
      expect(unexpected).toEqual([]);
    });
  }
}

test('radio groups, panel tabs and More menu support keyboard navigation', async ({ page }) => {
  await open(page);
  await page.getByTestId('tool-select').focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByTestId('tool-spot')).toBeFocused(); await active(page.getByTestId('tool-spot'));
  await page.keyboard.press('End'); await active(page.getByTestId('tool-label'));
  await page.keyboard.press('Home'); await active(page.getByTestId('tool-select'));
  const tabs = page.getByRole('tablist', { name: 'Canvas panel' });
  await tabs.getByRole('tab', { name: /^Spots / }).focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByTestId('tab-checks')).toBeFocused(); await expect(page.getByTestId('tab-checks')).toHaveAttribute('aria-selected', 'true');
  const more = page.getByRole('button', { name: 'More tools', exact: true });
  await more.focus(); await page.keyboard.press('ArrowDown');
  await expect(more).toHaveAttribute('aria-haspopup', 'menu'); await expect(more).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByTestId('tool-outline')).toBeFocused();
  await page.keyboard.press('ArrowDown'); await expect(page.getByTestId('tool-brush')).toBeFocused();
  await page.keyboard.press('End'); await expect(page.getByTestId('tool-pluck')).toBeFocused();
  await page.keyboard.press('Escape'); await expect(more).toBeFocused(); await expect(more).toHaveAttribute('aria-expanded', 'false');
  await page.getByTestId('canvas-view').focus(); await page.keyboard.press('ArrowLeft');
  await expect(page.getByTestId('edit-image')).toBeFocused(); await expect(page.getByTestId('edit-image')).toHaveAttribute('aria-checked', 'true');
  // A grain-tool shortcut also leaves an active image gesture tool.
  await page.getByRole('button', { name: 'Rectangle', exact: true }).click();
  await page.keyboard.press('s'); await active(page.getByTestId('tool-spot'));
  await expect(page.getByRole('button', { name: 'Rectangle', exact: true })).toHaveAttribute('aria-pressed', 'false');
});

test('read-only and missing-pixel tool eligibility survives the regrouping', async ({ page }) => {
  const unexpected = await mockImageServer(page, { syncOnSave: false });
  await page.goto(projectURL);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await expect(page.getByTestId('edit-image')).toBeDisabled();
  await expect(page.getByRole('radiogroup', { name: 'Tool', exact: true }).getByRole('radio')).toHaveCount(2);
  await expect(page.getByTestId('tool-spot')).toHaveCount(0);
  await expect(page.getByTestId('tool-pluck')).toHaveCount(0);
  await page.getByTestId('edit-copy').click();
  // Canvas can edit geometry over published tiles before Area loads full-resolution pixels.
  await expect(page.getByRole('button', { name: 'Add grain', exact: true })).toBeEnabled();
  await page.keyboard.press('g');
  await active(page.getByTestId('tool-add-grain'));
  await expect(page.getByTestId('add-mode-draw')).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByTestId('add-grain-pixels-hint')).toBeVisible();
  const more = page.getByRole('button', { name: 'More tools', exact: true });
  await more.click(); await expect(page.getByTestId('tool-brush')).toBeDisabled();
  await expect(page.getByTestId('tool-outline')).toBeEnabled(); await expect(page.getByTestId('tool-pluck')).toBeEnabled();
  await page.keyboard.press('Escape');
  for (const key of ['b']) {
    await page.getByTestId('tool-select').click(); await page.keyboard.press(key); await active(page.getByTestId('tool-select'));
  }
  await page.keyboard.press('o'); await more.click(); await active(page.getByTestId('tool-outline'));
  expect(unexpected).toEqual([]);
});
