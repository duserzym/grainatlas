import { describe, expect, it, vi } from 'vitest';
import { commandDeduper } from './menu-command';
import { InterfaceZoom, INTERFACE_ZOOM_KEY, INTERFACE_ZOOM_STEPS, stepInterfaceZoom } from './interface-zoom';
import { dispatchZoom, isTextFieldTarget, matchZoomKey, zoomMenuCommand, ZoomViewRegistry, type ZoomView } from './zoom';

function key(key: string, code = '', patch: Partial<KeyboardEvent> = {}) {
  return { key, code, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, isComposing: false, ...patch };
}
function field(tagName: string, isContentEditable = false): EventTarget {
  return { tagName, isContentEditable } as unknown as EventTarget;
}
function view(visible = true): ZoomView {
  return { zoomBy: vi.fn(), fit: vi.fn(), visible: () => visible };
}

describe('zoom keys', () => {
  for (const platform of ['MacIntel', 'Win32', 'Linux x86_64']) {
    const modifier = platform === 'MacIntel' ? { metaKey: true, ctrlKey: false } : { metaKey: false, ctrlKey: true };
    it(`matches primary modifier, logical keys and physical keys on ${platform}`, () => {
      for (const [value, code, action] of [
        ['=', 'Equal', 'in'], ['+', 'Equal', 'in'], ['+', 'BracketRight', 'in'],
        ['-', 'Minus', 'out'], ['0', 'Digit0', 'fit'],
        ['-', 'Equal', 'out'], ['0', 'Equal', 'fit'],
        ['Add', 'NumpadAdd', 'in'], ['Subtract', 'NumpadSubtract', 'out'], ['Insert', 'Numpad0', 'fit'],
        // Option changes the logical key on macOS; physical codes still work.
        ['≠', 'Equal', 'in'], ['–', 'Minus', 'out'], ['º', 'Digit0', 'fit'],
      ]) {
        for (const altKey of [false, true]) {
          expect(matchZoomKey(key(value, code, { ...modifier, altKey }), platform)).toEqual({ action, interface: altKey });
        }
      }
      expect(matchZoomKey(key('+', 'Equal', { ...modifier, shiftKey: true }), platform)).toEqual({ action: 'in', interface: false });
      expect(matchZoomKey(key('+', 'NumpadAdd', { ...modifier, altKey: true }), platform)).toEqual({ action: 'in', interface: true });
      expect(matchZoomKey(key('=', 'Equal', { metaKey: !modifier.metaKey, ctrlKey: !modifier.ctrlKey }), platform)).toBeNull();
      expect(matchZoomKey(key('=', 'Equal', { metaKey: false, ctrlKey: false }), platform)).toBeNull();
      expect(matchZoomKey(key('=', 'Equal', { metaKey: true, ctrlKey: true }), platform)).toBeNull();
      for (const [value, code] of [['_', 'Minus'], [')', 'Digit0'], ['=', 'Equal'], ['a', 'KeyA']]) {
        expect(matchZoomKey(key(value, code, { ...modifier, shiftKey: true }), platform)).toBeNull();
      }
      expect(matchZoomKey(key('a', 'KeyA', modifier), platform)).toBeNull();
      expect(matchZoomKey(key('=', 'Equal', { ...modifier, isComposing: true }), platform)).toBeNull();
    });
  }

  it('recognizes all text fields, including descendants inheriting contenteditable', () => {
    for (const target of [field('INPUT'), field('TEXTAREA'), field('SELECT'), field('DIV', true), field('SPAN', true)]) {
      expect(isTextFieldTarget(target)).toBe(true);
    }
    for (const target of [null, field('BUTTON'), field('DIV'), field('SVG')]) expect(isTextFieldTarget(target)).toBe(false);
  });
});

it('chooses visible registered views by priority, then recency, and unregisters on destroy', () => {
  const views = new ZoomViewRegistry();
  expect(views.active()).toBeUndefined();
  const first = view(), second = view(), overlay = view(), hidden = view(false);
  const removeFirst = views.register(first);
  const removeSecond = views.register(second);
  expect(views.active()).toBe(second);
  const removeOverlay = views.register(overlay, 10);
  const removeHidden = views.register(hidden, 20);
  expect(views.active()).toBe(overlay);
  overlay.visible = () => false;
  expect(views.active()).toBe(second);
  first.visible = () => false;
  removeSecond();
  expect(views.active()).toBeUndefined();
  first.visible = () => true;
  expect(views.active()).toBe(first);
  removeFirst(); removeFirst(); removeOverlay(); removeHidden();
  expect(views.active()).toBeUndefined();
});

