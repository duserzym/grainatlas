// Generate first: cargo run -p grainatlas-core --example large_bundle
// Run from ui/: PERF=1 PERF_LABEL=before pnpm exec playwright test e2e/perf-placement.spec.ts
import { test, expect, type Page } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import type { Grain, Spot } from '../src/lib/core/types';
import { distribution, latencyMs, mockLargeServer, openEdit, outputPath, report, savedProject } from './support/perf';

interface Sample {
  kind: 'place' | 'undo';
  start: number | null;
  expected: number;
  fallbackMs: number | null;
  pendingPaint: boolean;
}
interface Entry { name: string; startTime: number; duration: number; interactionId?: number }
interface Recorder {
  current: Sample | null;
  events: Entry[];
  longtasks: Entry[];
  eventObserver: PerformanceObserver | null;
  taskObserver: PerformanceObserver | null;
}
declare global { interface Window { __placementPerf: Recorder } }

async function installRecorder(page: Page) {
  await page.evaluate(() => {
    const state: Recorder = window.__placementPerf = { current: null, events: [], longtasks: [], eventObserver: null, taskObserver: null };
    if (PerformanceObserver.supportedEntryTypes.includes('event')) {
      state.eventObserver = new PerformanceObserver(list => state.events.push(...list.getEntries()));
      state.eventObserver.observe({ type: 'event', buffered: true, durationThreshold: 16 } as PerformanceObserverInit);
    }
    if (PerformanceObserver.supportedEntryTypes.includes('longtask')) {
      state.taskObserver = new PerformanceObserver(list => state.longtasks.push(...list.getEntries()));
      state.taskObserver.observe({ type: 'longtask', buffered: true });
    }
    // The timestamp comes from input dispatch in the page, not the driver clock.
    document.addEventListener('pointerdown', () => {
      const sample = state.current;
      if (sample?.kind === 'place' && sample.start === null) sample.start = performance.now();
    }, true);
    document.addEventListener('keydown', event => {
      const sample = state.current;
      if (sample?.kind === 'undo' && sample.start === null && event.key.toLowerCase() === 'z') sample.start = performance.now();
    }, true);
    new MutationObserver(() => {
      const sample = state.current;
      if (!sample || sample.start === null || sample.pendingPaint || sample.fallbackMs !== null) return;
      if (document.querySelectorAll('[data-testid=spot]').length !== sample.expected) return;
      sample.pendingPaint = true;
      requestAnimationFrame(() => requestAnimationFrame(() => {
        sample.fallbackMs = performance.now() - sample.start!;
      }));
    }).observe(document.body, { childList: true, subtree: true });
  });
}

async function measure(page: Page, kind: Sample['kind'], expected: number, input: () => Promise<unknown>) {
  await page.evaluate(({ kind, expected }) => {
    window.__placementPerf.current = { kind, expected, start: null, fallbackMs: null, pendingPaint: false };
  }, { kind, expected });
  await input();
  await expect(page.getByTestId('spot')).toHaveCount(expected);
  await page.waitForFunction(() => window.__placementPerf.current?.fallbackMs !== null);
  return page.evaluate(async () => {
    const state = window.__placementPerf, sample = state.current!;
    if (sample.start === null) throw new Error('Input timestamp was not captured');
    // Each input gets its own, non-overlapping 1.5 s observation window.
    await new Promise(resolve => setTimeout(resolve, Math.max(0, sample.start! + 1500 - performance.now())));
    state.events.push(...(state.eventObserver?.takeRecords() ?? []));
    state.longtasks.push(...(state.taskObserver?.takeRecords() ?? []));
    const names = sample.kind === 'place' ? ['pointerdown', 'pointerup', 'click'] : ['keydown', 'keyup'];
    // Event startTime may be a fraction of a millisecond before our capture listener.
    const events = state.events.filter(e => e.startTime >= sample.start! - 1 && e.startTime < sample.start! + 1500 && names.includes(e.name));
    const tasks = state.longtasks.filter(e => e.startTime >= sample.start! - 1 && e.startTime < sample.start! + 1500);
    const result = { kind: sample.kind, expectedSpots: sample.expected, inputAtMs: sample.start,
      eventLatencyMs: events.length ? Math.max(...events.map(e => e.duration)) : null,
      fallbackLatencyMs: sample.fallbackMs,
      longtaskSumMs: tasks.reduce((sum, e) => sum + e.duration, 0),
      longtaskMaxMs: tasks.length ? Math.max(...tasks.map(e => e.duration)) : 0,
      longtaskCount: tasks.length,
      events: events.map(e => ({ name: e.name, duration: e.duration, interactionId: e.interactionId ?? 0 })) };
    state.current = null;
    return result;
  });
}

async function screenPoint(page: Page, grain: Grain) {
  return page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, centroid) => {
    const matrix = g.getScreenCTM();
    if (!matrix) throw new Error('Canvas micrometre transform is missing');
    const p = new DOMPoint(...centroid).matrixTransform(matrix);
    const viewer = g.closest('[data-view]')!.getBoundingClientRect();
    return { x: p.x, y: p.y, bounds: { left: viewer.left, right: viewer.right, top: viewer.top, bottom: viewer.bottom } };
  }, grain.centroid_um);
}

