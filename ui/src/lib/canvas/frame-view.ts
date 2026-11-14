import type { View } from './view';

/** Gesture calculations see the latest value immediately; reactive consumers see one per frame. */
export function frameView(read: () => View, publish: (view: View) => void,
  request = requestAnimationFrame, cancel = cancelAnimationFrame) {
  let pending: View | null = null;
  let base: View | null = null;
  let frame: number | null = null;
  const clear = () => {
    if (frame !== null) cancel(frame);
    frame = null; pending = base = null;
  };
  const get = () => {
    const current = read();
    // Fit, resize and focus can replace the view between gesture events.
    if (pending && current !== base) clear();
    return pending ?? current;
  };
  return {
    get,
    set(view: View) {
      get();
      base ??= read();
      pending = view;
      if (frame === null) frame = request(() => {
        const next = get();
        const changed = pending !== null;
        frame = null; pending = base = null;
        if (changed) publish(next);
      });
    },
    clear,
  };
}
