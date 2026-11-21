import { parseDiameter } from './spots.ts';

/** A focus transaction belongs to one target and is invalidated by external value changes. */
export class SizeEditor {
  private value: number | null | undefined;
  private target: string | undefined;
  private editing: { target: string; write: (d: number) => void } | null = null;
  private stale = false;
  sync(value: number | null, target: string): string | null {
    if (Object.is(value, this.value) && target === this.target) return null;
    if (this.editing) this.stale = true;
    this.value = value; this.target = target;
    return value === null ? '' : String(value);
  }
  focus(target: string, write: (d: number) => void) { this.editing = { target, write }; this.stale = false; }
  blur() { this.editing = null; this.stale = false; }
  commit(text: string, value: number | null, target: string): number | null {
    this.sync(value, target);
    const d = parseDiameter(text);
    if (!this.editing || this.stale || this.editing.target !== target || d === null) return null;
    // Account for our own committed update before Svelte reflects it back into props.
    this.value = d;
    if (d !== value) this.editing.write(d);
    return d;
  }
}
