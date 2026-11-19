import type { Core } from './core/core';
import type { PrintRegistration, SheetStats, WholeResult, Progress } from './core/types';

/** Cooperative page boundaries let the worker service canvas/edit requests and
 * let the caller leave/cancel without terminating its project session. */
export class PrintQuality {
  next = 0;
  ready = false;
  error: Error | null = null;
  residentBytes: number;
  summary: { registration: PrintRegistration; stats: SheetStats | WholeResult['stats'] } | null = null;
  private active = false;
  private task: Promise<void> | null = null;
  private listeners = new Set<() => void>();
  constructor(readonly core: Core, readonly job: number, readonly total: number, previewBytes = 0) { this.residentBytes = previewBytes; }
  subscribe(listener: () => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private changed() { for (const listener of this.listeners) listener(); }
  stop(): Promise<void> { this.active = false; return this.task?.catch(() => {}) ?? Promise.resolve(); }
  release() {
    this.stop();
    void (this.task ?? Promise.resolve()).catch(() => {}).then(() => this.core.printTask(this.job, 'cancel')).catch(() => {});
  }
  start(): Promise<void> {
    this.active = true;
    if (this.ready) return Promise.resolve();
    if (this.task) return this.task;
    this.error = null;
    this.task = this.prepare().finally(() => { this.task = null; this.changed(); });
    // Background failures are presented by the screen; export awaits the same task.
    void this.task.catch(() => {});
    return this.task;
  }
  private async prepare() {
    try {
      // Let the first preview paint before queuing CPU work on the worker.
      await new Promise(resolve => setTimeout(resolve, 32));
      while (this.active && this.next < this.total) {
        const status = JSON.parse(new TextDecoder().decode(await this.core.printTask(this.job, 'quality', this.next)));
        this.residentBytes = status.resident_bytes ?? this.residentBytes;
        this.next++;
        this.changed();
        await new Promise(resolve => setTimeout(resolve, 0));
      }
      if (this.next === this.total && !this.ready) {
        this.summary = JSON.parse(new TextDecoder().decode(await this.core.printTask(this.job, 'summary')));
        this.ready = true;
        this.changed();
      }
    } catch (error) { this.error = error instanceof Error ? error : new Error(String(error)); this.changed(); throw error; }
  }
  async wait(progress: (p: Progress) => void, cancelled: () => boolean) {
    const report = () => progress({ stage: `Preparing print quality: page ${Math.min(this.next + 1, this.total)} of ${this.total}`, fraction: this.next / this.total });
    const unsubscribe = this.subscribe(report);
    try { report(); await this.start(); if (cancelled() || !this.ready) throw new Error('cancelled'); }
    finally { unsubscribe(); }
  }
  async pdf(progress: (p: Progress) => void, cancelled: () => boolean): Promise<Uint8Array> {
    await this.wait(progress, cancelled);
    if (cancelled()) throw new Error('cancelled');
    progress({ stage: 'Starting PDF', fraction: 0 });
    let finished = false;
    try {
      await this.core.printTask(this.job, 'pdf_begin');
      for (const action of ['pdf_prepare', 'pdf_page']) {
        for (let page = 0; page < this.total; page++) {
          if (cancelled()) throw new Error('cancelled');
          progress({ stage: `${action === 'pdf_prepare' ? 'Preparing PDF fonts' : 'Writing PDF'}: page ${page + 1} of ${this.total}`, fraction: (page / this.total + (action === 'pdf_page' ? 1 : 0)) / 2 });
          await this.core.printTask(this.job, action, page);
        }
      }
      if (cancelled()) throw new Error('cancelled');
      progress({ stage: 'Finishing PDF', fraction: 1 });
      const bytes = await this.core.printTask(this.job, 'pdf_finish');
      finished = true; return bytes;
    } finally { if (!finished) await this.core.printTask(this.job, 'pdf_abort').catch(() => {}); }
  }
}

export function printPdfName(mount: string, date = new Date()): string {
  let stem = mount.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/g, '').trim() || 'grainatlas';
  // Windows reserves these stems even when an extension is present.
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(stem)) stem = `_${stem}`;
  return `${stem}_print_${date.toISOString().slice(0, 10)}.pdf`;
}
