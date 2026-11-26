import { describe, expect, it } from 'vitest';
import { headerSort, lastChangeTime, mountColumns, sortFromHeader } from './open-presentation';

describe('Open last-change display', () => {
  const now = Date.parse('2026-11-26T12:00:00Z');
  it('uses compact relative units at each boundary', () => {
    for (const [age, text] of [[0, 'just now'], [59_999, 'just now'], [60_000, '1 min ago'], [3_599_999, '59 min ago'], [3_600_000, '1 h ago'], [86_399_999, '23 h ago'], [86_400_000, '1 d ago'], [3 * 86_400_000, '3 d ago'], [29 * 86_400_000, '29 d ago'], [30 * 86_400_000, '1 mo ago'], [365 * 86_400_000, '1 y ago']] as const) {
      expect(lastChangeTime(new Date(now - age).toISOString(), now)).toBe(text);
    }
  });
  it('handles missing, malformed and future timestamps', () => {
    expect(lastChangeTime(null, now)).toBe('No changes yet');
    expect(lastChangeTime(undefined, now)).toBe('No changes yet');
    expect(lastChangeTime('', now)).toBe('No changes yet');
    expect(lastChangeTime('invalid', now)).toBe('unknown time');
    expect(lastChangeTime(new Date(now + 1000).toISOString(), now)).toBe('just now');
  });
});

describe('Open sort headers', () => {
  it('maps only existing server options and toggles both ways', () => {
    expect(mountColumns.filter(c => c.sort).map(c => c.sort)).toEqual(['label', 'last_changed']);
    expect(sortFromHeader('-last_changed', 'label')).toBe('label');
    expect(sortFromHeader('label', 'label')).toBe('-label');
    expect(sortFromHeader('-label', 'label')).toBe('label');
    expect(sortFromHeader('creator', 'last_changed')).toBe('-last_changed');
    expect(sortFromHeader('-last_changed', 'last_changed')).toBe('last_changed');
    expect(sortFromHeader('last_changed', 'last_changed')).toBe('-last_changed');
    for (const key of ['sample', 'grain_count', 'spot_count', 'bogus']) expect(sortFromHeader('-created', key)).toBe('-created');
  });
  it('reflects sort-field and direction changes in aria-sort', () => {
    expect(headerSort('-label', 'label')).toBe('descending');
    expect(headerSort('label', 'label')).toBe('ascending');
    expect(headerSort('-created', 'label')).toBe('none');
    expect(headerSort('-last_changed', 'last_changed')).toBe('descending');
    expect(headerSort('label', null)).toBeUndefined();
  });
});
