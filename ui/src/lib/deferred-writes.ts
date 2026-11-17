/** Coalesce edits until after a paint. Storage readers and lifecycle boundaries
 * drain this queue synchronously; writers must read the latest state when run. */
export class DeferredWrites {
  private pending = new Map<object, () => void>();
  private cancel: (() => void) | null = null;
  private flushing = false;
  private schedule: (run: () => void) => () => void;
  constructor(schedule: (run: () => void) => () => void) { this.schedule = schedule; }
  enqueue(key: object, write: () => void): void {
    this.pending.set(key, write);
    if (!this.cancel && !this.flushing) this.cancel = this.schedule(() => this.flush());
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
export const deferredWrites = new DeferredWrites(afterPaint);
export const flushWrites = () => deferredWrites.flush();

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', flushWrites, { capture: true });
  window.addEventListener('pagehide', flushWrites);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushWrites(); });
}
