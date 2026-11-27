// Node timing/equivalence adapter for the same pure browser-worker jobs.
import { parentPort } from 'node:worker_threads';
import { runSyncWork } from '../../ui/src/lib/sync-compute-work.ts';
parentPort.on('message', ({id,job}) => {
  try { parentPort.postMessage({id,value:runSyncWork(job)}); }
  catch(error) { parentPort.postMessage({id,error:error.message}); }
});
