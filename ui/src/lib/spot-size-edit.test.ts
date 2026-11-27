import { describe, it, expect } from 'vitest';
import { SpotSizeEdit } from './spot-size-edit';

function fixture() {
  return {
    spots: [{ id: 'one', area_id: 'a', diameter_um: 15 }, { id: 'two', area_id: 'a', diameter_um: 40 }, { id: 'away', area_id: 'b', diameter_um: 20 }],
    diameter: 25, readOnly: false, historyRevision: 0, checkpoints: 0, writes: 0,
    checkpoint() { this.checkpoints++; this.historyRevision++; },
    persist() { this.writes++; },
  };
}

describe('explicit spot size transactions', () => {
  it('changes only the next size without history, persistence or registered edits', () => {
    const state = fixture(), spots = state.spots;
    new SpotSizeEdit().change(state, 'project/a', 'a', [], 30);
    expect(state.diameter).toBe(30); expect(state.spots).toBe(spots);
    expect(state.checkpoints).toBe(0); expect(state.writes).toBe(0);
  });
  it('replaces only selected current-area spots and groups repeated steps', () => {
    const state = fixture(), before = state.spots, edit = new SpotSizeEdit();
    edit.change(state, 'project/a', 'a', ['one', 'two', 'away'], 30, 0);
    edit.change(state, 'project/a', 'a', ['two', 'one', 'away'], 31, 1);
    expect(state.spots.map(s => s.diameter_um)).toEqual([31, 31, 20]);
    expect(state.spots[2]).toBe(before[2]); expect(before.map(s => s.diameter_um)).toEqual([15, 40, 20]);
    expect(state.checkpoints).toBe(1); expect(state.writes).toBe(2); expect(state.diameter).toBe(31);
  });
  it('starts a fresh undo transaction after unrelated edits, undo/redo, replacement, scope or time changes', () => {
    for (const interrupt of [
      (s: ReturnType<typeof fixture>) => { s.historyRevision++; },
      (s: ReturnType<typeof fixture>) => { s.spots = [...s.spots]; },
    ]) {
      const state = fixture(), edit = new SpotSizeEdit();
      edit.change(state, 'project/a', 'a', ['one'], 30, 0);
      interrupt(state);
      edit.change(state, 'project/a', 'a', ['one'], 31, 1);
      expect(state.checkpoints).toBe(2);
    }
    const state = fixture(), edit = new SpotSizeEdit();
    edit.change(state, 'project/a', 'a', ['one'], 30, 0);
    edit.change(state, 'other/a', 'a', ['one'], 31, 1);
    edit.change(state, 'other/a', 'a', ['one'], 32, 2002);
    expect(state.checkpoints).toBe(3);
  });
  it('rejects invalid sizes and read-only edits', () => {
    const state = fixture(), edit = new SpotSizeEdit(), spots = state.spots;
    for (const diameter of [NaN, Infinity, 0, -1]) edit.change(state, 'p', 'a', ['one'], diameter);
    state.readOnly = true; edit.change(state, 'p', 'a', ['one'], 30); edit.change(state, 'p', 'a', [], 30);
    expect(state.spots).toBe(spots); expect(state.diameter).toBe(25);
    expect(state.checkpoints).toBe(0); expect(state.writes).toBe(0);
  });
  it('an already matching selection still sets the next size without a registered edit', () => {
    const state = fixture(), spots = state.spots;
    new SpotSizeEdit().change(state, 'p', 'a', ['one'], 15);
    expect(state.diameter).toBe(15); expect(state.spots).toBe(spots);
    expect(state.checkpoints).toBe(0); expect(state.writes).toBe(0);
  });
});
