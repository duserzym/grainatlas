/** Latest UI notification per animation frame. Non-browser consumers stay
 * synchronous; explicit command reports can flush/cancel immediately. */
export class FrameUpdate {
  private pending: (() => void) | null = null;
  private frame: number | null = null;
  enqueue(run: () => void): void {
    if (typeof requestAnimationFrame !== 'function') { run(); return; }
    this.pending = run;
    if (this.frame === null) this.frame = requestAnimationFrame(() => { this.frame = null; this.flush(); });
  }
  flush(): void {
    const run = this.pending;
    this.cancel();
    run?.();
  }
  cancel(): void {
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.frame = null; this.pending = null;
  }
}
