/** Opt in from devtools: globalThis.__GRAINATLAS_PERF__ = true.
 * Timings contain phase names/durations only, never project data. */
export function perfEnabled(): boolean {
  return (globalThis as { __GRAINATLAS_PERF__?: boolean }).__GRAINATLAS_PERF__ === true;
}
let sequence = 0;
export function timed<T>(phase: string, run: () => T): T {
  if (!perfEnabled()) return run();
  const start = `grainatlas:${phase}:${sequence++}`;
  performance.mark(start);
  try { return run(); }
  finally {
    performance.measure(`grainatlas:${phase}`, { start, detail: { thread: 'main' } });
    performance.clearMarks(start);
    // Keep a bounded trace in long editing sessions.
    const entries = performance.getEntriesByName(`grainatlas:${phase}`, 'measure');
    if (entries.length > 256) performance.clearMeasures(`grainatlas:${phase}`);
  }
}

/** Async spans are wall time (network/core/worker waits), never main-thread CPU. */
export async function timedAsync<T>(phase: string, run: () => Promise<T>): Promise<T> {
  if (!perfEnabled()) return run();
  const start = `grainatlas:${phase}:${sequence++}`;
  performance.mark(start);
  try { return await run(); }
  finally {
    performance.measure(`grainatlas:${phase}`, { start, detail: { thread: 'wall' } });
    performance.clearMarks(start);
    if (performance.getEntriesByName(`grainatlas:${phase}`, 'measure').length > 256) performance.clearMeasures(`grainatlas:${phase}`);
  }
}
