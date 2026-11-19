// Invented bundle only. The supervisor runs this against the browser build.
import type { Locator, Page } from '@playwright/test';
import { test, expect } from './mock-session';
import { mockServer, server, projectId, mountId } from './print-fixture';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'], { cwd: root, stdio: 'pipe' });
});

async function openCanvas(page: Page) {
  const unexpected = await mockServer(page);
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await page.getByRole('button', { name: 'Fit', exact: true }).click();
  return unexpected;
}

function canvasScale(page: Page) {
  return page.getByTestId('canvas-um-space').evaluate((g: SVGGElement) => {
    const m = g.getScreenCTM();
    if (!m) throw new Error('Canvas transform is missing');
    return Math.hypot(m.a, m.b);
  });
}

function previewScale(preview: Locator) {
  return preview.locator('.content').evaluate(el => {
    const m = new DOMMatrix(getComputedStyle(el).transform);
    return Math.hypot(m.a, m.b);
  });
}

async function stableScale(read: () => Promise<number>) {
  let previous = NaN;
  let stableSince = 0;
  let stableReads = 0;
  await expect.poll(async () => {
    const scale = await read();
    const now = performance.now();
    if (scale > 0 && Number.isFinite(scale) && Math.abs(scale - previous) < 1e-9) stableReads++;
    else {
      previous = scale;
      stableSince = now;
      stableReads = 1;
    }
    return stableReads >= 5 && now - stableSince >= 1000;
  }, { message: 'View scale stays stable for at least one second before the next zoom key', intervals: [250] }).toBe(true);
  return previous;
}

async function waitForPrintIdle(page: Page) {
  const quality = page.getByTestId('print-quality');
  await expect(quality).not.toContainText('Rendering pieces');
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(quality).toHaveAttribute('data-ready', 'true');
}

async function pressZoom(page: Page, read: () => Promise<number>, key: string, expected: number) {
  await stableScale(read);
  await page.keyboard.press(key);
  await expect.poll(read).toBeCloseTo(expected, 4);
}

for (const windows of [false, true]) {
  test.describe(windows ? 'Windows Edge / WebView2 zoom keys' : 'macOS zoom keys', () => {
    if (windows) test.use({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0' });
    test.beforeEach(async ({ page }) => {
      await page.addInitScript(windows => {
        Object.defineProperty(navigator, 'platform', { configurable: true, get: () => windows ? 'Win32' : 'MacIntel' });
        if ('userAgentData' in navigator) {
          Object.defineProperty(navigator.userAgentData, 'platform', { configurable: true, get: () => windows ? 'Windows' : 'macOS' });
        }
      }, windows);
    });
    const mod = windows ? 'Control' : 'Meta';

    for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
      test(`Canvas zoom, fit, numpad and text-field focus (${viewport.width} x ${viewport.height})`, async ({ page }) => {
        await page.setViewportSize(viewport);
        const unexpected = await openCanvas(page);
        const scale = () => canvasScale(page);
        const fitted = await stableScale(scale);
        expect(fitted).toBeGreaterThan(0);
        await pressZoom(page, scale, `${mod}+=`, fitted * 1.5);
        await pressZoom(page, scale, `${mod}+-`, fitted);
        await pressZoom(page, scale, `${mod}+Shift+Equal`, fitted * 1.5);
        await pressZoom(page, scale, `${mod}+0`, fitted);
        await pressZoom(page, scale, `${mod}+NumpadAdd`, fitted * 1.5);
        await pressZoom(page, scale, `${mod}+0`, fitted);

        // A real text input must never send modified zoom keys to the Canvas.
        const input = page.getByTestId('size-toolbar');
        await input.fill('35');
        await pressZoom(page, scale, `${mod}+=`, fitted);
        await expect(input).toHaveValue('35');
        await pressZoom(page, scale, `${mod}+-`, fitted);
        await pressZoom(page, scale, `${mod}+0`, fitted);

        await page.getByTestId('tool-select').click();
        // Explicit interface zoom belongs to the browser in this build.
        await pressZoom(page, scale, `${mod}+Alt+=`, fitted);
        // Existing plain keys continue to work when the pointer is in the view.
        await page.locator('[data-view]').hover();
        await pressZoom(page, scale, '=', fitted * 1.5);
        await pressZoom(page, scale, '0', fitted);
        expect(unexpected).toEqual([]);
      });
    }

    test('Layout print preview, Strip and Arrange zoom the active sheet', async ({ page }) => {
      const unexpected = await openCanvas(page);
      await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
      await page.getByTestId('sheet-scale').fill('4');
      await page.getByTestId('sheet-packing').selectOption('lines');
      await page.getByLabel('denoise (NLM)').uncheck();
      await page.getByLabel('sharpen', { exact: true }).uncheck();
      await page.getByTestId('sheets').click();
      await expect(page.getByTestId('sheet-stats')).toContainText('in lines');
      // Full-quality URLs replace the draft preview and trigger a fit; wait before testing zoom.
      await waitForPrintIdle(page);
      await expect(page.getByTestId('page')).toBeVisible();

      for (const label of ['Page preview', 'Strip editor', 'Arrange pieces']) {
        if (label === 'Strip editor') await page.getByTestId('edit-strips').check();
        if (label === 'Arrange pieces') await page.getByTestId('arrange-mode').click();
        const preview = page.getByRole('application', { name: label, exact: true });
        await expect(preview).toBeVisible();
        // Mode changes render pieces asynchronously and can change the paper size again.
        await waitForPrintIdle(page);
        // Remove checkbox/select focus before requesting view zoom.
        await page.getByRole('button', { name: 'Fit', exact: true }).click();
        const scale = () => previewScale(preview);
        const fitted = await stableScale(scale);
        await pressZoom(page, scale, `${mod}+=`, fitted * 1.5);
        await pressZoom(page, scale, `${mod}+-`, fitted);
        await pressZoom(page, scale, `${mod}+NumpadAdd`, fitted * 1.5);
        await pressZoom(page, scale, `${mod}+0`, fitted);
      }
      expect(unexpected).toEqual([]);
    });
  });
}
