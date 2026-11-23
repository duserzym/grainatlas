import type { SyncState } from './sync';

/** Start-screen opens alone opt into catch-up. Attention flows always take priority. */
export function catchUpAction(input: {
  state: SyncState; localChanges: boolean; newer?: boolean; automatic?: boolean;
  staleOpen?: boolean; review?: boolean; confirmation?: boolean; imageDecision?: boolean;
  intentFailure?: boolean; held?: boolean; linkConflict?: boolean;
}): 'update' | 'review' | 'none' {
  if (input.staleOpen || input.review || input.confirmation || input.imageDecision || input.intentFailure || input.held || input.linkConflict) return 'none';
  if (input.state === 'server-newer' && !input.localChanges) return 'update';
  if (input.state === 'both-changed' || input.localChanges && input.newer && !input.automatic) return 'review';
  return 'none';
}
