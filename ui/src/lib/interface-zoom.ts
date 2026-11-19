import type { ZoomAction } from './zoom';

export const INTERFACE_ZOOM_KEY = 'grainatlas.interfaceZoom';
export const INTERFACE_ZOOM_STEPS = [0.5, 0.67, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;

export function stepInterfaceZoom(value: number, action: ZoomAction): number {
  if (action === 'fit') return 1;
  if (action === 'in') return INTERFACE_ZOOM_STEPS.find(step => step > value) ?? 2;
  return [...INTERFACE_ZOOM_STEPS].reverse().find(step => step < value) ?? 0.5;
}

type ZoomStorage = Pick<Storage, 'getItem' | 'setItem'>;

/** Per-user UI preference, independent of projects. Serialize native calls, including startup. */
export class InterfaceZoom {
  private value = 1;
  private pending = Promise.resolve();

  constructor(private setZoom: (factor: number) => void | Promise<void>,
    private storage: () => ZoomStorage | undefined = () => globalThis.localStorage) {
    try {
      const saved = Number(storage()?.getItem(INTERFACE_ZOOM_KEY));
      if (INTERFACE_ZOOM_STEPS.some(step => step === saved)) this.value = saved;
    } catch { /* storage may be unavailable */ }
  }

  restore(): Promise<void> { return this.enqueue(() => this.apply(this.value)); }

  change(action: ZoomAction): Promise<void> {
    return this.enqueue(() => this.apply(stepInterfaceZoom(this.value, action)));
  }

  private enqueue(run: () => Promise<void>): Promise<void> {
    const next = this.pending.then(run);
    this.pending = next.catch(() => {});
    return next;
  }

  private async apply(value: number): Promise<void> {
    await this.setZoom(value);
    this.value = value;
    try { this.storage()?.setItem(INTERFACE_ZOOM_KEY, String(value)); } catch { /* keep zoom usable */ }
  }
}
