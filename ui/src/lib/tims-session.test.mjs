import assert from 'node:assert/strict';
import test from 'node:test';
import { compileModule } from 'svelte/compiler';
import ts from 'typescript';
import { registerHooks } from 'node:module';
import { flush } from 'svelte/internal/client';

const items = new Map();
let mirrorWrites = 0;
globalThis.__timsMirror = { getItem: key => items.get(key) ?? null, setItem: (key, value) => { mirrorWrites++; items.set(key, value); } };
const windowBefore = globalThis.window, documentBefore = globalThis.document, fetchBefore = globalThis.fetch;
const clientEntry = new URL('index-client.js', import.meta.resolve('svelte')).href;
globalThis.window = new EventTarget();
globalThis.document = Object.assign(new EventTarget(), { visibilityState: 'visible' });
const harnessUrl = new URL('./tims-session-harness.svelte.ts', import.meta.url).href;
const hooks = registerHooks({
  resolve(specifier, context, next) {
    if (specifier === 'svelte') return { url: clientEntry, shortCircuit: true };
    if (specifier === harnessUrl) return { url: harnessUrl, shortCircuit: true };
    try { return next(specifier, context); }
    catch (error) {
      if (specifier.startsWith('.') && (!/\.[a-z]+$/.test(specifier) || specifier.endsWith('.svelte'))) return next(specifier + '.ts', context);
      throw error;
    }
  },
  load(url, context, next) {
    if (url.endsWith('/atlas.ts')) return { format: 'module', shortCircuit: true, source: 'export const authToken = async app => app.link.token || null;' };
    if (url.endsWith('/mount-mirror.ts')) return { format: 'module', shortCircuit: true, source: 'export const mountMirror = globalThis.__timsMirror;' };
    const result = url === harnessUrl ? { source: `
      import { TimsSession } from './tims-store.svelte.ts';
      export function harness() {
        let app = $state({ server: 'https://tims.example.test', remote: null,
          project: { uuid: 'project-1', upbdb: { mount_id: 'mount-1' } },
          link: { token: 'token-a', syncAutomatically: true }, readOnly: false,
          sync: { syncedAt: null, draft: null, status: null } });
        const session = new TimsSession(app);
        const stop = $effect.root(() => session.start());
        return { app, session, stop };
      }` } : next(url, context);
    if (url.endsWith('.svelte.ts')) {
      const source = ts.transpileModule(String(result.source), { compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext } }).outputText;
      return { format: 'module', shortCircuit: true, source: compileModule(source, { filename: new URL(url).pathname, generate: 'client' }).js.code };
    }
    return result;
  },
});
const { harness } = await import(harnessUrl);
const { hideMountCache } = await import('./mount-browser.ts');
async function until(predicate) {
  for (let i = 0; i < 200; i++) { flush(); if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 2)); }
  assert.ok(predicate(), 'reactive TIMS session settled');
}
test.after(() => { hooks.deregister(); globalThis.window = windowBefore; globalThis.document = documentBefore; globalThis.fetch = fetchBefore; delete globalThis.__timsMirror; });

test('real rune session hides on sign-out, isolates accounts, resumes one writer and keeps viewer reads off disk', async () => {
  let offline = true, next = 0;
  const sent = [], fractions = [];
  globalThis.fetch = async (input, init = {}) => {
    const url = new URL(input), headers = new Headers(init.headers);
    if (url.pathname === '/v1/accounts/me') return Response.json({ id: headers.get('Authorization') === 'Bearer token-a' ? 'account-a' : 'account-b', username: 'synthetic', can_manage_people: false });
    if (url.pathname === '/v1/server') return Response.json({ dataset_id: 'dataset-1' });
    if (url.pathname === '/v1/labs') return Response.json([]);
    if (url.pathname === '/v1/fractions' && init.method === 'POST') {
      sent.push(headers.get('Idempotency-Key'));
      if (offline) throw new TypeError('Offline');
      const body = JSON.parse(init.body);
      const fraction = { ...body, id: `fraction-${++next}`, status: 'picked', sample: { id: 'sample-1', name: 'Synthetic' }, sources: body.sources.map(s => ({ ...s, id: 'source-1' })) };
      fractions.push(fraction); return Response.json(fraction);
    }
    if (url.pathname === '/v1/fractions') return Response.json(fractions);
    throw new Error(`Unexpected test request ${url.pathname}`);
  };
  const first = harness(), second = harness();
  try {
    await until(() => first.session.store && second.session.store);
    assert.equal(first.session.store, second.session.store, 'two tabs share one durable writer');
    const store = first.session.store;
    await store.createFraction({ name: 'z1', mineral: 'zircon', batch_id: null, capsule_number: null, notes: '', sources: [{ atlas_grain_id: 'grain-1' }] }, { id: 'sample-1', name: 'Synthetic' });
    await store.flush(); const key = store.data.outbox[0].key;
    assert.equal(first.session.badges.get('grain-1').state, 'pending');
    first.app.link.token = ''; second.app.link.token = ''; hideMountCache();
    assert.equal(first.session.store, null); assert.equal(second.session.store, null);
    await until(() => !store.enabled);
    assert.ok([...items.values()].some(value => value.includes(key)), 'unsent key remains durable on sign-out');
    first.app.link.token = 'token-b';
    await until(() => first.session.store);
    assert.equal(first.session.store.data.outbox.length, 0); assert.equal(first.session.badges.size, 0);
    offline = false; first.app.link.token = 'token-a';
    await until(() => first.session.store?.key.includes('/account-a/'));
    await first.session.store.flush();
    assert.equal(first.session.store.data.outbox.length, 0);
    assert.equal(sent.at(-1), key); assert.equal(next, 1);
    first.app.readOnly = true;
    await until(() => first.session.store && first.session.store !== store);
    const writes = mirrorWrites;
    await first.session.store.refresh();
    assert.equal(mirrorWrites, writes, 'viewer revalidation writes no local mirror');
  } finally { first.stop(); second.stop(); }
});
