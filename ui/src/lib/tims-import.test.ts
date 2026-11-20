import { afterEach, describe, expect, it, vi } from 'vitest';
import { importable, pickNotesRequest, pickNotesRows, PickNotesIdentity, PickNotesRefused, type PickNotesPlan } from './tims-import';
import { TimsApi } from './tims';
const text = '### 11/14/2026\nSAMPLE1\nKrogh\nK1-z1:S 195\nK2-z2:S 999';
const plan = (): PickNotesPlan => ({ lab_id: 'lab', first_line: 1, last_line: 5,
  summary: { ok: 1, refused: 1, skipped: 0, ignored: 3 }, batches: [{ set_id: 'K', started_on: '2026-11-14', action: 'create', id: null }],
  lines: [{ number: 1, text: '### 11/14/2026', kind: 'date', status: 'ignored', code: '', message: '', parsed: {}, resolved: {} },
    { number: 4, text: 'K1-z1:S 195', kind: 'pick', status: 'ok', code: '', message: '', parsed: { name: 'z1' }, resolved: { sample: { id: 'sample', label: 'SAMPLE1' }, mount: { id: 'mount', label: 'MOUNT_A' }, spot: { id: 'spot', label: 'SAMPLE1 195' }, grain: { id: 'grain', label: 'MOUNT_A/1' }, capsule_label: 'K1' } },
    { number: 5, text: 'K2-z2:S 999', kind: 'pick', status: 'refused', code: 'spot_not_found', message: 'No matching spot', parsed: { name: 'z2' }, resolved: {} }],
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });
describe('pick-note request and table', () => {
  it('keeps exact text and includes only provided optional fields', () => {
    expect(pickNotesRequest({ text, lab: 'lab' })).toEqual({ text, lab_id: 'lab' });
    expect(pickNotesRequest({ text, lab: 'lab', first: 2, last: 4, filename: 'notes.md', started: '2026-11-01' })).toEqual({ text, lab_id: 'lab', first_line: 2, last_line: 4, filename: 'notes.md', started_on: '2026-11-01' });
  });
  it('refuses empty input, missing lab and invalid line bounds', () => {
    for (const input of [{ text: '', lab: 'lab' }, { text, lab: '' }, { text, lab: 'lab', first: 0 }, { text, lab: 'lab', last: 6 }, { text, lab: 'lab', first: 4, last: 2 }, { text, lab: 'lab', last: 1.5 }]) expect(() => pickNotesRequest(input)).toThrow();
  });
  it('maps labels, fraction, refusal messages and the open mount; collapses ignored lines', () => {
    const rows = pickNotesRows(plan(), 'mount'); expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ number: 4, result: 'ok', sample: 'SAMPLE1', mount: 'MOUNT_A', spot: 'SAMPLE1 195', grain: 'MOUNT_A/1', capsule: 'K1', fraction: 'z1', openMount: true });
    expect(rows[1].result).toBe('refused: No matching spot'); expect(rows[1].openMount).toBe(false);
    expect(pickNotesRows(plan(), null, true)).toHaveLength(3);
    const p = plan(); p.lines[1].status = 'skipped'; expect(pickNotesRows(p, null)[0].result).toBe('skipped: already imported');
  });
  it('requires at least one ok pick and no refused line in the selected range', () => {
    const p = plan(); expect(importable(p)).toBe(false); p.last_line = 4; expect(importable(p)).toBe(true);
    p.lines[1].status = 'skipped'; expect(importable(p)).toBe(false);
  });
  it('keeps the idempotency key through retries and changes it only for a changed request', () => {
    const id = new PickNotesIdentity(), body = pickNotesRequest({ text, lab: 'lab' }), first = id.forRequest(body);
    expect(id.forRequest(structuredClone(body)).key).toBe(first.key); first.keyChanged('replacement');
    expect(id.forRequest(body).key).toBe('replacement'); expect(id.forRequest({ ...body, last_line: 4 }).key).not.toBe('replacement');
  });
});
describe('pick-note transport', () => {
  it('sends one preview request for thousands of lines, with auth; maps rows without further requests', async () => {
    const p = plan(); p.lines = Array.from({ length: 3000 }, (_, i) => ({ ...p.lines[1], number: i + 1 })); p.last_line = 3000;
    const fetcher = vi.fn(async (_url, init: RequestInit) => { expect(new Headers(init.headers).get('Authorization')).toBe('Bearer token'); return Response.json(p); }); vi.stubGlobal('fetch', fetcher);
    const client = new TimsApi('https://db.example.test', 'token'), body = { text: Array(3000).fill('K1-z1:S 195').join('\n'), lab_id: 'lab' };
    const before = performance.now(); const preview = await client.previewPickNotes(body);
    const after = performance.now(); expect(pickNotesRows(preview, 'mount')).toHaveLength(3000); expect(fetcher).toHaveBeenCalledTimes(1);
    console.info(JSON.stringify({ pickNotesLines: 3000, before: { requests: 0, ms: 0 }, after: { requests: 1, ms: Math.round(after - before) }, perRowRequests: 0 }));
  });
  it('exposes the refused plan on 409 and sends the same import key on retry', async () => {
    const p = plan(), keys: (string | null)[] = [];
    const fetcher = vi.fn(async (_url, init: RequestInit) => { keys.push(new Headers(init.headers).get('Idempotency-Key')); return Response.json({ code: 'pick_notes_refused', detail: 'Fix refused lines', plan: p }, { status: 409 }); }); vi.stubGlobal('fetch', fetcher);
    const client = new TimsApi('https://db.example.test', 'token'), body = pickNotesRequest({ text, lab: 'lab' }), id = new PickNotesIdentity();
    await expect(client.importPickNotes(body, id.forRequest(body))).rejects.toBeInstanceOf(PickNotesRefused);
    await expect(client.importPickNotes(body, id.forRequest(body))).rejects.toMatchObject({ plan: p, message: 'Database 409: Fix refused lines' });
    expect(keys[0]).toBeTruthy(); expect(keys[1]).toBe(keys[0]); expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it('returns the created counts on success', async () => {
    const reply = { plan: plan(), created: { fractions: ['f'], batches: ['b'], sources: ['s'], steps: [] } };
    vi.stubGlobal('fetch', vi.fn(async () => Response.json(reply)));
    expect(await new TimsApi('https://db.example.test', 'token').importPickNotes(pickNotesRequest({ text, lab: 'lab' }), { key: 'stable-key' })).toEqual(reply);
  });
});
