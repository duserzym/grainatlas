import { spotDiameter } from './spot-size.ts';

type Sized = { id: string; area_id: string; diameter_um?: number | null };
export interface SizeEditState<T extends Sized> {
  spots: T[];
  diameter: number;
  readOnly: boolean;
  historyRevision: number;
  checkpoint(): void;
  persist(): void;
}

/** One explicit size transaction, shared by every canvas size control. */
export class SpotSizeEdit<T extends Sized> {
  private last: { state: SizeEditState<T>; scope: string; ids: string; spots: T[]; revision: number; at: number } | null = null;

  change(state: SizeEditState<T>, scope: string, area: string | null, ids: readonly string[], diameter: number, at = performance.now()) {
    if (state.readOnly || spotDiameter({ diameter_um: diameter }) === null) return;
    if (!ids.length) {
      state.diameter = diameter;
      this.last = null;
      return;
    }
    const selected = new Set(ids);
    const next = state.spots.map(s => s.area_id === area && selected.has(s.id) && s.diameter_um !== diameter ? { ...s, diameter_um: diameter } : s);
    if (next.every((s, i) => s === state.spots[i])) return;
    const key = JSON.stringify([...selected].sort());
    const last = this.last;
    // An intervening edit, Undo/Redo or context switch always starts a new run.
    if (!(last && last.state === state && last.scope === scope && last.ids === key && last.spots === state.spots && last.revision === state.historyRevision && at - last.at < 2000)) state.checkpoint();
    state.spots = next;
    state.persist();
    this.last = { state, scope, ids: key, spots: state.spots, revision: state.historyRevision, at };
  }
}
