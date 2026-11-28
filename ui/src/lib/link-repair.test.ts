import { describe, expect, it, vi } from 'vitest';
import { assignLink, keepHeldVersion, linkIssues, repairLinks, rememberLinkIssues } from './link-repair';
import { appliedPart, acknowledgedBase, diffElements, emptyElements, needsFirstSnapshot, syncChanges, syncIntentFailure, type Elements, type ReadElements, type SyncGrain, type SyncSpot } from './sync';
import { syncWireBody } from './sync-wire';
import { pendingElements } from './drafts';
import { linkUnlinkedSpots, relinkSpots } from './spots';
import type { Grain, Spot } from './core/types';

const uuid = 'synthetic-project';
const grain = (id = 'old', x = 0, area_id = 'area'): SyncGrain => ({ id, area_id, number: 1,
  polygon_um: [[x, 0], [x + 10, 0], [x + 10, 10], [x, 10]], centroid_um: [x + 5, 5], area_um2: 100 });
const spot = (grain_id: string | null = 'old'): SyncSpot => ({ id: 'measured', name: 'Synthetic measured spot', area_id: 'area', grain_id,
  x_um: 5, y_um: 5, x2_um: null, y2_um: null, kind: 'spot', diameter_um: 12, comment: 'preserve', acquisition_ids: ['synthetic-measurement'], acquisition_id: 'synthetic-link' });
const local = (grains = [grain('replacement')], spots = [spot()]): Elements => ({ grains, spots, labels: [] });
const database = (grains = [grain()], spots = [spot()]): ReadElements => ({ grains: grains.map(g => ({ ...g, project_uuid: uuid })),
  spots: spots.map(s => ({ ...s, project_uuid: uuid })), labels: [] });
const reply = { revision: 2, previous_revision: 1, not_in_snapshot: { grains: [], spots: [], labels: [] } };

