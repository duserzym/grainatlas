// Standalone desktop app only: tell UPb Lab that a mount changed, by one small JSON file in
// UPb Lab's inbox (written by the notify_upblab Tauri command). Hosted, UPb Lab hears it
// through the bridge (hosted-sync.ts); the web build has no such channel. Failures are logged.
import { isHosted } from '@lab/bridge';

export function notifyUpbLab(mountId: string, projectUuid: string, revision: number, server: string | null | undefined): void {
  try {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window) || isHosted() || !server) return;
  } catch { return; }
  void import('@tauri-apps/api/core')
    .then(({ invoke }) => invoke('notify_upblab', { mountId, projectUuid, revision, server }))
    .catch(e => console.warn('Could not leave a notice for UPb Lab:', e));
}
