import { test, expect } from './mock-session';
import { mockServer, server, projectId, mountId } from './print-fixture';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

test.beforeAll(() => {
  execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'],
    { cwd: fileURLToPath(new URL('../..', import.meta.url)), stdio: 'pipe' });
});

interface PrintSnapshot { pages: number; images: number; decoded: number; title: string }
interface PrintState { calls: PrintSnapshot[]; opened: number; roots: Element[] }
declare global { interface Window { __printDialog: PrintState } }

test('Windows desktop sends Tabloid to the selected printer and does not retry on Letter', async ({ page }) => {
  const unexpected = await mockServer(page, { standalone: true });
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByLabel('Paper', { exact: true }).selectOption('tabloid');
  await page.getByTestId('sheet-scale').fill('4');
  await page.getByTestId('sheet-packing').selectOption('lines');
  await page.getByLabel('denoise (NLM)').uncheck();
  await page.getByLabel('sharpen', { exact: true }).uncheck();
  await page.getByTestId('sheets').click();
  await expect(page.getByTestId('sheet-stats')).toBeVisible();
  await expect(page.getByTestId('busy')).toHaveCount(0);

  // The invented bundle stays on the WASM core. Stub only the Windows
  // printing commands at the point where the prepared pages are printed.
  await page.evaluate(() => {
    const jobs: unknown[] = [];
    const w = window as Window & { __windowsJobs?: unknown[]; __TAURI_INTERNALS__?: unknown };
    w.__windowsJobs = jobs;
    Object.defineProperty(navigator, 'platform', { value: 'Win32' });
    window.print = () => { throw new Error('Windows must not fall back to browser printing'); };
    w.__TAURI_INTERNALS__ = { invoke: async (command: string, args: { job: { printer: string } }) => {
      if (command === 'windows_print_printers') return { names: ['Letter printer', 'Tabloid printer'], defaultName: 'Letter printer' };
      if (command === 'windows_print_document') {
        jobs.push(args.job);
        if (args.job.printer === 'Letter printer') throw new Error('This printer does not support the selected paper size.');
      }
    } };
  });
  await page.getByRole('button', { name: 'Print…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Print sheets' });
  await expect(dialog.getByText('17 × 11 in', { exact: false })).toBeVisible();
  await dialog.getByRole('button', { name: 'Print', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('does not support');
  await expect(page.locator('#ga-print-root')).toHaveCount(1);
  await dialog.getByLabel('Printer').selectOption('Tabloid printer');
  await dialog.getByLabel('Copies').fill('2');
  await dialog.getByLabel('Pages', { exact: true }).fill('2');
  await dialog.getByRole('button', { name: 'Print', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('#ga-print-root')).toHaveCount(0);
  expect(await page.evaluate(() => (window as Window & { __windowsJobs?: unknown[] }).__windowsJobs)).toEqual([
    { printer: 'Letter printer', copies: 1, pageRanges: '', paperMm: [431.8, 279.4] },
    { printer: 'Tabloid printer', copies: 2, pageRanges: '2', paperMm: [431.8, 279.4] },
  ]);
  expect(unexpected).toEqual([]);
});

for (const mode of ['browser', 'Windows WebView2', 'native promise'] as const) {
  test.describe(mode, () => {
    if (mode === 'Windows WebView2') {
      test.use({ userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0' });
    }
    test('prints decoded vector pages without pop-ups and keeps them until cleanup', async ({ page }) => {
      const unexpected = await mockServer(page);
      await page.addInitScript(({ windows, native }) => {
        if (windows) {
          Object.defineProperty(navigator, 'platform', { configurable: true, get: () => 'Win32' });
          if ('userAgentData' in navigator) {
            Object.defineProperty(navigator.userAgentData, 'platform', { configurable: true, get: () => 'Windows' });
          }
        }
        const state = window.__printDialog = { calls: [], opened: 0, roots: [] } as PrintState;
        const decoded = new Set<string>();
        const decode = HTMLImageElement.prototype.decode;
        HTMLImageElement.prototype.decode = async function () {
          await decode.call(this);
          decoded.add(this.src);
        };
        window.open = () => { state.opened++; return null; };
        window.print = () => {
          const root = document.getElementById('ga-print-root');
          const images = [...root?.querySelectorAll('svg image') ?? []];
          state.calls.push({ pages: root?.querySelectorAll(':scope > svg').length ?? 0,
            images: images.length, decoded: images.filter(image => decoded.has(
              image.getAttribute('href') ?? image.getAttributeNS('http://www.w3.org/1999/xlink', 'href') ?? '')).length,
            title: document.title });
          if (root) state.roots.push(root);
          // Resolving the native invocation must not remove the print document.
          if (native) return Promise.resolve();
        };
      }, { windows: mode === 'Windows WebView2', native: mode === 'native promise' });
      await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
      await page.getByTestId('edit-copy').click();
      const nav = page.getByRole('navigation', { name: 'Screens' });
      await nav.getByRole('button', { name: 'Layout', exact: true }).click();
      await page.getByLabel('Title', { exact: true }).fill('Mount A print sheets');
      await page.getByTestId('sheet-scale').fill('4');
      await page.getByTestId('sheet-packing').selectOption('lines');
      await page.getByLabel('denoise (NLM)').uncheck();
      await page.getByLabel('sharpen', { exact: true }).uncheck();
      await page.getByTestId('sheets').click();
      await expect(page.getByTestId('sheet-stats')).toBeVisible();
      await expect(page.getByTestId('busy')).toHaveCount(0);
      const pages = Number((await page.locator('.pager .num').innerText()).split('/')[1].trim());
      expect(pages).toBeGreaterThan(1);
      const originalTitle = await page.title();
      const print = page.getByRole('button', { name: 'Print…', exact: true });
      await print.click();
      await expect.poll(() => page.evaluate(() => window.__printDialog.calls.length)).toBe(1);
      await expect(page.getByRole('alert')).toHaveCount(0);
      const snapshot = await page.evaluate(() => window.__printDialog.calls[0]);
      expect(snapshot.pages).toBe(pages);
      expect(snapshot.images).toBeGreaterThan(0);
      expect(snapshot.decoded).toBe(snapshot.images);
      expect(snapshot.title).toBe('Mount A print sheets');
      expect(await page.evaluate(() => window.__printDialog.opened)).toBe(0);
      const root = page.locator('#ga-print-root');
      await expect(root.locator(':scope > svg')).toHaveCount(pages);
      await expect(root).toBeHidden();
      expect(await root.locator('[id]').evaluateAll(elements => {
        const ids = elements.map(element => element.id);
        return new Set(ids).size === ids.length;
      })).toBe(true);

      await page.emulateMedia({ media: 'print' });
      await expect(page.locator('#grainatlas')).toBeHidden();
      await expect(root).toBeVisible();
      for (const svg of await root.locator(':scope > svg').all()) await expect(svg).toBeVisible();
      expect(await page.evaluate(() => {
        const body = getComputedStyle(document.body), html = getComputedStyle(document.documentElement);
        return body.overflow === 'visible' && html.overflow === 'visible'
          && body.minHeight === '0px' && html.minHeight === '0px'
          && document.body.scrollHeight > innerHeight;
      })).toBe(true);
      await page.emulateMedia({ media: 'screen' });
      await expect(root).toBeHidden();
      await expect(page.locator('#grainatlas')).toBeVisible();

      // A second print replaces a leftover root even if afterprint never fired.
      await print.click();
      await expect.poll(() => page.evaluate(() => window.__printDialog.calls.length)).toBe(2);
      expect(await page.evaluate(() => window.__printDialog.roots[0].isConnected)).toBe(false);
      await expect(root).toHaveCount(1);
      await page.evaluate(() => window.dispatchEvent(new Event('afterprint')));
      if (mode === 'native promise') {
        // Native afterprint may arrive while WebKit is still paginating.
        await expect(root).toHaveCount(1);
        await expect(page).toHaveTitle('Mount A print sheets');
      } else {
        await expect(root).toHaveCount(0);
        await expect(page).toHaveTitle(originalTitle);
      }

      // Component teardown is the other safe cleanup boundary.
      await print.click();
      await expect.poll(() => page.evaluate(() => window.__printDialog.calls.length)).toBe(3);
      expect(await page.evaluate(() => window.__printDialog.roots[1].isConnected)).toBe(false);
      await expect(root).toHaveCount(1);
      await nav.getByRole('button', { name: 'Canvas', exact: true }).click();
      await expect(root).toHaveCount(0);
      await expect(page).toHaveTitle(originalTitle);
      expect(unexpected).toEqual([]);
    });
  });
}
