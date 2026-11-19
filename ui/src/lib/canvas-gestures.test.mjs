import assert from 'node:assert/strict';
import test from 'node:test';
import { attachGestures } from './canvas/gestures.ts';

function harness(t, takesDrag = true) {
  const previous = Object.fromEntries(['window', 'document', 'requestAnimationFrame', 'cancelAnimationFrame']
    .map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const window = new EventTarget(), document = { activeElement: null };
  Object.assign(globalThis, { window, document, requestAnimationFrame: () => 1, cancelAnimationFrame: () => {} });
  const captures = new Set(), calls = { press: [], drag: [], release: [], cancel: 0, click: [] };
  class Host extends EventTarget {
    closest() { return null; }
    hasAttribute() { return false; }
    getBoundingClientRect() { return { left: 0, top: 0, width: 800, height: 600 }; }
    focus() { document.activeElement = this; }
    contains(el) { return el === this; }
    setPointerCapture(id) { captures.add(id); }
    hasPointerCapture(id) { return captures.has(id); }
    releasePointerCapture(id) { captures.delete(id); }
  }
  const host = new Host();
  const detach = attachGestures({ host,
    getView: () => ({ cx: 0, cy: 0, w: 800, h: 600, zoom: 1, rot: 0 }),
    setView: () => {}, fit: () => {},
    onpress: p => { calls.press.push(p); return takesDrag; }, ondrag: p => calls.drag.push(p),
    onrelease: p => calls.release.push(p), oncancel: () => calls.cancel++, onclick: p => calls.click.push(p),
  });
  t.after(() => {
    detach();
    for (const [key, descriptor] of Object.entries(previous)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key];
    }
  });
  const send = (type, props = {}) => {
    const ev = new Event(type, { cancelable: true });
    Object.assign(ev, { pointerId: 1, pointerType: 'mouse', button: 0, clientX: 100, clientY: 100, ...props });
    host.dispatchEvent(ev);
    return ev;
  };
  return { host, window, document, captures, calls, send };
}

for (const interruption of ['pointercancel', 'lostpointercapture', 'blur']) {
  for (const moved of [false, true]) {
    test(`${interruption} ${moved ? 'during a box' : 'before click slop'} cancels once without release or click`, t => {
      const h = harness(t);
      h.send('pointerdown');
      if (moved) h.send('pointermove', { clientX: 200 });
      if (interruption === 'blur') h.window.dispatchEvent(new Event('blur')); else h.send(interruption);
      h.send('lostpointercapture'); h.send('pointerup');
      assert.equal(h.calls.cancel, 1);
      assert.equal(h.calls.release.length, 0);
      assert.equal(h.calls.click.length, 0);
      assert.equal(h.captures.size, 0);
      h.send('pointerdown'); h.send('pointermove', { clientX: 200 }); h.send('pointerup');
      assert.equal(h.calls.release.length, 1, 'a new gesture works after interruption');
    });
  }
}

for (const takesDrag of [true, false]) {
  for (const nextEvent of ['pointerup', 'pointermove']) {
    test(`releasing pending capture before ${nextEvent} cancels a ${takesDrag ? 'tool' : 'possible click'} without a lost event`, t => {
      const h = harness(t, takesDrag);
      h.send('pointerdown', { ctrlKey: true });
      // Before the next pointer event, capture is pending. Releasing it here
      // clears that request without a gotpointercapture/lostpointercapture pair.
      h.host.releasePointerCapture(1);
      h.send(nextEvent, { ctrlKey: true });
      h.send('pointerup', { ctrlKey: true });
      assert.equal(h.calls.cancel, takesDrag ? 1 : 0);
      assert.equal(h.calls.drag.length, 0);
      assert.equal(h.calls.release.length, 0);
      assert.equal(h.calls.click.length, 0);
      h.send('pointerdown'); h.send('pointerup');
      assert.equal(h.calls.release.length, takesDrag ? 1 : 0, 'the next tool gesture works');
      assert.equal(h.calls.click.length, takesDrag ? 0 : 1, 'the next click works');
    });
  }
}

test('pointer capture APIs may be absent without canceling every gesture', t => {
  const h = harness(t);
  h.host.setPointerCapture = undefined;
  h.host.hasPointerCapture = undefined;
  h.host.releasePointerCapture = undefined;
  h.send('pointerdown'); h.send('pointermove', { clientX: 200 }); h.send('pointerup');
  assert.equal(h.calls.drag.length, 1);
  assert.equal(h.calls.release.length, 1);
  assert.equal(h.calls.cancel, 0);
});

for (const pointerType of ['mouse', 'pen', 'touch']) {
  test(`${pointerType} primary drag focuses the canvas and retains capture until release`, t => {
    const h = harness(t);
    assert.equal(h.send('pointerdown', { pointerType }).defaultPrevented, true);
    assert.equal(h.document.activeElement, h.host);
    assert.ok(h.captures.has(1));
    h.send('pointermove', { pointerType, button: -1, clientX: 500 });
    h.send('pointerup', { pointerType, clientX: 500 }); h.send('lostpointercapture');
    assert.equal(h.calls.press.length, 1);
    assert.equal(h.calls.drag.length, 1);
    assert.equal(h.calls.release.length, 1);
    assert.equal(h.calls.cancel, 0);
    assert.equal(h.captures.size, 0);
  });
}

test('second finger cancels the box; pinch and remaining finger never release it', t => {
  const h = harness(t);
  h.send('pointerdown', { pointerType: 'touch' });
  h.send('pointermove', { pointerType: 'touch', clientX: 200 });
  h.send('pointerdown', { pointerType: 'touch', pointerId: 2, clientX: 300 });
  h.send('pointermove', { pointerType: 'touch', pointerId: 2, clientX: 350 });
  h.send('pointerup', { pointerId: 2 });
  h.send('pointermove', { clientX: 400 }); h.send('pointerup');
  assert.equal(h.calls.cancel, 1);
  assert.equal(h.calls.release.length, 0);
  assert.equal(h.calls.click.length, 0);
  assert.equal(h.calls.drag.length, 1);
});

test('unrelated pointer events and right buttons cannot finish a box', t => {
  const h = harness(t);
  h.send('pointerdown', { button: 2 }); h.send('pointerup', { pointerId: 3 });
  assert.equal(h.calls.press.length, 0);
  assert.equal(h.captures.size, 0);
  h.send('pointerdown');
  h.send('pointermove', { pointerId: 3 }); h.send('pointerup', { pointerId: 3 });
  assert.equal(h.calls.drag.length, 0);
  assert.equal(h.calls.release.length, 0);
  h.send('pointerup');
  assert.equal(h.calls.release.length, 1);
});

test('native drag starts are prevented; cancellation of a possible click never clicks', t => {
  const h = harness(t, false);
  assert.equal(h.send('dragstart').defaultPrevented, true);
  h.send('pointerdown'); h.send('pointercancel'); h.send('pointerup');
  assert.equal(h.calls.click.length, 0);
});

test('canvas UI controls keep their focus and default pointer behaviour', t => {
  const h = harness(t);
  h.host.closest = selector => selector === '[data-ui]' ? h.host : null;
  assert.equal(h.send('pointerdown').defaultPrevented, false);
  assert.equal(h.send('dragstart').defaultPrevented, false);
  assert.equal(h.document.activeElement, null);
  assert.equal(h.calls.press.length, 0);
});