async function visibleCentroid(page: Page, grain: Grain) {
  let p = await screenPoint(page, grain);
  const { bounds: b } = p;
  // Keep clear of toolbar, map, status and floating controls. Middle drag is the
  // app's normal pan gesture and does not create a spot or an undo checkpoint.
  const left = b.left + 80, right = b.right - 80, top = b.top + 140, bottom = b.bottom - 120;
  const dx = Math.max(left, Math.min(right, p.x)) - p.x;
  const dy = Math.max(top, Math.min(bottom, p.y)) - p.y;
  if (dx || dy) {
    const cx = (b.left + b.right) / 2, cy = (top + bottom) / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down({ button: 'middle' });
    await page.mouse.move(cx + dx, cy + dy, { steps: 8 });
    await page.mouse.up({ button: 'middle' });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    p = await screenPoint(page, grain);
  }
  // Catch UI overlays before attributing a failed placement to application timing.
  expect(await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y)?.closest('[data-view]'), p)).toBe(true);
  return p;
}

test('measure 60 grain placements and 10 undos on a large published mount', async ({ page }) => {
  test.skip(!process.env.PERF, 'Set PERF=1 to collect performance measurements');
  test.setTimeout(15 * 60_000);
  const mock = await mockLargeServer(page);
  await openEdit(page);
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Canvas', exact: true }).click();
  const areaId = mock.project.areas[0].id as string;
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption(areaId);
  await expect(page.getByTestId('spot')).toHaveCount(10);
  // Hide the info panel and side table so every click reaches the mosaic.
  const layers = page.locator('details.layers');
  await layers.evaluate((d: HTMLDetailsElement) => { d.open = true; });
  await page.getByLabel('info panel', { exact: true }).uncheck();
  await page.getByTestId('side-collapse').click();
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  await page.getByTestId('tool-spot').click();
  await page.getByRole('button', { name: '+', exact: true }).click();
  const existing = new Set(mock.ui.spots.map(s => s.grain_id));
  const targets = mock.ui.grains[areaId].filter(g => !g.artifact && !existing.has(g.id)).slice(0, 60);
  expect(targets).toHaveLength(60);
  await installRecorder(page);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.setSamplingInterval', { interval: 1000 });
  await cdp.send('Profiler.start');
  const placed: Awaited<ReturnType<typeof measure>>[] = [];
  const start = await page.evaluate(() => performance.now());
  let totalMs = 0;
  try {
    for (const [i, grain] of targets.entries()) {
      const p = await visibleCentroid(page, grain);
      placed.push(await measure(page, 'place', 11 + i, () => page.mouse.click(p.x, p.y)));
    }
    totalMs = await page.evaluate(start => performance.now() - start, start);
  } finally {
    const { profile } = await cdp.send('Profiler.stop');
    await writeFile(await outputPath('placement', 'cpuprofile'), JSON.stringify(profile));
    await cdp.detach();
  }
  const saved = await savedProject(page);
  const originalIds = new Set(mock.ui.spots.map(s => s.id));
  const additions = (saved.grainatlas_ui.spots as Spot[]).filter(s => !originalIds.has(s.id));
  expect(additions).toHaveLength(60);
  expect(saved.grainatlas_ui.spots).toHaveLength(70);
  for (const [i, spot] of additions.entries()) {
    expect(spot.grain_id).toBe(targets[i].id);
    expect(spot.area_id).toBe(areaId);
  }
  await page.getByRole('application', { name: 'Mosaic canvas' }).focus();
  const undone: Awaited<ReturnType<typeof measure>>[] = [];
  for (let i = 0; i < 10; i++) {
    undone.push(await measure(page, 'undo', 69 - i, () => page.keyboard.press('ControlOrMeta+z')));
  }
  const summary = (samples: typeof placed) => ({
    eventLatencyMs: distribution(samples.flatMap(s => s.eventLatencyMs === null ? [] : [s.eventLatencyMs])),
    fallbackLatencyMs: distribution(samples.flatMap(s => s.fallbackLatencyMs === null ? [] : [s.fallbackLatencyMs])),
    longtaskSumMs: distribution(samples.map(s => s.longtaskSumMs)),
    longtaskMaxMs: distribution(samples.map(s => s.longtaskMaxMs)),
    longtaskCount: distribution(samples.map(s => s.longtaskCount)),
    samples,
  });
  await report('placement', { latencyMs, totalMs, observationWindowMs: 1500,
    totalIncludes: '60 inputs, paint waits, per-input 1.5 s windows and any pan gestures',
    eventNote: 'Only events meeting the 16 ms reporting threshold are present; missing events are null, not zero.',
    observerSupport: await page.evaluate(() => PerformanceObserver.supportedEntryTypes),
    placements: summary(placed), undos: summary(undone) });
  expect(mock.unexpected).toEqual([]);
});
