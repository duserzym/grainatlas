// Generate first: cargo run -p grainatlas-core --example large_bundle
// Run from ui/: PERF_LABEL=before PERF_LATENCY_MS=150 pnpm exec playwright test -c playwright.perf.config.ts e2e/perf-area.spec.ts
import { test, expect, type Page } from '@playwright/test';
import { latencyMs, mockLargeServer, openEdit, report, tileTotals } from './support/perf';

interface AreaRecorder {
  start: number | null;
  firstImageMs: number | null;
  editableMs: number | null;
  busySince: number | null;
  busyIntervals: { startMs: number; endMs: number }[];
  busyShown: boolean;
  stop: () => void;
}
declare global { interface Window { __areaPerf: AreaRecorder } }

async function startAreaRecorder(page: Page) {
  await page.evaluate(() => {
    const state: AreaRecorder = window.__areaPerf = { start: null, firstImageMs: null, editableMs: null,
      busySince: null, busyIntervals: [], busyShown: false, stop: () => {} };
    const scratch = document.createElement('canvas');
    scratch.width = scratch.height = 32;
    const context = scratch.getContext('2d', { willReadFrequently: true })!;
    let frame = 0;
    const begin = (event: PointerEvent) => {
      const target = event.target instanceof Element ? event.target.closest('nav[aria-label=Screens] button') : null;
      if (target?.textContent?.trim() === 'Area' && state.start === null) state.start = performance.now();
    };
    document.addEventListener('pointerdown', begin, true);
    const observeBusy = () => {
      if (state.start === null) return;
      const now = performance.now() - state.start;
      const busy = !!document.querySelector('[data-testid=busy]');
      if (busy && state.busySince === null) { state.busyShown = true; state.busySince = now; }
      if (!busy && state.busySince !== null) {
        state.busyIntervals.push({ startMs: state.busySince, endMs: now });
        state.busySince = null;
      }
    };
    const observer = new MutationObserver(observeBusy);
    observer.observe(document.body, { childList: true, subtree: true });
    const sample = () => {
      if (state.start !== null) {
        observeBusy();
        const elapsed = performance.now() - state.start;
        if (state.firstImageMs === null) {
          for (const canvas of document.querySelectorAll<HTMLCanvasElement>('[data-testid=area-view] canvas')) {
            if (!canvas.width || !canvas.height || !canvas.getBoundingClientRect().width) continue;
            // Downsample rather than reading the entire viewport on every frame.
            // The viewer paints a uniform background even before tiles arrive.
            context.clearRect(0, 0, 32, 32);
            context.drawImage(canvas, 0, 0, 32, 32);
            const rgba = context.getImageData(0, 0, 32, 32).data;
            const colours = new Set<number>();
            for (let i = 0; i < rgba.length; i += 4) {
              if (rgba[i + 3]) colours.add((rgba[i] << 16) | (rgba[i + 1] << 8) | rgba[i + 2]);
            }
            if (colours.size > 4) { state.firstImageMs = elapsed; break; }
          }
        }
        const low = document.querySelector<HTMLInputElement>('[data-testid=low-pct]');
        if (state.editableMs === null && low && !low.matches(':disabled') && !document.querySelector('[data-testid=busy]')) {
          state.editableMs = elapsed;
        }
      }
      frame = requestAnimationFrame(sample);
    };
    frame = requestAnimationFrame(sample);
    state.stop = () => {
      observeBusy();
      observer.disconnect();
      cancelAnimationFrame(frame);
      document.removeEventListener('pointerdown', begin, true);
    };
  });
}

async function measureArea(page: Page) {
  await startAreaRecorder(page);
  const nav = page.getByRole('navigation', { name: 'Screens' });
  await nav.getByRole('button', { name: 'Area', exact: true }).click();
  await page.waitForFunction(() => window.__areaPerf.firstImageMs !== null && window.__areaPerf.editableMs !== null,
    undefined, { timeout: 8 * 60_000 });
  await expect(page.locator('svg.hist path').first()).toHaveAttribute('d', /^M0 90 L/);
  await expect(page.getByTestId('low-pct')).toBeEnabled();
  await expect(page.getByTestId('pixels-loading')).toHaveCount(0);
  await expect(page.getByTestId('busy')).toHaveCount(0);
  // No application-state hook exists. Layout's working print controls and the
  // absence of the inline image status are the public completion signal. Keep this
  // separately timed so incremental Area rendering can improve independently.
  await nav.getByRole('button', { name: 'Layout', exact: true }).click();
  await expect(page.getByTestId('sheets')).toBeEnabled({ timeout: 8 * 60_000 });
  await expect(page.getByTestId('busy')).toHaveCount(0);
  return page.evaluate(() => {
    const state = window.__areaPerf;
    state.stop();
    if (state.start === null) throw new Error('Area input timestamp was not captured');
    const allPixelsMs = performance.now() - state.start;
    const busyDurationMs = state.busyIntervals.reduce((sum, interval) => sum + interval.endMs - interval.startMs, 0);
    return { firstImageMs: state.firstImageMs, editableMs: state.editableMs, allPixelsMs,
      allPixelsSignal: 'Layout print controls enabled and busy dialog absent (observable proxy; no app-state hook)',
      busyShown: state.busyShown, busyDurationMs, busyIntervals: state.busyIntervals };
  });
}

test('measure first and second Area opens of a large published mount', async ({ page }) => {
  test.setTimeout(20 * 60_000);
  const mock = await mockLargeServer(page);
  const firstStart = mock.requests.length;
  await openEdit(page);
  const firstAreaStart = mock.requests.length;
  const first = await measureArea(page);
  const firstEnd = mock.requests.length;
  // Navigate again in this page/context: document and wasm state are reset;
  // IndexedDB, localStorage, and browser cache settings are retained. openEdit
  // restores the same deep link because the app removes it after handling it.
  const secondStart = mock.requests.length;
  await page.reload();
  await openEdit(page);
  const secondAreaStart = mock.requests.length;
  const second = await measureArea(page);
  const secondEnd = mock.requests.length;
  await report('area', { latencyMs, interception: 'CDP Fetch; HTTP cache not disabled; no storage cleared',
    expectedLevelZeroTiles: mock.atlas.areas.reduce((sum: number, a: { w: number; h: number }) =>
      sum + Math.ceil(a.w / mock.atlas.tile_size) * Math.ceil(a.h / mock.atlas.tile_size), 0),
    firstOpen: { ...first, wholeOpen: tileTotals(mock.requests.slice(firstStart, firstEnd)),
      areaOnly: tileTotals(mock.requests.slice(firstAreaStart, firstEnd)) },
    secondOpen: { ...second, wholeOpen: tileTotals(mock.requests.slice(secondStart, secondEnd)),
      areaOnly: tileTotals(mock.requests.slice(secondAreaStart, secondEnd)) } });
  expect(mock.unexpected).toEqual([]);
});
