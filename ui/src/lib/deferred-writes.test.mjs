import assert from 'node:assert/strict';
import test from 'node:test';
import { DeferredWrites } from './deferred-writes.ts';

function harness() {
  let scheduled, schedules = 0, cancellations = 0;
  const writes = new DeferredWrites(run => { scheduled = run; schedules++; return () => { scheduled = null; cancellations++; }; });
  return { writes, paint: () => scheduled?.(), get schedules() { return schedules; }, get cancellations() { return cancellations; } };
}
test('a burst writes the latest state once after paint, then can schedule another burst', () => {
  const h = harness(), key = {}, saved = [];
  let state = 0;
  for (state = 1; state <= 60; state++) h.writes.enqueue(key, () => saved.push(state));
  state = 60;
  assert.deepEqual(saved, []);
  assert.equal(h.schedules, 1);
  h.paint();
  assert.deepEqual(saved, [60]);
  state = 61; h.writes.enqueue(key, () => saved.push(state)); h.paint();
  assert.deepEqual(saved, [60, 61]);
});
test('flush on demand writes the latest state once and cancels the pending callback', () => {
  const h = harness(), key = {}, saved = [];
  h.writes.enqueue(key, () => saved.push('old'));
  h.writes.enqueue(key, () => saved.push('latest'));
  h.writes.flush(); h.writes.flush(); h.paint();
  assert.deepEqual(saved, ['latest']);
  assert.equal(h.cancellations, 1);
});
test('flush drains a project and the draft it enqueues, with reentrant storage reads', () => {
  const h = harness(), project = {}, draft = {}, saved = [];
  h.writes.enqueue(project, () => {
    h.writes.flush();
    saved.push('project');
    h.writes.enqueue(draft, () => { h.writes.flush(); saved.push('draft'); });
  });
  h.writes.flush(); h.paint();
  assert.deepEqual(saved, ['project', 'draft']);
  assert.equal(h.schedules, 1);
});

test('browser lifecycle boundaries synchronously drain the latest edits', async t => {
  const previous = { window: globalThis.window, document: globalThis.document, requestAnimationFrame: globalThis.requestAnimationFrame, cancelAnimationFrame: globalThis.cancelAnimationFrame };
  const window = new EventTarget(), document = new EventTarget();
  document.visibilityState = 'visible';
  let frame;
  Object.assign(globalThis, { window, document, requestAnimationFrame: run => { frame = run; return 1; }, cancelAnimationFrame: () => { frame = null; } });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const { deferredWrites } = await import('./deferred-writes.ts?lifecycle');
    const key = {}, saved = [];
    let state = 1;
    deferredWrites.enqueue(key, () => saved.push(state));
    state = 2;
    assert.deepEqual(saved, []);
    frame();
    t.mock.timers.tick(79);
    assert.deepEqual(saved, []);
    t.mock.timers.tick(1);
    assert.deepEqual(saved, [2], 'normal writes follow the paint and trailing timer');
    for (const event of ['beforeunload', 'pagehide', 'visibilitychange']) {
      state++;
      deferredWrites.enqueue(key, () => saved.push(state));
      if (event === 'visibilitychange') { document.visibilityState = 'hidden'; document.dispatchEvent(new Event(event)); }
      else window.dispatchEvent(new Event(event));
      assert.equal(saved.at(-1), state);
    }
    t.mock.timers.tick(250);
    assert.deepEqual(saved, [2, 3, 4, 5]);
    state = 6;
    deferredWrites.enqueue(key, () => saved.push(state));
    t.mock.timers.tick(199);
    assert.equal(saved.at(-1), 5);
    t.mock.timers.tick(1);
    assert.equal(saved.at(-1), 6, 'throttled frames still write within 200 ms');
  } finally {
    t.mock.timers.reset();
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete globalThis[key]; else globalThis[key] = value;
    }
  }
});

test('background work yields between project and draft; a forced flush still drains both', () => {
  const h=harness(),saved=[];
  h.writes.enqueue({},()=>{saved.push('project');h.writes.enqueue({},()=>saved.push('draft'));});
  h.paint();assert.deepEqual(saved,['project']);
  h.paint();assert.deepEqual(saved,['project','draft']);
  h.writes.enqueue({},()=>{saved.push('project2');h.writes.enqueue({},()=>saved.push('draft2'));});
  h.paint();h.writes.flush();h.paint();assert.deepEqual(saved,['project','draft','project2','draft2']);
});
