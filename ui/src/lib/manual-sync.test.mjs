import assert from 'node:assert/strict';
import test from 'node:test';
import { startSyncPolling } from './sync-scheduling.ts';
import { DEFAULT_LINK, LINK_KEY, loadLink, saveLink } from './upbdb.ts';
import { manualSyncText } from './sync-status.ts';

test('manual mode creates no periodic work over five minutes; opt-in restores the interval', t => {
  t.mock.timers.enable({ apis: ['setInterval'] });
  let checks = 0;
  const manual = startSyncPolling(false, 120_000, () => checks++);
  t.mock.timers.tick(300_000);
  assert.equal(checks, 0);
  manual();
  const automatic = startSyncPolling(true, 120_000, () => checks++);
  t.mock.timers.tick(300_000);
  assert.equal(checks, 2);
  automatic();
  t.mock.timers.tick(300_000);
  assert.equal(checks, 2, 'turning automatic sync off cancels the poll');
});

test('legacy automatic preferences migrate once, keeping unrelated settings and later opt-in', t => {
  const previousLocal = globalThis.localStorage, previousSession = globalThis.sessionStorage;
  const values = new Map([[LINK_KEY, JSON.stringify({ syncAutomatically: true, syncOnSave: false, syncPollSeconds: 240, server: 'https://synthetic.test', quality: 97 })]]);
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  globalThis.localStorage = storage; globalThis.sessionStorage = storage;
  t.after(() => { globalThis.localStorage = previousLocal; globalThis.sessionStorage = previousSession; });
  assert.equal(DEFAULT_LINK.syncAutomatically, false);
  const migrated = loadLink();
  assert.equal(migrated.syncAutomatically, false);
  assert.equal(migrated.syncModeVersion, 1);
  assert.equal(migrated.syncOnSave, false);
  assert.equal(migrated.syncPollSeconds, 240);
  assert.equal(migrated.quality, 97);
  assert.equal(migrated.server, 'https://synthetic.test');
  assert.equal(loadLink().syncAutomatically, false);
  saveLink({ ...migrated, syncAutomatically: true });
  assert.equal(loadLink().syncAutomatically, true, 'migration must not reset a deliberate opt-in');
  values.delete(LINK_KEY);
  assert.equal(loadLink().syncAutomatically, false, 'fresh installs are manual too');
  values.set(LINK_KEY, JSON.stringify({ syncAutomatically: true }));
  values.set('grainatlas.upbdb.token', 'synthetic-session-token');
  storage.setItem = () => { throw new Error('Synthetic storage quota'); };
  const quota = loadLink();
  assert.equal(quota.syncAutomatically, false);
  assert.equal(quota.token, 'synthetic-session-token', 'migration write failure must not hide a valid session');
});

test('manual chip counts elements, distinguishes unknown/image work, and keeps attention states', () => {
  assert.equal(manualSyncText('Saved', { spots: 1, grains: 0, labels: 0 }, true), '1 change not synced');
  assert.equal(manualSyncText('Saved', { spots: 2, grains: 3, labels: 1 }, true), '6 changes not synced');
  assert.equal(manualSyncText('Saved', { spots: 0, grains: 0, labels: 0 }, true), 'Not synced');
  assert.equal(manualSyncText('Offline, will sync later', { spots: 1, grains: 0, labels: 0 }, true), '1 change not synced');
  assert.equal(manualSyncText('Offline, will sync later', { spots: 0, grains: 0, labels: 0 }, false), 'Offline');
  for (const status of ['Needs your attention', 'Syncing…', 'Up to date']) {
    assert.equal(manualSyncText(status, { spots: 0, grains: 0, labels: 0 }, status !== 'Up to date'), status);
  }
});
