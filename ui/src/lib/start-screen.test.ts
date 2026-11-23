import { describe, expect, it } from 'vitest';
import { startLayout, startEntries, localMatch, entryStatus, relativeTime } from './mount-browser';
import { DEFAULT_LAYOUT, loadLayout, saveLayout } from './layout-prefs';
import { catchUpAction } from './start-catch-up';
import { bundleKey, cacheBundle, readBundle, bundleProjects, BUNDLE_BYTES, type OfflineBundle } from './offline-bundles';
import type { Draft } from './drafts';
import type { MountOut } from './upbdb';

const server = 'https://database.example.test';
const now = Date.parse('2026-11-23T12:00:00Z');
const at = new Date(now - 300_000).toISOString();
const recent = [{ id: 'project-a', name: 'Invented local', when: at, areas: 1, spots: 1 }];
const mount = { id: 'mount-a', label: 'Invented mount', atlas: { project_id: 'project-a' } } as MountOut;
const draft: Draft = { uuid: 'project-a', name: 'Invented local', server, mount: 'mount-a', updated_at: at, base: null, base_revision: 2, changes: { spots: 2, grains: 1, labels: 0 }, synced_at: at };

describe('start-screen decisions', () => {
  it('defaults and validates the remembered view', () => {
    expect(startLayout({}).startView).toBe('database');
    expect(startLayout({ startView: 'invalid' }).startView).toBe('database');
    expect(startLayout({ startView: 'local' }).startView).toBe('local');
    const s = storage(); saveLayout(s, { ...DEFAULT_LAYOUT, startView: 'local' });
    expect(loadLayout(s).startView).toBe('local');
  });
  it('merges visible linked copies, including the separate copy offered on a tile', () => {
    expect(startEntries([mount], recent, [draft], server)).toEqual({ tiles: [{ mount, match: { recent: recent[0], draft } }], localOnly: [] });
    const other = { ...recent[0], id: 'other-project' }, separate = { ...draft, uuid: other.id };
    expect(startEntries([mount], [other], [separate], server).localOnly).toEqual([]);
    const invisible = { ...recent[0], id: 'not-visible' };
    expect(startEntries([mount], [...recent, invisible], [draft, { ...draft, uuid: invisible.id, mount: 'invisible-mount' }], server).localOnly).toEqual([invisible]);
    expect(startEntries([mount], recent, [draft], null).localOnly).toEqual(recent);
    expect(startEntries([], recent, [draft], server).localOnly).toEqual(recent);
  });
  it('matches the same way as localMatch, case-insensitively, and preserves extra copies', () => {
    const extra = { ...recent[0], name: 'Invented second', id: 'other-project' };
    const aliases = [{ ...draft, uuid: 'other-project', linked_uuid: 'PROJECT-A' }];
    const result = startEntries([mount], [extra], aliases, server);
    expect(result.tiles[0].match).toEqual(localMatch(mount, [extra], aliases, server));
    expect(startEntries([mount], [...recent, extra], [draft, { ...draft, uuid: extra.id }], server).localOnly).toEqual([extra]);
  });
  it('indexes 5,000 rows and recent entries in under 50 ms', () => {
    const rows = Array.from({ length: 5000 }, (_, i) => ({ ...mount, id: `mount-${i}`, atlas: { project_id: `project-${i}`, thumbnail: null } }));
    const entries = rows.map((m, i) => ({ ...recent[0], id: m.atlas.project_id, name: `Invented ${i}` }));
    const drafts = rows.map(m => ({ ...draft, uuid: m.atlas.project_id, mount: m.id }));
    const started = performance.now();
    const result = startEntries(rows, entries, drafts, server);
    const ms = performance.now() - started;
    expect(result.tiles).toHaveLength(5000); expect(result.localOnly).toEqual([]);
    expect(ms).toBeLessThan(50); console.info(`Start entries: 5,000 tiles + recent + drafts, ${ms.toFixed(1)} ms; no requests.`);
  });
  it('uses short relative times and dates', () => {
    for (const [age, expected] of [[0, 'just now'], [300_000, '5 min ago'], [7_200_000, '2 h ago'], [259_200_000, '3 d ago']] as const)
      expect(relativeTime(new Date(now - age).toISOString(), now)).toBe(expected);
    expect(relativeTime('2026-01-01', now)).toBe(new Date('2026-01-01').toLocaleDateString());
    expect(relativeTime('invalid', now)).toBe('unknown time');
  });
  it('describes database, local work and freshness on separate short lines', () => {
    expect(entryStatus(mount, null)).toEqual(['Database only']);
    expect(entryStatus(mount, { draft }, 3, { now })).toEqual(['Database · local copy synced 5 min ago', '3 changes not yet uploaded', 'Newer in database']);
    const clean = { ...draft, synced_at: undefined, checked_at: at, changes: null, pending: null };
    expect(entryStatus(mount, { draft: clean }, 2, { now })).toEqual(['Database · local copy synced 5 min ago']);
    expect(entryStatus(mount, { recent: recent[0] })).toEqual(['Database · local copy not yet synced']);
    expect(entryStatus(mount, { draft: { ...draft, uuid: 'other' } })).toEqual(['Database only']);
    expect(entryStatus(mount, { draft: { ...clean, checked_at: undefined, incomplete: { reason: 'Invented incomplete', omissions: { spots: [], grains: [], labels: [] } } } })).toEqual(['Database · local copy not yet synced', 'Local changes']);
  });
  it('distinguishes local, cached read-only and unavailable offline copies without a timestamp', () => {
    const offline = { offline: true, now };
    expect(entryStatus(mount, { draft }, 3, offline)).toEqual(['Local copy on this computer', '3 changes not yet uploaded']);
    expect(entryStatus(mount, { recent: recent[0] }, 3, offline)).toEqual(['Local copy on this computer']);
    expect(entryStatus(mount, null, undefined, { ...offline, hasOffline: true })).toEqual(['Offline copy available (read-only)']);
    expect(entryStatus(mount, null, undefined, offline)).toEqual(['Not on this computer']);
    const other = { draft: { ...draft, uuid: 'other-project' } };
    expect(entryStatus(mount, other, 3, offline)).toEqual(['Not on this computer']);
    expect(entryStatus(mount, other, 3, { ...offline, hasOffline: true })).toEqual(['Offline copy available (read-only)']);
  });
});

