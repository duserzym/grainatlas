import { describe, expect, it, vi } from 'vitest';
import { progressState, segmentedIndex, segmentedKeydown } from './ui-controls';

describe('segmented control keyboard navigation', () => {
  it.each([
    ['ArrowLeft', 0, 3, 2], ['ArrowRight', 2, 3, 0],
    ['ArrowRight', 0, 3, 1], ['ArrowLeft', 2, 3, 1],
    ['Home', 2, 3, 0], ['End', 0, 3, 2],
    ['ArrowLeft', 0, 1, 0], ['ArrowRight', 0, 1, 0],
    ['Home', 0, 0, null], ['End', 0, 0, null],
    ['Enter', 1, 3, null], [' ', 1, 3, null], ['Tab', 0, 3, null],
  ] as const)('%s from %s among %s options selects %s', (key, index, length, next) => {
    expect(segmentedIndex(key, index, length)).toBe(next);
  });

  function strip(key: string, modifiers: Partial<KeyboardEvent> = {}) {
    const buttons = Array.from({ length: 3 }, () => ({ click: vi.fn(), focus: vi.fn() }));
    const querySelectorAll = vi.fn(() => buttons);
    const event = { key, currentTarget: { querySelectorAll }, target: buttons[0],
      preventDefault: vi.fn(), stopPropagation: vi.fn(), ...modifiers };
    segmentedKeydown(event as unknown as KeyboardEvent);
    return { buttons, event, querySelectorAll };
  }
  it('activates and focuses the wrapped radio, consuming the key before canvas shortcuts', () => {
    const { buttons, event, querySelectorAll } = strip('ArrowLeft');
    expect(querySelectorAll).toHaveBeenCalledWith('button[role="radio"]:not(:disabled)');
    expect(buttons[2].click).toHaveBeenCalledOnce();
    expect(buttons[2].focus).toHaveBeenCalledOnce();
    expect(buttons[0].click).not.toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
  });
  it.each(['altKey', 'ctrlKey', 'metaKey'] as const)('leaves %s shortcuts alone', modifier => {
    const { buttons, event } = strip('ArrowRight', { [modifier]: true });
    expect(buttons.every(button => button.click.mock.calls.length === 0)).toBe(true);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
  it('leaves Tab and native button activation alone', () => {
    const { buttons, event } = strip('Tab');
    expect(buttons.every(button => button.click.mock.calls.length === 0)).toBe(true);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
  it('ignores events outside the radio buttons', () => {
    const { event } = strip('Home', { target: null });
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
});

describe('progress accessibility values', () => {
  it.each([null, undefined, NaN, Infinity])('keeps an unknown value (%s) empty without aria-valuenow', value => {
    expect(progressState(value, 8, 'tiles')).toEqual({ max: 8, now: undefined, percent: 0, caption: '— of 8 tiles' });
  });
  it('clamps values below zero and above the total', () => {
    expect(progressState(-4, 10, 'tiles')).toEqual({ max: 10, now: 0, percent: 0, caption: '0 of 10 tiles' });
    expect(progressState(12, 10, 'tiles')).toEqual({ max: 10, now: 10, percent: 100, caption: '10 of 10 tiles' });
  });
  it.each([0, -1, NaN, Infinity])('keeps an invalid or empty total (%s) empty', total => {
    expect(progressState(3, total, 'tiles')).toEqual({ max: 0, now: 0, percent: 0, caption: '0 of 0 tiles' });
  });
  it('exposes the full count and fraction for known progress', () => {
    expect(progressState(3, 12, 'tiles')).toEqual({ max: 12, now: 3, percent: 25, caption: '3 of 12 tiles' });
  });
});
