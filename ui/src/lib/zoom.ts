import { primaryModifier } from './platform';

export type ZoomAction = 'in' | 'out' | 'fit';
export interface ZoomCommand { action: ZoomAction; interface: boolean }
type ZoomKey = Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'isComposing'>;
const ZOOM_CODES: Readonly<Partial<Record<string, ZoomAction>>> = {
  Equal: 'in', NumpadAdd: 'in', Minus: 'out', NumpadSubtract: 'out', Digit0: 'fit', Numpad0: 'fit',
};

export function matchZoomKey(e: ZoomKey, platform?: string): ZoomCommand | null {
  const mac = primaryModifier(platform) === 'Cmd';
  if (e.isComposing || !(mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey)) return null;
  // Shift is only part of the plus shortcut; leave other Shift commands alone.
  if (e.shiftKey && e.key !== '+') return null;
  // Prefer the typed symbol on non-US layouts; codes also cover numpad and Option symbols.
  const action: ZoomAction | undefined = e.key === '=' || e.key === '+' ? 'in'
    : e.key === '-' ? 'out' : e.key === '0' ? 'fit'
    : ZOOM_CODES[e.code];
  if (!action) return null;
  return { action, interface: e.altKey };
}

export function isTextFieldTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return !!el && (['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) || el.isContentEditable === true);
}

export interface ZoomView {
  zoomBy(factor: number): void;
  fit(): void;
  visible(): boolean;
}

/** Mounted views own their registrations. Priority wins, then the latest visible view. */
export class ZoomViewRegistry {
  private views = new Map<ZoomView, number>();

  register(view: ZoomView, priority = 0): () => void {
    this.views.set(view, priority);
    return () => { this.views.delete(view); };
  }

  active(): ZoomView | undefined {
    let chosen: ZoomView | undefined;
    let priority = -Infinity;
    for (const [view, rank] of this.views) {
      if (rank >= priority && view.visible()) { chosen = view; priority = rank; }
    }
    return chosen;
  }
}

export const zoomViews = new ZoomViewRegistry();

export function registerZoomView(host: HTMLElement, view: Pick<ZoomView, 'zoomBy' | 'fit'>): () => void {
  return zoomViews.register({ ...view, visible: () => {
    if (!host.isConnected || host.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
    const rect = host.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 && getComputedStyle(host).visibility === 'visible';
  } });
}

export function zoomMenuCommand(id: string): ZoomCommand | null {
  switch (id) {
    case 'zoom-in': return { action: 'in', interface: false };
    case 'zoom-out': return { action: 'out', interface: false };
    case 'zoom-fit': return { action: 'fit', interface: false };
    case 'interface-larger': return { action: 'in', interface: true };
    case 'interface-smaller': return { action: 'out', interface: true };
    case 'interface-reset': return { action: 'fit', interface: true };
    default: return null;
  }
}

/** True only when handled: web callers must leave browser interface zoom alone. */
export function dispatchZoom(command: ZoomCommand, target: EventTarget | null, options: {
  views: ZoomViewRegistry;
  desktop: boolean;
  interfaceZoom(action: ZoomAction): void;
  actOnce?(): boolean;
}): boolean {
  const view = !command.interface && !isTextFieldTarget(target) ? options.views.active() : undefined;
  if (!view && !options.desktop) return false;
  if (options.actOnce && !options.actOnce()) return true;
  if (!view) options.interfaceZoom(command.action);
  else if (command.action === 'fit') view.fit();
  else view.zoomBy(command.action === 'in' ? 1.5 : 1 / 1.5);
  return true;
}
