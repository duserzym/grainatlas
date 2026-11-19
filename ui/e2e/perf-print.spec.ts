// Generate: cargo run --release -p grainatlas-core --example large_bundle
// Run: PERF=1 PERF_LABEL=after pnpm exec playwright test e2e/perf-print.spec.ts
import { test, expect, type Page } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import { latencyMs, mockLargeServer, outputPath, projectURL } from './support/perf';

async function decodedPage(page: Page) {
  await expect(page.getByTestId('page')).toBeVisible();
  await page.getByTestId('page').evaluate(async (img: HTMLImageElement) => {
    await img.decode();
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
}

test('measure print generation and cached return at 4x CPU', async ({ page }) => {
  test.skip(!process.env.PERF, 'Set PERF=1 to collect performance measurements');
  test.setTimeout(20 * 60_000);
  const mock = await mockLargeServer(page);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
  await page.addInitScript(() => {
    const w = window as any;
    w.__printPerf = { firstImageMs: null, printStart: null, firstPageMs: null, armReturn: false, backStart: null, cachedReturnMs: null, allPreviewsMs: null, fullResolutionReadyMs: null, pdfBytesReadyMs: null, pdfStart: null, pdfProgressSeen: false, pdfProgressGaps: 0, finished: false };
    document.addEventListener('click', event => {
      const target = event.target as Element;
      if (target?.closest('[data-testid=export-pdf]')) w.__printPerf.pdfStart = performance.now();
      if (target?.closest('[data-testid=sheets]')) w.__printPerf.printStart = performance.now();
      if (w.__printPerf.armReturn && target?.closest('nav[aria-label=Screens] button')?.textContent?.trim() === 'Layout') {
        w.__printPerf.backStart = performance.now(); w.__printPerf.armReturn = false;
      }
    }, true);
    const createURL = URL.createObjectURL.bind(URL);
    URL.createObjectURL = blob => {
      if (blob instanceof Blob && blob.type === 'application/pdf' && w.__printPerf.pdfStart !== null) w.__printPerf.pdfBytesReadyMs = performance.now() - w.__printPerf.pdfStart;
      return createURL(blob);
    };
    const scratch = document.createElement('canvas'); scratch.width = scratch.height = 32;
    const ctx = scratch.getContext('2d', { willReadFrequently: true })!;
    function sample() {
      const state = w.__printPerf;
      if (state.firstImageMs === null) {
        for (const canvas of document.querySelectorAll<HTMLCanvasElement>('[aria-label="Mosaic canvas"] canvas')) {
          if (!canvas.width || !canvas.height || !canvas.getBoundingClientRect().width) continue;
          ctx.clearRect(0, 0, 32, 32); ctx.drawImage(canvas, 0, 0, 32, 32);
          const rgba = ctx.getImageData(0, 0, 32, 32).data;
          const colours = new Set<number>();
          for (let i = 0; i < rgba.length; i += 4) if (rgba[i + 3]) colours.add((rgba[i] << 16) | (rgba[i + 1] << 8) | rgba[i + 2]);
          if (colours.size > 4) { state.firstImageMs = performance.now(); break; }
        }
      }
      if (state.printStart !== null) {
        if (state.allPreviewsMs === null && document.querySelector('[data-testid=preview-note]')) state.allPreviewsMs = performance.now() - state.printStart;
        if (state.fullResolutionReadyMs === null && document.querySelector('[data-testid=print-quality][data-ready=true]')) state.fullResolutionReadyMs = performance.now() - state.printStart;
      }
      if (state.printStart !== null && state.firstPageMs === null) {
        const img = document.querySelector<HTMLImageElement>('[data-testid=page]');
        if (img?.complete && img.naturalWidth && img.getBoundingClientRect().width && document.querySelector('[data-testid=sheet-stats]')) {
          state.firstPageMs = performance.now() - state.printStart;
        }
      }
      if (state.backStart !== null && state.cachedReturnMs === null) {
        const img = document.querySelector<HTMLImageElement>('[data-testid=page]');
        if (img?.complete && img.naturalWidth && img.getBoundingClientRect().width && document.querySelector('[data-testid=sheet-stats]')) {
          state.cachedReturnMs = performance.now() - state.backStart;
        }
      }
      if (state.pdfStart !== null && state.pdfBytesReadyMs === null) {
        if (document.querySelector('[data-testid=busy]')) state.pdfProgressSeen = true;
        else if (state.pdfProgressSeen) state.pdfProgressGaps++;
      }
      if (!state.finished) requestAnimationFrame(sample);
    }
    requestAnimationFrame(sample);
  });
  try {
    await page.goto(projectURL);
    await page.waitForFunction(() => (window as any).__printPerf.firstImageMs !== null);
    await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
    await page.getByTestId('edit-copy').click();
    const nav = page.getByRole('navigation', { name: 'Screens' });
    await nav.getByRole('button', { name: 'Layout', exact: true }).click();
    await expect(page.getByTestId('sheets')).toBeEnabled({ timeout: 8 * 60_000 });
    // The default auto-scale leaves one oversized band outside this invented
    // mount's paper. Keep export validation intact and measure a valid layout.
    await page.getByTestId('sheet-scale').fill('5');
    await page.getByTestId('sheets').click();
    await expect(page.getByTestId('sheet-stats')).toBeVisible({ timeout: 8 * 60_000 });
    await decodedPage(page);
    await page.waitForFunction(() => (window as any).__printPerf.firstPageMs !== null);
    const pager = page.locator('.pager .num');
    const pages = Number((await pager.innerText()).split('/')[1].trim());
    // All preview SVGs arrive together; only the selected page is mounted.
    // Avoid including 672 Playwright pagination round trips in render timing.
    await page.waitForFunction(() => (window as any).__printPerf.fullResolutionReadyMs !== null, undefined, { timeout: 8 * 60_000 });
    const lastURL = await page.getByTestId('page').getAttribute('src');
    await nav.getByRole('button', { name: 'Canvas', exact: true }).click();
    await page.evaluate(() => { (window as any).__printPerf.armReturn = true; });
    await nav.getByRole('button', { name: 'Layout', exact: true }).click();
    await expect(page.getByTestId('sheet-stats')).toBeVisible();
    await decodedPage(page);
    expect(await page.getByTestId('page').getAttribute('src')).toBe(lastURL);
    await page.waitForFunction(() => (window as any).__printPerf.cachedReturnMs !== null);
    const report = async (phase: 'before-pdf' | 'complete') => {
      const numbers = await page.evaluate(finished => {
        const state = (window as any).__printPerf; state.finished = finished; return state;
      }, phase === 'complete');
      const result = { phase, cpuRate: 4, latencyMs, umPerMm: 5, openFirstImageMs: numbers.firstImageMs, firstPreviewPageMs: numbers.firstPageMs,
        allPreviewsMs: numbers.allPreviewsMs, fullResolutionReadyMs: numbers.fullResolutionReadyMs, pdfBytesReadyMs: numbers.pdfBytesReadyMs,
        pages, cachedReturnMs: numbers.cachedReturnMs, pdfProgressSeen: numbers.pdfProgressSeen, pdfProgressGaps: numbers.pdfProgressGaps,
        allPreviewsIncludes: 'all preview SVGs available; selected page decoded separately' };
      await writeFile(await outputPath(phase === 'before-pdf' ? 'print-before-pdf' : 'print'), JSON.stringify(result) + '\n');
      console.log(JSON.stringify(result)); return result;
    };
    // Preserve all earlier numbers even when export is blocked or fails.
    await report('before-pdf');
    await expect(page.getByTestId('export-blocked')).toHaveCount(0);
    await expect(page.getByTestId('export-pdf')).toBeEnabled({ timeout: 10_000 });
    const [pdf] = await Promise.all([
      page.waitForEvent('download', { timeout: 120_000 }),
      page.getByTestId('export-pdf').click({ timeout: 10_000 }),
    ]);
    expect(pdf.suggestedFilename()).toMatch(/_print_\d{4}-\d{2}-\d{2}\.pdf$/);
    await page.waitForFunction(() => (window as any).__printPerf.pdfBytesReadyMs !== null);
    const result = await report('complete');
    expect(result.pdfProgressSeen).toBe(true);
    expect(result.pdfProgressGaps).toBe(0);
    expect(mock.unexpected).toEqual([]);
  } finally {
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 });
    await cdp.detach();
  }
});
