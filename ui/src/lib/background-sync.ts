/** Timers are independent of Svelte so edit coalescing and retry policy share one implementation. */
export class BackgroundSync {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private failures = 0;
  private run: () => void;
  private random: () => number;
  private editDelay: number;
  constructor(run: () => void, random: () => number = Math.random, editDelay = 3000) { this.run = run; this.random = random; this.editDelay = editDelay; }
  get scheduled() { return this.timer !== undefined; }
  edit() { this.schedule(this.editDelay); }
  ensure() { if (!this.scheduled) this.edit(); }
  retry(after = 0) {
    if (this.scheduled) return;
    const delay = Math.min(300_000, 5000 * 2 ** Math.min(this.failures++, 6));
    this.schedule(Math.max(after, Math.min(300_000, delay * (0.8 + this.random() * 0.4))));
  }
  wake() { this.cancel(); this.schedule(0); }
  reset() { this.failures = 0; this.cancel(); }
  cancel() { if (this.timer !== undefined) clearTimeout(this.timer); this.timer = undefined; }
  private schedule(delay: number) {
    this.cancel();
    this.timer = setTimeout(() => { this.timer = undefined; this.run(); }, delay);
  }
}
