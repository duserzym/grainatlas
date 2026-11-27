import type { Draft } from './drafts.ts';
import { FrameUpdate } from './frame-update.ts';
import { timed } from './sync-perf.ts';
/** Draft saves already have the record. Only deletion/unknown changes need a
 * fresh storage listing; keep listing order and coalesce reports before paint. */
export class HostedDraftReports {
  private frame = new FrameUpdate();
  private drafts: Draft[] | null = null;
  private load: () => Draft[];
  private send: (drafts: Draft[]) => void;
  constructor(load: () => Draft[], send: (drafts: Draft[]) => void) { this.load = load; this.send = send; }
  changed(draft?: Draft): void {
    if (draft && this.drafts) {
      const index = this.drafts.findIndex(d => d.uuid === draft.uuid);
      this.drafts = index < 0 ? [...this.drafts, draft] : this.drafts.map((d, i) => i === index ? draft : d);
    } else this.drafts = null;
    this.frame.enqueue(() => timed('hosted.drafts', () => { this.drafts ??= this.load(); this.send(this.drafts); }));
  }
  dispose(): void { this.frame.cancel(); }
}
