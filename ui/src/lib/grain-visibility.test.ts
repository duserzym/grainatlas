import { expect, it } from 'vitest';
import { shownGrains } from './grain-visibility';

const grains = [{ id: 'g1' }, { id: 'g2' }, { id: 'g3' }];
const spots = [{ id: 's1', grain_id: 'g1' }, { id: 's2', grain_id: 'g2' }, { id: 's3', grain_id: null }];
const copy = (g: { id: string }) => g.id === 'g3';

it('shows every grain when nothing is hidden', () => {
  expect(shownGrains(grains, copy, true, null, spots).map((g) => g.id)).toEqual(['g1', 'g2', 'g3']);
});

it('drops neighbouring-area copies when they are switched off', () => {
  expect(shownGrains(grains, copy, false, null, spots).map((g) => g.id)).toEqual(['g1', 'g2']);
});

it('keeps only grains with a spot in the Review subset', () => {
  expect(shownGrains(grains, copy, true, new Set(['s2']), spots).map((g) => g.id)).toEqual(['g2']);
  expect(shownGrains(grains, copy, true, new Set(), spots)).toEqual([]);
});
