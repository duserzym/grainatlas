import { FrameUpdate } from './frame-update.ts';
// Bridge v4 adapter: pure snapshots and command completion, injectable for unit tests.
import { ATLAS_EVENTS, atlasStatus, atlasDrafts, atlasSynced, commandResult, isBridgeCommand, isHosted, notify,
  type AtlasStatusEvent, type AtlasSyncedEvent, type BridgeCommand, type BridgeEvent, type CommandResult } from '@lab/bridge';
import { SyncApiError } from './sync-api.ts';
import type { Draft } from './drafts.ts';
import { draftSyncStatus } from './sync-status.ts';
export type AtlasCommand = Exclude<BridgeCommand, { type: 'focus-spot' | 'focus-grain' | 'review-spots' | 'set-presentation' }>;
export class HostedSync {
  private lastStatus = '';
  private statusFrame = new FrameUpdate();
  private lastDrafts = '';
  private queue: Promise<void> = Promise.resolve();
  private send: (event: BridgeEvent) => unknown;
  constructor(send: (event: BridgeEvent) => unknown) { this.send = send; }
  status(value: AtlasStatusEvent, force = false) {
    const event = atlasStatus(value);
    if (!event) throw new Error('Invalid atlas status');
    const key = JSON.stringify(event);
    const send = () => { if (force || key !== this.lastStatus) { this.lastStatus = key; this.send(event); } };
    if (force) { this.statusFrame.cancel(); send(); } else this.statusFrame.enqueue(send);
  }
  drafts(drafts: Draft[], force = false) {
    const event = atlasDrafts({ type: ATLAS_EVENTS.drafts, drafts: drafts.flatMap(d => {
      const status = draftSyncStatus(d);
      return status.pending ? [{ project: d.uuid, name: d.name, mount: d.mount, baseRevision: d.base_revision, updatedAt: d.updated_at, changes: { ...(d.changes ?? { spots: 0, grains: 0, labels: 0 }), pixels: status.pixels } }] : [];
    }) });
    if (!event) throw new Error('Invalid atlas drafts');
    const key = JSON.stringify(event);
    if (force || key !== this.lastDrafts) { this.lastDrafts = key; this.send(event); }
  }
  synced(value: Omit<AtlasSyncedEvent, 'type'>) {
    const event = atlasSynced({ type: ATLAS_EVENTS.synced, ...value });
    if (!event) throw new Error('Invalid atlas sync notice');
    this.send(event);
  }
  /** Serialise mutations even on an older shell; emit one result after the awaited action. */
  command(value: unknown, run: (cmd: AtlasCommand) => Promise<Partial<CommandResult> | void>): Promise<void> {
    if (!isBridgeCommand(value) || ['focus-spot', 'focus-grain', 'review-spots', 'set-presentation'].includes(value.type)) return Promise.resolve();
    const cmd = value as AtlasCommand;
    const execute = async () => {
      let result: Partial<CommandResult> = {};
      try { result = await run(cmd) ?? {}; }
      catch (e) { result = { ok: false, code: 'failed', error: (e instanceof SyncApiError ? e.messages[0] : e instanceof Error ? e.message : String(e)).slice(0, 300) }; }
      if (cmd.type !== 'report-status' && cmd.type !== 'atlas-synced' && cmd.id) {
        const event = commandResult({ type: ATLAS_EVENTS.commandResult, id: cmd.id, command: cmd.type, project: 'project' in cmd ? cmd.project : undefined, ok: true, ...result });
        if (!event) throw new Error('Invalid atlas command result');
        this.send(event);
      }
    };
    const pending = this.queue.then(execute);
    this.queue = pending.catch(() => {});
    return pending;
  }
}
export const hostedSync = new HostedSync(event => { if (isHosted()) void notify(event).catch(() => {}); });
export { ATLAS_EVENTS };
