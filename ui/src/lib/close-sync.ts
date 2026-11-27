import type { AppState } from './store.svelte';
import { appStorage } from './app-storage';
import { flushWrites } from './deferred-writes';

export function syncReminder(tab: AppState) {
  tab.tims.version;
  const counts = tab.sync.counts;
  const changes = counts.spots + counts.grains + counts.labels;
  const images = tab.sync.pixelsChanged || !!tab.sync.draft?.publish_job && tab.sync.draft.publish_job.decision !== 'keep' ? 1 : 0;
  const outbox = tab.tims.store?.data.outbox.length ?? 0;
  return { title: tab.title, changes, images, outbox };
}

export function needsCloseReminder(tab: AppState): boolean {
  return !tab.readOnly && (tab.dirty || tab.sync.unsynced || syncReminder(tab).outbox > 0);
}

export function reminderCounts(item: ReturnType<typeof syncReminder>): string {
  return [
    `${item.changes} ${item.changes === 1 ? 'change' : 'changes'} not synced`,
    ...(item.images ? [`${item.images} image set pending`] : []),
    ...(item.outbox ? [`${item.outbox} outbox ${item.outbox === 1 ? 'entry' : 'entries'}`] : []),
  ].join(' · ');
}

/** Close only after the existing local persistence queue has finished successfully. */
export async function keepLocalForClose(tab: AppState): Promise<void> {
  flushWrites();
  await appStorage.flushed();
  await tab.tims.store?.persist();
  if (tab.storageWarning) throw new Error(tab.storageWarning);
}

/** Use the normal guarded flows; a conflict, registration or image decision leaves the tab open. */
export async function syncForClose(tab: AppState): Promise<boolean> {
  await keepLocalForClose(tab);
  if (tab.sync.unsynced) {
    await tab.sync.sync();
    if (!tab.sync.error && !tab.sync.staleOpen && !tab.sync.review && !tab.sync.confirmation && !tab.sync.dialog
      && (tab.sync.pixelsChanged || tab.sync.draft?.publish_job && tab.sync.draft.publish_job.decision !== 'keep')) await tab.sync.publishImages();
  }
  if (!tab.sync.unsynced) {
    tab.tims.store?.mountSynced();
    await tab.tims.store?.flush();
  }
  await keepLocalForClose(tab);
  const pending = tab.sync.unsynced || syncReminder(tab).outbox > 0;
  if (pending) {
    tab.notice = tab.sync.error || tab.sync.imageError || tab.tims.store?.error
      || (tab.sync.reachable === false ? 'Offline. Your work is saved locally in the outbox. Sync next time you are online.'
        : 'Sync needs your attention. Resolve it, then try closing again. Your work is saved locally.');
  }
  return !pending;
}