describe('same-identity spatial repair', () => {
  it('leaves untouched unassigned points quiet and surfaces changed or ambiguous candidates', () => {
    for (const grains of [[], [grain()]]) {
      const mine = local(grains, [spot(null)]), base = database(grains, [spot(null)]);
      expect(linkIssues(mine, base, uuid)).toEqual([]);
      expect(repairLinks(mine, base, uuid).elements.spots[0]).toBe(mine.spots[0]);
    }
    const mine = local([grain('one'), grain('two')], [spot(null)]);
    expect(linkIssues(mine, database(mine.grains, mine.spots), uuid)[0].reason).toContain('Several outlines');
    expect(linkIssues(local([], [spot(null)]), database([], [spot(null)]), uuid, new Set(['measured']))).toHaveLength(1);
  });
  it('retains unresolved edit evidence after a successful baseline advance without sending it', () => {
    const mine = local([], [spot(null)]);
    const pending = rememberLinkIssues(mine, linkIssues(mine, database(), uuid));
    expect(pending.spots[0]).toMatchObject({ id: 'measured', grain_id: null, grain_link_review: { action: 'review' } });
    const advanced = database([], [spot(null)]);
    expect(linkIssues(pending, advanced, uuid)).toHaveLength(1);
    expect(pendingElements(pending).spots[0].grain_link_review?.action).toBe('review');
    const wire = syncWireBody({ ...diffElements(database(), pending), project_uuid: uuid, mode: 'delta', base_revision: 1 });
    expect(wire.spots.upsert[0]).not.toHaveProperty('grain_link_review');
    expect(mine.spots[0]).not.toHaveProperty('grain_link_review');
  });
  it('repairs registered replacements and previous missing grains without moving or replacing the spot', () => {
    for (const old of ['old', null]) {
      const mine = local(undefined, [spot(old)]), before = JSON.stringify(mine);
      const result = repairLinks(mine, database(undefined, [spot(old)]), uuid);
      expect(result.repaired).toHaveLength(1);
      expect(result.elements.spots[0]).toMatchObject({ ...spot(old), grain_id: 'replacement', grain_number: 1 });
      expect(result.elements.spots[0].grain_link_review?.action).toBe('assign');
      expect(JSON.stringify(mine)).toBe(before);
      const delta = diffElements(database(undefined, [spot(old)]), result.elements);
      const wire = syncWireBody({ ...delta, project_uuid: uuid, mode: 'delta', base_revision: 1 });
      expect(wire.spots.upsert).toHaveLength(1);
      expect(wire.spots.upsert[0]).toMatchObject({ id: 'measured', grain_id: 'replacement', x_um: 5, y_um: 5 });
      expect(wire.spots.delete).toEqual([]);
      expect(wire.spots.upsert[0]).not.toHaveProperty('grain_link_review');
      expect(pendingElements(result.elements).spots[0].grain_link_review).toEqual(result.elements.spots[0].grain_link_review);
    }
  });
  it('leaves valid links alone despite overlapping replacements', () => {
    const s = spot();
    const mine = local([grain(), grain('overlap')], [s]);
    expect(linkIssues(mine, database(), uuid)).toEqual([]);
    expect(repairLinks(mine, database(), uuid).elements.spots[0]).toBe(s);
  });
  it('repairs an outline edit invalidating the old relationship using the unchanged spot centre', () => {
    const mine = local([grain('old', 50), grain('replacement')]);
    expect(repairLinks(mine, database(), uuid).elements.spots[0]).toMatchObject({ id: 'measured', x_um: 5, y_um: 5, grain_id: 'replacement' });
  });
  it('never guesses overlaps, boundary, no match, artifact or incompatible lineage', () => {
    const cases = [local([grain('one'), grain('two')]), local([grain('edge', 5)]), local([grain('far', 50)]),
      local([{ ...grain('artifact'), artifact: 'manual' }]), local([{ ...grain('child'), split_from: 'unrelated' }])];
    for (const mine of cases) {
      const issue = linkIssues(mine, database(), uuid)[0];
      expect(issue.automatic).toBe(false);
      expect(issue.reason.length).toBeGreaterThan(10);
      expect(repairLinks(mine, database(), uuid).elements.spots[0]).toBe(mine.spots[0]);
    }
    const issue = linkIssues(cases[0], database(), uuid)[0];
    expect(assignLink(issue.spot, issue.candidates[1].grain, 'Analyst selected.').grain_id).toBe('two');
  });
  it('rejects another area, foreign identity, moved position and removed spots', () => {
    expect(linkIssues(local([grain('other-area', 0, 'other')]), database(), uuid)[0].candidates).toEqual([]);
    const foreign = database(); foreign.spots[0].project_uuid = 'foreign';
    expect(repairLinks(local(), foreign, uuid).repaired).toEqual([]);
    const foreignGrain = database([grain('replacement')]); foreignGrain.grains[0].project_uuid = 'foreign';
    expect(linkIssues(local(), foreignGrain, uuid)[0].candidates).toEqual([]);
    const moved = local(undefined, [{ ...spot(), x_um: 6 }]);
    expect(linkIssues(moved, database(), uuid)[0].candidates).toEqual([]);
    expect(linkIssues(local(undefined, []), database(), uuid)).toEqual([]);
  });
  it('preserves detach and keep intent through repeated preparation and draft storage', () => {
    for (const action of ['detach', 'keep'] as const) {
      const s = { ...spot(null), grain_link_review: { action, grain_id: null, reason: 'Explicit choice.' } };
      const mine = pendingElements(local(undefined, [s]));
      expect(repairLinks(mine, database(), uuid).repaired).toEqual([]);
    }
    expect(repairLinks(local(undefined, [{ ...spot(null), detach_grain: true }]), database(), uuid).repaired).toEqual([]);
    expect(repairLinks(local(undefined, [spot(null)]), database(), uuid).repaired).toEqual([]);
  });
  it('local detection also refuses ambiguous and boundary inference', () => {
    const s = spot(null) as Spot;
    for (const grains of [[grain('one'), grain('two')], [grain('edge', 5)]]) {
      expect(linkUnlinkedSpots([s], 'area', grains as Grain[])[0]).toBe(s);
      expect(relinkSpots([s], 'area', grains as Grain[])[0]).toBe(s);
    }
  });
  it('searches available same-area geometry at scale without network requests', () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockImplementation(() => { throw Error('No geometry requests'); });
    try {
      const n = 3000;
      const grains = Array.from({ length: n }, (_, i) => grain(`new-${i}`, i * 20));
      const spots = grains.map((g, i) => ({ ...spot(null), id: `spot-${i}`, x_um: i * 20 + 5 }));
      const start = performance.now();
      const result = repairLinks(local(grains, spots), database([], spots), uuid);
      expect(result.repaired).toHaveLength(n);
      expect(fetch).not.toHaveBeenCalled();
      console.info(`Synthetic link search: ${n} spots/outlines, ${(performance.now() - start).toFixed(1)} ms, 0 requests`);
    } finally { fetch.mockRestore(); }
  });
});

