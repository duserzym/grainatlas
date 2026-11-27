import { runSyncWork, type SyncWork } from './sync-compute-work.ts';
const worker = globalThis as unknown as {
  onmessage: (event: MessageEvent<{ id: number; job: SyncWork }>) => void;
  postMessage: (value: unknown) => void;
};
worker.onmessage = ({ data: { id, job } }) => {
  try { worker.postMessage({ id, value: runSyncWork(job) }); }
  catch (error) { worker.postMessage({ id, error: error instanceof Error ? error.message : String(error) }); }
};
