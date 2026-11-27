import { timed } from './sync-perf.ts';
/** Coalesce edits until after a paint. Storage readers and lifecycle boundaries
 * drain this queue synchronously; writers must read the latest state when run. */
export class DeferredWrites {
  private pending = new Map<object, () => void>();
  private cancel: (() => void) | null = null;
  private flushing = false;
  private schedule: (run: () => void) => () => void;
  private nextTask: (run: () => void) => () => void;
  constructor(schedule: (run: () => void) => () => void, nextTask = schedule) { this.schedule = schedule; this.nextTask = nextTask; }
  enqueue(key: object, write: () => void): void {
    this.pending.set(key, write);
    if (!this.cancel && !this.flushing) this.cancel = this.schedule(() => this.step());
  }
  /** Background persistence drains one callback per task. Explicit readers and
   * lifecycle boundaries still synchronously drain everything via flush(). */
  private step(): void {
    this.cancel?.(); this.cancel = null;
    if (this.flushing) return;
    this.flushing = true;
    try {
      const next = this.pending.entries().next().value;
      if (next) { this.pending.delete(next[0]); timed('persistence.batch', next[1]); }
    } finally {
      this.flushing = false;
      if (this.pending.size && !this.cancel) this.cancel = this.nextTask(() => this.step());
    }
  }
  flush(): void {
    if (this.flushing) return;
    this.cancel?.(); this.cancel = null;
    this.flushing = true;
    try {
      // A project write can enqueue its draft. Drain both at this boundary.
      while (this.pending.size) {
        const [key, write] = this.pending.entries().next().value!;
        this.pending.delete(key);
        write();
      }
    } finally { this.flushing = false; }
  }
}

function afterPaint(run: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  // The fallback also bounds the delay in a background/throttled window.
  const deadline = setTimeout(run, 200);
  const frame = typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => { timer = setTimeout(run, 80); }) : undefined;
  return () => {
    clearTimeout(deadline); clearTimeout(timer);
    if (frame !== undefined) cancelAnimationFrame(frame);
  };
}
export const deferredWrites = new DeferredWrites(afterPaint, run => {
  const timer = setTimeout(run, 0);
  return () => clearTimeout(timer);
});
export const flushWrites = () => timed('persistence.flush', () => deferredWrites.flush());

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', flushWrites, { capture: true });
  window.addEventListener('pagehide', flushWrites);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushWrites(); });
}
