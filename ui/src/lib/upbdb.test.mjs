import assert from 'node:assert/strict';
import test from 'node:test';
import { ApiError, clearMountAcquisitionLabels, createMount, labelsForChecks, mountAcquisitionLabels, mountAcquisitions } from './upbdb.ts';

const mount = {
  id: '20000000-0000-4000-8000-000000000002', label: 'Synthetic mount', mount_type: 'grain_tape',
  grain_count: 0, spot_count: 0, can_edit: true, sample_count: 0, projects: [], prepared_at: null, atlas: null,
};

test('check labels replace all acquisitions linked to a renamed spot with its current name', () => {
  const acquisitions = [
    { id: 'acq-1', label: 'S1_12' },
    { id: 'acq-2', label: 'S1_12_repeat' },
    { id: 'acq-3', label: 'S3_1' },
  ];
  const spots = [
    { name: 'S2-12', acquisition_ids: ['acq-1', 'acq-2'] },
    { name: 'S3-1' },
    { name: 'S4-1', acquisition_ids: [] },
  ];
  assert.deepEqual(labelsForChecks(acquisitions, spots), ['S2-12', 'S3_1']);
  assert.deepEqual(spots[0].acquisition_ids, ['acq-1', 'acq-2']);
  assert.equal(acquisitions[0].label, 'S1_12');
});

test('check labels keep acquisitions whose linked spot is outside the given project', () => {
  const acquisitions = [{ id: 'acq-elsewhere', label: 'S1_12' }];
  assert.deepEqual(labelsForChecks(acquisitions, []), ['S1_12']);
  assert.deepEqual(labelsForChecks(acquisitions, [{ name: 'S2-12', acquisition_ids: ['acq-local'] }]), ['S1_12', 'S2-12']);
});

test('check labels preserve unlinked labels and deduplicate labels and linked spot names', () => {
  const acquisitions = [{ id: 'acq-1', label: 'S1_1' }, { id: 'acq-2', label: 'S1_1' }];
  assert.deepEqual(labelsForChecks(acquisitions, [{ name: 'S2-1' }, { name: 'S3-1', acquisition_ids: [] }]), ['S1_1']);
  assert.deepEqual(labelsForChecks(acquisitions, [{ name: 'S1_1', acquisition_ids: ['acq-1'] }]), ['S1_1']);
});

test('mount acquisition records and labels share requests, filtering and cache invalidation at scale', async t => {
  const base = 'https://upbdb.test';
  clearMountAcquisitionLabels(base, mount.id);
  t.after(() => clearMountAcquisitionLabels(base, mount.id));
  const sessions = Array.from({ length: 6 }, (_, i) => ({ id: `session-${i}` }));
  const rows = Array.from({ length: 2500 }, (_, i) => ({ id: `acq-${i}`, instrument_label: `S1_${i}`, reference_material_name: '' }));
  let active = 0, peak = 0;
  const fetch = t.mock.method(globalThis, 'fetch', async (url, init) => {
    assert.equal(init.headers.Authorization, 'Bearer test-token');
    active++;
    peak = Math.max(peak, active);
    await new Promise(resolve => setTimeout(resolve, 1));
    active--;
    const { pathname, searchParams } = new URL(url);
    if (pathname === '/v1/sessions') return Response.json(sessions);
    if (pathname === '/v1/mounts') return Response.json(searchParams.get('session') === 'session-5' ? [] : [mount]);
    assert.match(pathname, /^\/v1\/sessions\/session-[0-4]\/acquisitions$/);
    return Response.json([
      ...rows,
      { id: 'acq-ref', instrument_label: 'Reference_1', reference_material_name: 'Synthetic reference' },
      { id: 'acq-blank', instrument_label: '  ', reference_material_name: '' },
    ]);
  });
  const [acquisitions, labels] = await Promise.all([
    mountAcquisitions(base, 'test-token', mount.id),
    mountAcquisitionLabels(`${base}/`, 'test-token', mount.id.toUpperCase()),
  ]);
  assert.equal(acquisitions.length, 12500);
  assert.deepEqual(acquisitions[0], { id: 'acq-0', label: 'S1_0' });
  assert.deepEqual(labels, rows.map(a => a.instrument_label).sort());
  assert.equal(fetch.mock.callCount(), 12); // sessions + six mount lists + five acquisition lists
  assert.ok(peak <= 4);
  assert.strictEqual(await mountAcquisitions(base, 'test-token', mount.id), acquisitions);
  assert.deepEqual(await mountAcquisitionLabels(base, 'test-token', mount.id), labels);
  assert.equal(fetch.mock.callCount(), 12);
  clearMountAcquisitionLabels(`${base}/`, mount.id.toUpperCase());
  await mountAcquisitions(base, 'test-token', mount.id);
  assert.equal(fetch.mock.callCount(), 24);
});

test('failed mount acquisition requests are evicted from the shared cache', async t => {
  const base = 'https://upbdb.test';
  clearMountAcquisitionLabels(base, mount.id);
  t.after(() => clearMountAcquisitionLabels(base, mount.id));
  t.mock.method(console, 'warn', () => {});
  let fail = true;
  const fetch = t.mock.method(globalThis, 'fetch', async () => fail ? Response.json({ detail: 'Synthetic failure' }, { status: 500 }) : Response.json([]));
  await assert.rejects(mountAcquisitions(base, 'test-token', mount.id), e => e instanceof ApiError && e.status === 500);
  fail = false;
  assert.deepEqual(await mountAcquisitionLabels(base, 'test-token', mount.id), []);
  assert.equal(fetch.mock.callCount(), 2);
});

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
