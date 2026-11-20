import { describe, expect, it, vi } from 'vitest';
import { buildPickListRows, renderPickList, type PickListOptions } from './tims-picklist';
import type { Grain, MosaicJson, Project, Spot, ToneParams } from './core/types';
import type { BatchDetail, CapsuleOut, FractionOut } from './tims';
import type { OutboxEntry } from './tims-outbox';
const set: CapsuleOut = { id: 'K', lab_id: 'lab', name: 'Krogh', code: 'K', positions: 15, capsule_labels: Array.from({ length: 15 }, (_, i) => `K${i + 1}`) };
const second: CapsuleOut = { ...set, id: 'B', name: 'Becquerel', code: 'B', capsule_labels: set.capsule_labels.map(s => s.replace('K', 'B')) };
const batch: BatchDetail = { id: 'batch-1', capsule_set_id: set.id, started_on: '2026-01-01', operator_name: '', notes: '', capsule_map: {} };
const grain = (id: string, n: number) => ({ id, number: n, label: `MOUNT_A/${n}`, polygon_um: [[20, 20], [40, 20], [40, 60], [20, 60]], bbox_um: [20, 20, 40, 60], centroid_um: [30, 40] }) as Grain;
const f = (id: string, n: number, grains = ['A']): FractionOut => ({ id, name: `z${n}`, sample: { id: 'sample', name: 'SAMPLE1' }, mineral: 'zircon', batch_id: batch.id, capsule_number: n, notes: '', status: 'picked', sources: grains.map((id, i) => ({ id: `source-${i}`, atlas_grain_id: id })) });
const options: PickListOptions = { set: '', batch: '', paper: 'a4', title: 'MOUNT_A pick list', cropMargin: 30, cropSize: 35 };
function input() {
  return { fractions: [f('f3', 3), f('f1', 1), f('f2', 2)], batches: [batch], sets: [set], grains: { area: [grain('A', 1), grain('B', 2)] }, spots: [{ id: 'spot', area_id: 'area', grain_id: 'A', name: 'SAMPLE1 195', x_um: 30, y_um: 40, kind: 'spot', diameter_um: 15 }] as Spot[], areas: [{ id: 'area', name: 'AREA_A' }], outbox: [] as OutboxEntry[], filter: { set: '', batch: '' } };
}
describe('pick-list rows', () => {
  it('sorts by server capsule-set order then capsule number, leaving sample-only and batch-less rows last', () => {
    const data = input(); data.sets = [second, set]; data.batches.push({ ...batch, id: 'batch-2', capsule_set_id: second.id });
    data.fractions.push({ ...f('fB', 9), batch_id: 'batch-2' }, { ...f('sample-only', 4, []), batch_id: null, capsule_number: null }, { ...f('no-batch', 5), batch_id: null, capsule_number: null });
    expect(buildPickListRows(data).map(r => r.id)).toEqual(['fB', 'f1', 'f2', 'f3', 'sample-only', 'no-batch']);
  });
  it('keeps several grains in one fraction row and draws every LA spot on each grain', () => {
    const data = input(); data.fractions = [f('multi', 1, ['A', 'B', 'A'])];
    const rows = buildPickListRows(data);
    expect(rows).toHaveLength(1); expect(rows[0].grains.map(g => g.id)).toEqual(['A', 'B']);
    expect(rows[0].grains[0].spots[0].name).toBe('SAMPLE1 195'); expect(rows[0].grains[0].sources).toHaveLength(2);
  });
  it('marks pending and waiting plucks, including an unsent batch', () => {
    const data = input(); data.outbox = [{ id: 'out', op: 'fraction.create', target: 'f1', state: 'waiting' } as OutboxEntry];
    expect(buildPickListRows(data).filter(r => r.pending).map(r => r.id)).toEqual(['f1']);
    data.outbox = [{ id: 'out', op: 'batch.create', target: batch.id, state: 'pending' } as OutboxEntry];
    expect(buildPickListRows(data).every(r => r.pending)).toBe(true);
  });
  it('filters batches and sets and leaves retired fractions and sources out', () => {
    const data = input(); data.fractions.push({ ...f('other', 4), batch_id: 'other-batch' });
    data.fractions[0].retired_at = '2026-01-02'; data.fractions[1].sources[0].retired_at = '2026-01-02';
    data.filter.batch = batch.id;
    expect(buildPickListRows(data).map(r => r.id)).toEqual(['f2', 'f1']);
    data.filter.set = 'missing'; expect(buildPickListRows(data)).toEqual([]);
  });
});
describe('pick-list print pages', () => {
  const project = { package: { name: 'MOUNT_A' }, areas: [{ id: 'area', params: { tone: { gamma: 1.3 } } }] } as unknown as Project;
  const mosaic = { w: 200, h: 200, um_per_px: 1 } as MosaicJson;
  it('prints bench-sized type and keeps K15 on one line even with the largest crop on letter paper', async () => {
    const data = input(); data.fractions = [f('large', 15)];
    const r = await renderPickList({ core: { pyramidTile: vi.fn(), toneLut: vi.fn() }, cropImage: vi.fn(async () => 'data:image/png;base64,'), project, rows: buildPickListRows(data), options: { ...options, paper: 'letter', cropSize: 60 }, mosaics: { area: mosaic }, tone: {}, mount: 'MOUNT_A' });
    const svg = r.pages[0];
    expect(svg).toMatch(/font-size="7" font-weight="bold"><tspan[^>]*>K15<\/tspan><\/text>/);
    expect(svg).toMatch(/font-size="4" font-weight="bold"><tspan[^>]*>z15<\/tspan><\/text>/);
    expect(svg).toMatch(/font-size="3.2"><tspan[^>]*>SAMPLE1<\/tspan>/);
    expect(svg).toMatch(/font-size="2.8"><tspan[^>]*>Capsule<\/tspan>/);
    expect(svg).toMatch(/font-size="5"><tspan[^>]*>MOUNT_A pick list<\/tspan>/);
    expect(svg).toMatch(/font-size="3.2"><tspan[^>]*>MOUNT_A · Krogh · 2026-01-01<\/tspan>/);
  });
  it('sizes rows to the tallest wrapped cell or crop stack and moves overflowing rows to the next page', async () => {
    const data = input();
    data.sets = [{ ...set, capsule_labels: ['K'.repeat(20), ...set.capsule_labels.slice(1)] }];
    data.fractions = [f('capsule', 1), { ...f('fraction', 2), name: 'z'.repeat(60) }, { ...f('notes', 3), notes: 'n'.repeat(130) }, f('crops', 4, ['A', 'B']), f('next', 5)];
    const r = await renderPickList({ core: { pyramidTile: vi.fn(), toneLut: vi.fn() }, cropImage: vi.fn(async () => 'data:image/png;base64,'), project, rows: buildPickListRows(data), options, mosaics: { area: mosaic }, tone: {}, mount: 'MOUNT_A' });
    expect(r.pages).toHaveLength(2);
    expect(r.pages[0].match(/data-pick-row=/g)).toHaveLength(4);
    expect(r.pages[1]).toContain('data-pick-row="next"');
    const tops = [...r.pages[0].matchAll(/<path d="M8,([\d.]+)H/g)].map(match => Number(match[1]));
    // Five capsule lines at 9.1 mm, ten fraction lines at 5.2 mm,
    // and ten notes lines at 4.16 mm, each with 4 mm row padding.
    expect(tops[1] - tops[0]).toBeCloseTo(49.5);
    expect(tops[2] - tops[1]).toBeCloseTo(56);
    expect(tops[3] - tops[2]).toBeCloseTo(45.6);
    expect(tops[3] + 74).toBeLessThanOrEqual(297 - 8);
    expect(tops[3] + 74 + 39).toBeGreaterThan(297 - 8);
    expect(r.pages[0]).toContain('dy="9.1"');
    expect(r.pages[0]).toContain('dy="5.2"');
    expect(r.pages[0]).toContain('dy="4.16"');
    expect(r.pages[0]).toContain('>KKKK</tspan>');
    expect(r.pages[0]).toContain('>zzzzzz</tspan>');
    expect(r.pages[0]).toContain('>nnnnnnnnnnnnn</tspan>');
  });
  it('uses canvas crops with the displayed tone, outline and LA spots; escapes notes and adds page counts', async () => {
    const data = input(); data.fractions = [f('multi', 1, ['A', 'B'])]; data.fractions[0].notes = '<script>&';
    data.outbox = [{ target: 'multi' } as OutboxEntry];
    const cropImage = vi.fn(async () => 'data:image/png;base64,AQID'), tone = { area: { gamma: 1.3 } } as unknown as Record<string, ToneParams>;
    const result = await renderPickList({ core: { pyramidTile: vi.fn(), toneLut: vi.fn() }, cropImage, project, rows: buildPickListRows(data), options, mosaics: { area: mosaic }, tone, mount: 'MOUNT_A' });
    expect(cropImage).toHaveBeenCalledTimes(2); expect(cropImage.mock.calls[0].slice(0, 4)).toEqual(['area', [0, 0, 80, 90], mosaic, tone.area]);
    expect(result.pages).toHaveLength(1); expect(result.items).toBe(1);
    expect(result.pages[0]).toContain('1 / 1'); expect(result.pages[0]).toContain('not yet'); expect(result.pages[0]).toContain('&lt;script&gt;&amp;');
    expect(result.pages[0]).toContain('<polygon'); expect(result.pages[0]).toContain('<circle'); expect(result.pages[0].match(/data-pick-row=/g)).toHaveLength(1);
  });
  it('starts a page for each batch and uses the matching header', async () => {
    const data = input(); data.fractions.push({ ...f('other', 4), batch_id: 'batch-2' }); data.batches.push({ ...batch, id: 'batch-2', started_on: '2026-02-01' });
    const r = await renderPickList({ core: { pyramidTile: vi.fn(), toneLut: vi.fn() }, cropImage: vi.fn(async () => 'data:image/png;base64,'), project, rows: buildPickListRows(data), options, mosaics: { area: mosaic }, tone: {}, mount: 'MOUNT_A' });
    expect(r.pages).toHaveLength(2); expect(r.pages[0]).toContain('1 / 2'); expect(r.pages[1]).toContain('2 / 2'); expect(r.pages[1]).toContain('2026-02-01');
  });
  it('builds 3000 local rows without network calls and keeps 15-row page composition under one second', async () => {
    const data = input(); data.fractions = Array.from({ length: 3000 }, (_, i) => f(`f${i}`, i + 1));
    const start = performance.now(), rows = buildPickListRows(data);
    expect(rows).toHaveLength(3000);
    const core = { pyramidTile: vi.fn(), toneLut: vi.fn() }, cropImage = vi.fn(async () => 'data:image/png;base64,');
    const r = await renderPickList({ core, cropImage, project, rows: rows.slice(0, 15), options, mosaics: { area: mosaic }, tone: {}, mount: 'MOUNT_A' });
    expect(r.items).toBe(15); expect(cropImage).toHaveBeenCalledTimes(1); expect(core.pyramidTile).not.toHaveBeenCalled();
    expect(performance.now() - start).toBeLessThan(1000);
  });
});