it('routes view zoom and fit, desktop fallback and explicit interface zoom; web leaves browser keys alone', () => {
  const views = new ZoomViewRegistry(), canvas = view(), interfaceZoom = vi.fn();
  const unregister = views.register(canvas);
  for (const desktop of [false, true]) {
    const options = { views, desktop, interfaceZoom };
    for (const action of ['in', 'out', 'fit'] as const) expect(dispatchZoom({ action, interface: false }, null, options)).toBe(true);
    expect(canvas.zoomBy).toHaveBeenCalledWith(1.5);
    expect(canvas.zoomBy).toHaveBeenCalledWith(1 / 1.5);
    expect(canvas.fit).toHaveBeenCalled();
    for (const target of [field('INPUT'), field('TEXTAREA'), field('SELECT'), field('DIV', true)]) {
      vi.mocked(canvas.zoomBy).mockClear();
      expect(dispatchZoom({ action: 'in', interface: false }, target, options)).toBe(desktop);
      expect(canvas.zoomBy).not.toHaveBeenCalled();
    }
    vi.mocked(canvas.zoomBy).mockClear();
    expect(dispatchZoom({ action: 'out', interface: true }, null, options)).toBe(desktop);
    expect(canvas.zoomBy).not.toHaveBeenCalled();
  }
  expect(interfaceZoom).toHaveBeenCalledWith('in');
  expect(interfaceZoom).toHaveBeenCalledWith('out');
  unregister(); interfaceZoom.mockClear();
  expect(dispatchZoom({ action: 'fit', interface: false }, null, { views, desktop: false, interfaceZoom })).toBe(false);
  expect(interfaceZoom).not.toHaveBeenCalled();
  expect(dispatchZoom({ action: 'fit', interface: false }, null, { views, desktop: true, interfaceZoom })).toBe(true);
  expect(interfaceZoom).toHaveBeenCalledWith('fit');
});

it('pairs native menu and key events in either order and preserves rapid presses and repeats', () => {
  let now = 0;
  const once = commandDeduper(() => now);
  const views = new ZoomViewRegistry(), canvas = view();
  views.register(canvas);
  const run = (source: 'key' | 'menu') => dispatchZoom(zoomMenuCommand('zoom-in')!, null, {
    views, desktop: true, interfaceZoom: vi.fn(), actOnce: () => once('zoom-in', source),
  });
  for (const source of ['key', 'menu', 'menu', 'key', 'key', 'key', 'menu', 'menu'] as const) {
    expect(run(source)).toBe(true);
    now += 10;
  }
  expect(canvas.zoomBy).toHaveBeenCalledTimes(4);
  run('key');
  now += 401;
  run('menu');
  expect(canvas.zoomBy).toHaveBeenCalledTimes(6);
  expect(once('new-tab', 'key')).toBe(true);
  expect(once('close-tab', 'menu')).toBe(true);
  expect(once('new-tab', 'menu')).toBe(false);
});

it('maps every native zoom menu item to the matching keyboard command', () => {
  for (const [id, action, isInterface] of [
    ['zoom-in', 'in', false], ['zoom-out', 'out', false], ['zoom-fit', 'fit', false],
    ['interface-larger', 'in', true], ['interface-smaller', 'out', true], ['interface-reset', 'fit', true],
  ]) expect(zoomMenuCommand(id as string)).toEqual({ action, interface: isInterface });
  expect(zoomMenuCommand('new-tab')).toBeNull();
});

describe('interface zoom', () => {
  it('steps through every size, clamps at either end, and resets to 100 %', () => {
    const steps = INTERFACE_ZOOM_STEPS;
    for (let i = 0; i < steps.length; i++) {
      expect(stepInterfaceZoom(steps[i], 'in')).toBe(steps[Math.min(i + 1, steps.length - 1)]);
      expect(stepInterfaceZoom(steps[i], 'out')).toBe(steps[Math.max(i - 1, 0)]);
      expect(stepInterfaceZoom(steps[i], 'fit')).toBe(1);
    }
    expect(stepInterfaceZoom(1.2, 'in')).toBe(1.25);
    expect(stepInterfaceZoom(1.2, 'out')).toBe(1.1);
  });

  it('restores the user preference and serializes rapid native calls before persisting successful zoom', async () => {
    const storage = { getItem: vi.fn(() => '1.25'), setItem: vi.fn() };
    const native = vi.fn(async (_factor: number) => {});
    const zoom = new InterfaceZoom(native, () => storage);
    await Promise.all([zoom.restore(), zoom.change('in'), zoom.change('in'), zoom.change('out'), zoom.change('fit')]);
    expect(native.mock.calls.map(args => args[0])).toEqual([1.25, 1.5, 1.75, 1.5, 1]);
    expect(storage.getItem).toHaveBeenCalledWith(INTERFACE_ZOOM_KEY);
    expect(storage.setItem).toHaveBeenLastCalledWith(INTERFACE_ZOOM_KEY, '1');
  });

  it('uses 100 % for missing or invalid preferences and works with blocked storage', async () => {
    for (const saved of [null, '', 'garbage', 'Infinity', '0.2', '9', '1.2']) {
      const native = vi.fn();
      await new InterfaceZoom(native, () => ({ getItem: () => saved, setItem: vi.fn() })).restore();
      expect(native).toHaveBeenCalledWith(1);
    }
    for (const storage of [() => undefined, () => { throw new Error('blocked'); },
      () => ({ getItem: () => '1.5', setItem: () => { throw new Error('full'); } })]) {
      const native = vi.fn();
      const zoom = new InterfaceZoom(native, storage);
      await zoom.change('fit');
      expect(native).toHaveBeenCalledWith(1);
    }
  });

  it('reports native failures without persisting them and allows retry', async () => {
    const storage = { getItem: () => '1', setItem: vi.fn() };
    const native = vi.fn().mockRejectedValueOnce(new Error('zoom unavailable')).mockResolvedValue(undefined);
    const zoom = new InterfaceZoom(native, () => storage);
    await expect(zoom.change('in')).rejects.toThrow('zoom unavailable');
    expect(storage.setItem).not.toHaveBeenCalled();
    await zoom.change('in');
    expect(native).toHaveBeenNthCalledWith(2, 1.1);
    expect(storage.setItem).toHaveBeenCalledWith(INTERFACE_ZOOM_KEY, '1.1');
  });
});