describe('operation-specific acknowledgement and recovery', () => {
  it('keeps retired or absent parents without clearing active descendants or unrelated edits', () => {
    for (const present of [false, true]) {
      const child = { ...grain('child'), split_from: 'old' };
      const merged = { ...grain('merged'), merged_from: ['old', 'second'] };
      const remote = database([...(present ? [{ ...grain(), retired: true }] : []), child, merged], []);
      const mine = local([grain(), { ...child, number: 8 }, merged], []);
      mine.labels = [{ id: 'note', area_id: 'area', x_um: 1, y_um: 1, text: 'Independent edit' }];
      const result = keepHeldVersion(mine, remote, { kind: 'grain', id: 'old', op: 'upsert' }, uuid);
      expect(result.grains.map(g => g.id)).toEqual(['child', 'merged']);
      expect(result.grains[0]).toMatchObject({ split_from: 'old', number: 8 });
      expect(result.grains[1].merged_from).toEqual(['old', 'second']);
      expect(result.labels).toEqual(mine.labels);
      expect(mine.grains).toHaveLength(3);
    }
  });
  it('cancels only proposed lineage on restoration and retains authoritative history', () => {
    const historical = { ...grain('historical'), split_from: 'old' };
    const merged = { ...grain('merged'), merged_from: ['old', 'second'] };
    const remote = database([grain(), historical, merged], []);
    const mine = local([grain(), historical, merged, { ...grain('proposal'), split_from: 'old' },
      { ...grain('merge-proposal'), merged_from: ['old', 'second'] }], []);
    const result = keepHeldVersion(mine, remote, { kind: 'grain', id: 'old' }, uuid);
    expect(result.grains.find(g => g.id === 'historical')?.split_from).toBe('old');
    expect(result.grains.find(g => g.id === 'merged')?.merged_from).toEqual(['old', 'second']);
    expect(result.grains.find(g => g.id === 'proposal')?.split_from).toBeUndefined();
    expect(result.grains.find(g => g.id === 'merge-proposal')?.merged_from).toEqual(['second']);
  });
  it('verifies preserved history on saved bad deltas and snapshots, but detects ignored new lineage and fields', () => {
    for (const mode of ['delta', 'snapshot'] as const) {
      for (const lineage of [{ split_from: 'old' }, { merged_from: ['old', 'second'] }]) {
        const current = database([{ ...grain('child'), ...lineage }], []);
        const bad = local([{ ...grain('child'), number: 7, split_from: undefined, merged_from: [] }], []);
        const changes = syncChanges(mode, current, bad, current, uuid);
        const accepted = database([{ ...current.grains[0], id: 'child', split_from: 'split_from' in lineage ? lineage.split_from : undefined, number: 7 }], []);
        expect(syncIntentFailure(changes, bad, current, uuid, reply, accepted)).toBeNull();
        expect(syncIntentFailure(changes, bad, current, uuid, reply, current)).toContain('stored fields differ');
        const proposed = local([{ ...bad.grains[0], ...('split_from' in lineage ? { split_from: 'new-parent' } : { merged_from: ['old', 'second', 'new-parent'] }) }], []);
        const submitted = syncChanges(mode, current, proposed, current, uuid);
        expect(syncIntentFailure(submitted, proposed, current, uuid, reply, accepted)).toContain('child: lineage differs');
      }
      const current = database([{ ...grain('child'), split_from: 'old' }], []);
      const bad = local([grain('child')], []);
      const changes = syncChanges(mode, current, bad, current, uuid);
      expect(syncIntentFailure(changes, bad, current, uuid, { ...reply, revision: 1 }, current)).toBeNull();
      const fresh = local([{ ...grain('child'), split_from: 'old' }], []);
      const withoutHistory = database([grain('child')], []);
      expect(syncIntentFailure(syncChanges(mode, withoutHistory, fresh, withoutHistory, uuid), fresh, withoutHistory, uuid, reply, withoutHistory)).toContain('lineage differs');
    }
  });
  it('uses a confirmed base for saved additions while preserving first and legacy-adoption snapshots', () => {
    const current = database([grain()], []);
    const pending = local([grain(), grain('new')], []);
    expect(needsFirstSnapshot(current, pending)).toBe(true);
    expect(needsFirstSnapshot(current, pending, current)).toBe(false);
    expect(needsFirstSnapshot(emptyElements(), pending, current)).toBe(true);
    const legacy: ReadElements = { ...current, grains: [{ ...grain('legacy'), id: null, label: 'area/1' }] };
    expect(needsFirstSnapshot(legacy, pending, current)).toBe(true);
  });
  it('bounds verification reasons without including stored field values', () => {
    const current = database([], []);
    const mine = local(Array.from({ length: 8 }, (_, i) => ({ ...grain(`item-${i}-private-suffix`), sample: 'Never print this value' })), []);
    const message = syncIntentFailure(diffElements(current, mine), mine, current, uuid, reply, current)!;
    expect(message).toContain('item-0-p: missing from database');
    expect(message).toContain('5 more');
    expect(message).not.toContain('item-3');
    expect(message).not.toContain('private-suffix');
    expect(message).not.toContain('Never print');
  });
  it('verifies sent/applied elements independently of held and unrelated differences', () => {
    const current = database([grain(), grain('unrelated', 50)]);
    const mine = local([grain(), { ...grain('unrelated', 50), number: 9 }], [{ ...spot(), comment: 'sent' }]);
    const changes = diffElements(current, mine); changes.grains.upsert = [];
    const accepted = database(undefined, [{ ...spot(), comment: 'sent' }]);
    expect(syncIntentFailure(changes, mine, current, uuid, reply, accepted)).toBeNull();
    accepted.spots[0].comment = 'lost';
    expect(syncIntentFailure(changes, mine, current, uuid, reply, accepted)).toContain('did not save all spots');
    expect(syncIntentFailure(changes, mine, current, uuid, { ...reply, revision: 1 }, accepted)).toContain('did not advance');
  });
  it('does not acknowledge a held grain or require an all-held request to advance', () => {
    const current = database(); const mine = local([{ ...grain(), number: 9 }], [spot()]);
    const changes = diffElements(current, mine);
    const applied = appliedPart(changes, [{ kind: 'grain', id: 'old', op: 'upsert' }]);
    const base = acknowledgedBase(current, applied, mine);
    expect(applied.grains.upsert).toEqual([]);
    expect(base.grains[0].number).toBe(1);
    expect(syncIntentFailure(applied, base, current, uuid, { ...reply, revision: 1 }, current)).toBeNull();
  });
  it('does not let preparation-time measured deletion protection mask an ignored sent delete', () => {
    const current = database(); const mine = local([grain()], []);
    const changes = diffElements(current, mine); changes.spots.delete = ['measured'];
    expect(syncIntentFailure(changes, mine, current, uuid, reply, current)).toContain('did not save all spots');
  });
  it('keeps one rejected operation, restoring dependencies and preserving unrelated draft edits', () => {
    const mine = local(); mine.spots[0] = { ...mine.spots[0], grain_id: 'replacement', comment: 'local unrelated comment' };
    mine.labels = [{ id: 'note', area_id: 'area', x_um: 1, y_um: 1, text: 'Keep this edit' }];
    const result = keepHeldVersion(mine, database(), { kind: 'grain', id: 'replacement', op: 'upsert' }, uuid);
    expect(result.grains.map(g => g.id)).toEqual(['old']);
    expect(result.spots[0]).toMatchObject({ id: 'measured', grain_id: 'old', comment: 'local unrelated comment', acquisition_ids: ['synthetic-measurement'] });
    expect(result.labels).toEqual(mine.labels);
  });
  it('restores original spot and grain and cancels dependent retirement without replacing the spot identity', () => {
    const mine = local([{ ...grain('child'), split_from: 'old' }], [{ ...spot(), grain_id: 'child' }]);
    const result = keepHeldVersion(mine, database(), { kind: 'spot', id: 'measured' }, uuid);
    expect(result.spots[0]).toMatchObject({ ...spot(), grain_link_review: { action: 'keep', grain_id: 'old' } });
    expect(result.grains.some(g => g.id === 'old')).toBe(true);
    expect(result.grains.find(g => g.id === 'child')?.split_from).toBeUndefined();
    const foreign = database(); foreign.spots[0].project_uuid = 'foreign';
    expect(() => keepHeldVersion(mine, foreign, { kind: 'spot', id: 'measured' }, uuid)).toThrow('owning project');
    expect(keepHeldVersion(mine, emptyElements(), { kind: 'grain', id: 'child' }, uuid).spots[0].id).toBe('measured');
  });
  it('preserves independent valid grain edits when keeping a spot operation', () => {
    const edited = { ...grain(), number: 7, sample: 'Synthetic independent edit' };
    const mine = local([edited], [{ ...spot(), grain_id: null, detach_grain: true, comment: 'Rejected comment' }]);
    const result = keepHeldVersion(mine, database(), { kind: 'spot', id: 'measured' }, uuid);
    expect(result.grains[0]).toEqual(edited);
    expect(result.spots[0].comment).toBe('preserve');
    expect(result.spots[0].detach_grain).toBeUndefined();
    expect(() => keepHeldVersion(mine, database([], []), { kind: 'spot', id: 'measured' }, uuid)).toThrow('measured spot');
  });
});
