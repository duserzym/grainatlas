// Pure jobs shared by the browser worker and equivalence tests. Algorithms stay
// in sync.ts/sync-hash.ts; production never computes mock element hashes.
import { canonicalJson } from './sync-hash.ts';
import { syncWireBody } from './sync-wire.ts';
import { diffElements, firstSyncMerge, syncChanges, groupedGrains, uiSpots, type Elements, type ReadElements } from './sync.ts';
import type { Grain, Spot } from './core/types.ts';
import type { SyncBody } from './sync-api.ts';
export type SyncWork =
  | { op: 'parse'; text: string }
  | { op: 'copy'; value: unknown }
  | { op: 'display'; elements: ReadElements; grains: Record<string, Grain[]>; spots: Spot[]; samples: Record<string, string> }
  | { op: 'key'; body: SyncBody }
  | { op: 'wire'; body: SyncBody; requestId?: string }
  | { op: 'first-merge'; base: ReadElements; mine: Elements; current: ReadElements; uuid: string }
  | { op: 'prepare'; mode: SyncBody['mode']; base: ReadElements; mine: Elements; current: ReadElements; uuid: string; seen: ReadElements; since: ReadElements };
export function runSyncWork(job: SyncWork) {
  switch (job.op) {
    case 'parse': return JSON.parse(job.text);
    case 'copy': return JSON.parse(JSON.stringify(job.value));
    case 'display': {
      const elements = JSON.parse(JSON.stringify(job.elements)) as ReadElements;
      const ui = { spots: uiSpots(elements.spots, elements.grains, job.spots, job.samples), grains: groupedGrains(elements.grains, job.grains), labels: elements.labels.filter(l => !l.retired) };
      return ui;
    }
    case 'key': return canonicalJson(syncWireBody(job.body));
    case 'wire': return JSON.stringify({ ...syncWireBody(job.body), ...(job.requestId && !job.body.dry_run ? { request_id: job.requestId } : {}) });
    case 'first-merge': return firstSyncMerge(job.base, job.mine, job.current, job.uuid);
    case 'prepare': {
      const diff = syncChanges(job.mode, job.base, job.mine, job.current, job.uuid, job.seen);
      return { diff, sinceSeen: diffElements(job.since, job.current) };
    }
  }
}
export type WorkResult<J extends SyncWork> = J extends { op: 'parse' } ? any
  : J extends { op: 'copy'; value: infer V } ? V
  : J extends { op: 'display' } ? { spots: Spot[]; grains: Record<string, Grain[]>; labels: Elements['labels'] }
  : J extends { op: 'key' | 'wire' } ? string
  : J extends { op: 'first-merge' } ? ReturnType<typeof firstSyncMerge>
  : { diff: ReturnType<typeof syncChanges>; sinceSeen: ReturnType<typeof diffElements> };
