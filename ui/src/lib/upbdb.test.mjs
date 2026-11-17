import assert from 'node:assert/strict';
import test from 'node:test';
import { ApiError, createMount } from './upbdb.ts';

const mount = {
  id: '20000000-0000-4000-8000-000000000002', label: 'Synthetic mount', mount_type: 'grain_tape',
  grain_count: 0, spot_count: 0, can_edit: true, sample_count: 0, projects: [], prepared_at: null, atlas: null,
};

test('createMount posts JSON with a bearer token and returns the 201 mount', async t => {
  const body = { label: mount.label, mount_type: mount.mount_type, notes: 'Synthetic notes' };
  const fetch = t.mock.method(globalThis, 'fetch', async (url, init) => {
    assert.equal(url, 'https://upbdb.test/v1/mounts');
    assert.equal(init.method, 'POST');
    assert.deepEqual(init.headers, { Authorization: 'Bearer test-token', 'Content-Type': 'application/json' });
    assert.deepEqual(JSON.parse(init.body), body);
    return Response.json(mount, { status: 201 });
  });
  assert.deepEqual(await createMount('https://upbdb.test', 'test-token', body), mount);
  assert.equal(fetch.mock.callCount(), 1);
});

test('createMount omits optional notes when none are supplied', async t => {
  const body = { label: mount.label, mount_type: 'epoxy_1in' };
  t.mock.method(globalThis, 'fetch', async (_url, init) => {
    assert.deepEqual(JSON.parse(init.body), body);
    return Response.json({ ...mount, ...body }, { status: 201 });
  });
  await createMount('https://upbdb.test', 'test-token', body);
});

for (const [status, body, message] of [
  [409, { detail: 'A mount with this label already exists.', code: 'label_taken' }, 'A mount with this label already exists.'],
  [422, { detail: 'Label must be 1–100 characters.' }, 'Label must be 1–100 characters.'],
  [422, { detail: [{ loc: ['body', 'label'], msg: 'Label is required.' }] }, 'Label is required.'],
]) {
  test(`createMount surfaces the ${status} server detail: ${message}`, async t => {
    t.mock.method(console, 'warn', () => {});
    t.mock.method(globalThis, 'fetch', async () => Response.json(body, { status }));
    await assert.rejects(createMount('https://upbdb.test', 'test-token', { label: mount.label, mount_type: 'epoxy_1in' }),
      e => e instanceof ApiError && e.status === status && e.message === `Creating mount: ${message}`);
  });
}