describe('catch-up decision', () => {
  const clean = { state: 'server-newer' as const, localChanges: false };
  it('updates only a clean newer copy', () => {
    expect(catchUpAction(clean)).toBe('update');
    expect(catchUpAction({ ...clean, localChanges: true })).toBe('none');
    expect(catchUpAction({ ...clean, localChanges: true, newer: true, automatic: false })).toBe('review');
    expect(catchUpAction({ state: 'both-changed', localChanges: true, automatic: true })).toBe('review');
    expect(catchUpAction({ state: 'local-changes', localChanges: true, newer: true, automatic: false })).toBe('review');
    expect(catchUpAction({ state: 'local-changes', localChanges: true, newer: true, automatic: true })).toBe('none');
  });
  it('leaves every attention flow and all other states alone', () => {
    for (const flag of ['staleOpen', 'review', 'confirmation', 'imageDecision', 'intentFailure', 'held', 'linkConflict']) {
      expect(catchUpAction({ ...clean, [flag]: true })).toBe('none');
      expect(catchUpAction({ ...clean, state: 'both-changed', [flag]: true })).toBe('none');
    }
    for (const state of ['checking', 'unregistered', 'in-sync', 'local-changes', 'offline'] as const)
      expect(catchUpAction({ state, localChanges: false })).toBe('none');
  });
});

function storage() {
  const map = new Map<string, string>();
  return { map, getItem: (key: string) => map.get(key) ?? null, setItem: (key: string, value: string) => { map.set(key, value); }, removeItem: (key: string) => { map.delete(key); } };
}
function record(id: string, padding = 0): OfflineBundle {
  return { atlas: { project_id: id } as OfflineBundle['atlas'], project: { uuid: id, name: 'x'.repeat(padding) } as OfflineBundle['project'], elements: null, revision: 2, at };
}
describe('bounded account bundle cache', () => {
  it('keeps 20 bundles in LRU order, touches reads, replaces entries, isolates accounts/datasets', async () => {
    const s = storage();
    for (let i = 0; i < 20; i++) await cacheBundle(s, 'dataset', 'a', record(`project-${i}`));
    await readBundle(s, 'dataset', 'a', 'PROJECT-0');
    await cacheBundle(s, 'dataset', 'a', record('project-20'));
    expect(await readBundle(s, 'dataset', 'a', 'project-1')).toBeNull();
    expect(s.map.has(bundleKey('dataset', 'a', 'project-1'))).toBe(false);
    expect(await readBundle(s, 'dataset', 'a', 'project-0')).toEqual(record('project-0'));
    await cacheBundle(s, 'dataset', 'a', { ...record('project-0'), revision: 3 });
    expect((await readBundle(s, 'dataset', 'a', 'project-0'))?.revision).toBe(3);
    expect(await bundleProjects(s, 'dataset', 'a')).toHaveLength(20);
    expect(await readBundle(s, 'dataset', 'b', 'project-0')).toBeNull();
    expect(await readBundle(s, 'other', 'a', 'project-0')).toBeNull();
    expect(bundleKey('dataset', 'a', 'project-0')).toBe('mirror/dataset/a/bundles/project-0.json');
  });
  it('evicts actual records at 50 MB and skips a single bundle above 25 MB', async () => {
    const s = storage();
    for (let i = 0; i < 4; i++) await cacheBundle(s, 'dataset', 'a', record(`large-${i}`, 16 * 1024 * 1024));
    expect(await bundleProjects(s, 'dataset', 'a')).toEqual(['large-1', 'large-2', 'large-3']);
    expect(s.map.has(bundleKey('dataset', 'a', 'large-0'))).toBe(false);
    expect([...s.map.values()].reduce((n, value) => n + new TextEncoder().encode(value).length, 0)).toBeLessThan(BUNDLE_BYTES);
    expect(await cacheBundle(s, 'dataset', 'a', record('oversize', 26 * 1024 * 1024))).toBe(false);
    expect(await bundleProjects(s, 'dataset', 'a')).toHaveLength(3);
    expect(s.map.has(bundleKey('dataset', 'a', 'oversize'))).toBe(false);
  });
  it('removes an untracked record if writing the LRU index fails', async () => {
    const s = storage();
    const broken = { ...s, setItem(key: string, value: string) { if (key.endsWith('/index.json')) throw new Error('Synthetic quota'); s.setItem(key, value); } };
    await expect(cacheBundle(broken, 'dataset', 'a', record('project-a'))).rejects.toThrow('Synthetic quota');
    expect(s.map.has(bundleKey('dataset', 'a', 'project-a'))).toBe(false);
  });
  it('serializes concurrent writes without losing the index and tolerates damaged records', async () => {
    const s = storage();
    await Promise.all(Array.from({ length: 25 }, (_, i) => cacheBundle(s, 'dataset', 'a', record(`project-${i}`))));
    expect(await bundleProjects(s, 'dataset', 'a')).toHaveLength(20);
    s.setItem(bundleKey('dataset', 'a', 'project-24'), '{');
    expect(await readBundle(s, 'dataset', 'a', 'project-24')).toBeNull();
  });
});
