import { createSubscriber } from 'svelte/reactivity';
import { isBridgeCommand, type Presentation, type ViewerInset } from '@lab/bridge';

/** The shell controls presentation. Editor is also the standalone/older-shell default. */
export class PresentationState {
  private value: Presentation = 'editor';
  private queue = Promise.resolve();
  private listeners = new Set<(value: Presentation) => void>();
  private flushers = new Set<() => Promise<void>>();
  private observe = createSubscriber(update => this.subscribe(() => update()));
  get current(): Presentation { this.observe(); return this.value; }
  get writable(): boolean { return this.current === 'editor'; }
  // v5.1: the mount inset as UPb Lab remembers it, kept in memory only (the viewer writes nothing).
  private insetValue: ViewerInset | null = null;
  private insetListeners = new Set<() => void>();
  private observeInset = createSubscriber(update => {
    this.insetListeners.add(update);
    return () => { this.insetListeners.delete(update); };
  });
  get inset(): ViewerInset | null { this.observeInset(); return this.insetValue; }
  /** The inset changed (received from the shell, or set by the user): memory only. */
  keepInset(value: ViewerInset) {
    if (this.insetValue?.width === value.width && this.insetValue.collapsed === value.collapsed) return;
    this.insetValue = { width: value.width, collapsed: value.collapsed };
    for (const listener of this.insetListeners) listener();
  }
  subscribe(listener: (value: Presentation) => void) {
    this.listeners.add(listener); listener(this.value);
    return () => { this.listeners.delete(listener); };
  }
  beforeViewer(flush: () => Promise<void>) {
    this.flushers.add(flush);
    return () => { this.flushers.delete(flush); };
  }
  command(value: unknown, hosted: boolean): Promise<void> {
    if (!hosted || !isBridgeCommand(value) || value.type !== 'set-presentation') return Promise.resolve();
    const next = value.presentation;
    const inset = next === 'viewer' ? value.inset : undefined;
    const job = this.queue.then(async () => {
      if (inset) this.keepInset(inset);
      if (next === this.value) return;
      // Finish editor work before notifying the UI or closing the write gate.
      if (next === 'viewer') for (const flush of this.flushers) await flush();
      this.value = next;
      for (const listener of this.listeners) listener(next);
    });
    this.queue = job.catch(() => {});
    return job;
  }
}
export const presentation = new PresentationState();
export const canWrite = () => presentation.writable;
export function requireWrite() {
  if (!canWrite()) throw new Error('This viewer is read-only.');
}
