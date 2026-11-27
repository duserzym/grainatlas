// Ported from upblab ui/src/lib/ui-controls.ts at 72e91bc; keep in step.
export interface SegmentedOption { value: string; label: string; title?: string; ariaLabel?: string; testid?: string; disabled?: boolean }
export interface WorkflowStep {
  label: string; fullName: string; detail: string;
  state: 'done' | 'current' | 'pending'; hint?: string;
}

/** Unhandled keys leave native button behaviour alone; arrows wrap. */
export function segmentedIndex(key: string, current: number, length: number): number | null {
  if (length < 1) return null;
  const index = Math.max(0, Math.min(current, length - 1));
  switch (key) {
    case 'ArrowLeft': return (index + length - 1) % length;
    case 'ArrowRight': return (index + 1) % length;
    case 'Home': return 0;
    case 'End': return length - 1;
    default: return null;
  }
}

export function progressState(value: number | null | undefined, total: number, noun: string) {
  const max = Number.isFinite(total) ? Math.max(0, total) : 0;
  const now = value == null || !Number.isFinite(value) ? undefined : Math.max(0, Math.min(value, max));
  return { max, now, percent: now === undefined || max === 0 ? 0 : now / max * 100,
    caption: `${now ?? '—'} of ${max} ${noun}` };
}

// Local changes: tab navigation and vertical menu navigation for Canvas.
/** Use radio keyboard navigation without changing a measured editor strip's geometry. */
export function segmentedKeydown(event: KeyboardEvent) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const group = event.currentTarget as HTMLElement;
  const buttons = [...group.querySelectorAll<HTMLButtonElement>('button[role="radio"]:not(:disabled)')];
  const index = buttons.indexOf(event.target as HTMLButtonElement);
  if (index < 0) return;
  const next = segmentedIndex(event.key, index, buttons.length);
  if (next === null) return;
  event.preventDefault(); event.stopPropagation();
  buttons[next].click();
  buttons[next].focus();
}

/** Tabs share the radio group's wrapping focus and activation model. */
export function tabKeydown(event: KeyboardEvent) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const group = event.currentTarget as HTMLElement;
  const buttons = [...group.querySelectorAll<HTMLButtonElement>('button[role="tab"]:not(:disabled)')];
  const index = buttons.indexOf(event.target as HTMLButtonElement);
  if (index < 0) return;
  const next = segmentedIndex(event.key, index, buttons.length);
  if (next === null) return;
  event.preventDefault(); event.stopPropagation(); buttons[next].click(); buttons[next].focus();
}

export function menuIndex(key: string, current: number, length: number): number | null {
  return segmentedIndex(key === 'ArrowDown' ? 'ArrowRight' : key === 'ArrowUp' ? 'ArrowLeft' : key === 'ArrowLeft' || key === 'ArrowRight' ? '' : key, current, length);
}
