import { runSyncWork, type SyncWork, type WorkResult } from './sync-compute-work.ts';
import { timed } from './sync-perf.ts';
/** Yield a task, including on older WebViews without scheduler.yield. */
export async function yieldToInput(): Promise<void> {
  if (typeof window === 'undefined') return; // Node has no UI event loop to service.
  const scheduler = (globalThis as { scheduler?: { yield?: () => Promise<void> } }).scheduler;
  if (scheduler?.yield) await scheduler.yield();
  else await new Promise<void>(resolve => setTimeout(resolve, 0));
}
let worker: Worker | null | undefined;
let nextId = 0;
const pending = new Map<number, { resolve: (value: any) => void; reject: (reason: Error) => void }>();
function computeWorker() {
  if (worker !== undefined) return worker;
  try {
    worker = typeof Worker === 'undefined' ? null : new Worker(new URL('./sync-compute.worker.ts', import.meta.url), { type: 'module' });
    if (worker) {
      worker.onmessage = ({ data }) => {
        const request = pending.get(data.id);
        if (!request) return;
        pending.delete(data.id);
        if ('error' in data) request.reject(new Error(data.error)); else request.resolve(data.value);
      };
      worker.onerror = () => {
        worker?.terminate(); worker = null;
        for (const request of pending.values()) request.reject(new Error('Sync worker stopped. Changes are kept; try again.'));
        pending.clear();
      };
    }
  } catch { worker = null; }
  return worker;
}
export async function syncCompute<J extends SyncWork>(job: J): Promise<WorkResult<J>> {
  const current = computeWorker();
  if (!current) { await yieldToInput(); return timed(`compute.${job.op}.fallback`, () => runSyncWork(job)) as WorkResult<J>; }
  return new Promise<WorkResult<J>>((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    try { timed(`compute.${job.op}.handoff`, () => current.postMessage({ id, job })); }
    catch (error) { pending.delete(id); reject(error); }
  });
}
