import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';

const hooks = registerHooks({ load(url, context, next) {
  if (url.endsWith('/canvas/SpotPreview.svelte')) return {
    format: 'module', shortCircuit: true,
    source: compile(readFileSync(new URL(url), 'utf8'), { filename: new URL(url).pathname, generate: 'server' }).js.code,
  };
  return next(url, context);
} });
const { default: SpotPreview } = await import('./canvas/SpotPreview.svelte');
test.after(() => hooks.deregister());

test('snapped and free previews draw the actual diameter at the placement centre; leave clears it', () => {
  for (const diameter of [15, 30, 50]) for (const snap of [null, { x: 100, y: 200, what: 'synthetic centre' }]) {
    const body = render(SpotPreview, { props: { position: [10, 20], snap, diameter, k: .5, colour: '#fff', halo: '#111', contrast: 'light' } }).body;
    const ghost = body.match(/<circle[^>]*data-testid="spot-ghost"[^>]*>/)?.[0];
    assert.ok(ghost);
    assert.match(ghost, new RegExp(`r="${diameter / 2}"`));
    assert.match(ghost, new RegExp(`cx="${snap?.x ?? 10}"`));
    assert.match(ghost, new RegExp(`cy="${snap?.y ?? 20}"`));
  }
  assert.doesNotMatch(render(SpotPreview, { props: { position: null, snap: null, diameter: 30, k: .5, colour: '#fff', halo: '#111', contrast: 'light' } }).body, /data-testid="spot-ghost"/);
});
