import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import affected from '../ui/tests/affected.mjs';
import { choose } from './quick-check.mjs';

test('sync and Canvas select their scenario samples and smoke', () => {
  assert.deepEqual([...choose(['ui/src/lib/sync.ts']).specs].sort(),
    ['hosted-sync.spec.ts', 'smoke.spec.ts', 'sync.spec.ts']);
  assert.deepEqual([...choose(['ui/src/screens/Canvas.svelte']).specs].sort(),
    ['gestures.spec.ts', 'grain-edit.spec.ts', 'smoke.spec.ts', 'spot-cursor.spec.ts', 'spot-numbers.spec.ts']);
});

test('mixed core and UI changes keep native tests and rebuild wasm before browsers', () => {
  const plan = choose(['crates/grainatlas-core/src/lib.rs', 'crates/grainatlas-cli/src/main.rs', 'ui/src/lib/unknown.ts']);
  assert.ok(plan.core && plan.cli && plan.ui && plan.browser);
  assert.ok(plan.reasons.some(reason => reason.includes('map has no entry for ui/src/lib/unknown.ts')));
  assert.deepEqual([...plan.specs], ['smoke.spec.ts']);
});

test('docs do no work; configuration and unknown paths require full', () => {
  assert.equal(choose(['docs/x.png', 'site/index.html', 'README.md']).reasons.length, 0);
  for (const file of ['Makefile', 'Cargo.lock', 'ui/pnpm-lock.yaml', 'ui/package.json',
    'ui/playwright.config.ts', 'scripts/quick-check.mjs', 'unknown']) assert.ok(choose([file]).full, file);
});

test('wasm, desktop, unit helpers and browser helpers select the appropriate tier', () => {
  assert.ok(choose(['crates/grainatlas-wasm/src/lib.rs']).wasm);
  assert.ok(choose(['tauri/src-tauri/build.rs']).desktop);
  assert.ok(choose(['ui/unit/helpers/x.ts']).ui);
  assert.ok(choose(['ui/tests/helpers/storage.ts']).browser);
  assert.ok(choose(['ui/tests/example.spec.ts']).specs.has('example.spec.ts'));
});

test('every mapped scenario exists', () => {
  for (const specs of Object.values(affected)) for (const spec of specs)
    assert.ok(existsSync(`ui/tests/${spec}.spec.ts`), spec);
});
