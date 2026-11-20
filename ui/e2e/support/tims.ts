import type { Page, Route } from '@playwright/test';
import { createHash, randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { mockSessionRead, mockRevisionRead } from '../mock-session';
import type { BatchDetail, CapsuleOut, FractionOut, Mineral, SourceInput, SourceOut } from '../../src/lib/tims';
import type { Grain, Spot } from '../../src/lib/core/types';

export const server = 'https://tims.example.test';
export const projectId = '10000000-0000-4000-8000-000000000001';
export const mountId = '20000000-0000-4000-8000-000000000001';
export const sample = { id: '30000000-0000-4000-8000-000000000001', name: 'SAMPLE1' };
const set: CapsuleOut = { id: '40000000-0000-4000-8000-000000000001', lab_id: 'lab-1', name: 'Krogh', code: 'K', positions: 15, capsule_labels: Array.from({ length: 15 }, (_, i) => `K${i + 1}`) };
const letters: Record<Mineral, string> = { zircon: 'z', baddeleyite: 'b', titanite: 't', rutile: 'r', apatite: 'a', monazite: 'm', xenotime: 'x', other: 'o' };

/** Entirely invented data; the generated bundle is kept under ignored target/. */
export async function mockTimsServer(page: Page, generated: string) {
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui as { grains: Record<string, Grain[]>; spots: Spot[]; labels: object[] };
  const fractions: FractionOut[] = [];
  const sets = [set];
  const batches: BatchDetail[] = [{ id: 'batch-1', capsule_set_id: set.id, started_on: '2026-01-02', operator_name: 'Synthetic operator', notes: '', capsule_map: {} }];
  const writes: { method: string; path: string; body: Record<string, unknown>; key?: string }[] = [];
  const replay = new Map<string, { body: string; result: unknown }>();
  const unexpected: string[] = [];
  let offline = false;
  const grains = Object.entries(ui.grains).flatMap(([area_id, gs]) => gs.map(g => ({ ...g, area_id, sample: sample.name, project_uuid: projectId, retired: false, merged_from: [] })));
  const elements = { grains, spots: ui.spots.map(s => ({ ...s, project_uuid: projectId, acquisition_ids: [] })), labels: ui.labels };
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const counts = { grains: grains.length, spots: ui.spots.length, labels: ui.labels.length, grains_retired: 0, grains_without_atlas_id: 0 };
  const active = () => fractions.filter(f => !f.retired_at);
  const occupied = (b: BatchDetail) => Object.fromEntries(Array.from({ length: set.positions }, (_, i) => [String(i + 1), active().find(f => f.batch_id === b.id && f.capsule_number === i + 1)?.id ?? null]));
  await page.addInitScript(({ server }) => {
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: false, syncAutomatically: false, quality: 100, nasRoot: '' }));
    sessionStorage.setItem('grainatlas.upbdb.token', 'synthetic-token');
  }, { server });
  await page.route(`${server}/**`, async route => {
    const request = route.request(), url = new URL(request.url()), p = url.pathname, method = request.method();
    const headers = { 'access-control-allow-origin': '*', 'access-control-expose-headers': 'ETag, X-Next-Cursor', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' };
    const json = (value: unknown, status = 200) => route.fulfill({ json: value, status, headers });
    const error = (detail: string, status = 422, code = 'validation_failed') => json({ detail, code }, status);
    const cached = async (value: unknown, next?: string) => {
      const etag = `"${createHash('sha256').update(JSON.stringify(value)).digest('hex')}"`;
      const h = { ...headers, ETag: etag, ...(next ? { 'X-Next-Cursor': next } : {}) };
      return route.fulfill(request.headers()['if-none-match'] === etag ? { status: 304, headers: h } : { json: value, headers: h });
    };
    if (offline) {
      if (method !== 'GET' && method !== 'OPTIONS') writes.push({ method, path: p, body: request.postDataJSON(), key: request.headers()['idempotency-key'] });
      return route.abort('internetdisconnected');
    }
    if (method === 'OPTIONS') return route.fulfill({ status: 204, headers });
    if (p.startsWith(`/atlas/projects/${projectId}/`)) {
      const relative = p.slice(`/atlas/projects/${projectId}/`.length);
      if (relative.split('/').includes('..')) return error('Invalid path', 400);
      const body = await readFile(path.join(generated, relative));
      return route.fulfill({ body, headers, contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
    }
    if (p === '/v1/fractions/suggest-name') {
      const letter = letters[(url.searchParams.get('mineral') ?? 'zircon') as Mineral];
      const max = Math.max(0, ...fractions.filter(f => f.name.startsWith(letter)).map(f => Number(/\d+/.exec(f.name)?.[0] ?? 0)));
      return json({ suggestion: `${letter}${max + 1}`, sample });
    }
    if (method === 'GET' && p === '/v1/fractions') {
      const rows = active().filter(f => !url.searchParams.get('batch') || f.batch_id === url.searchParams.get('batch')), start = Number(url.searchParams.get('after') ?? 0), limit = Number(url.searchParams.get('limit') ?? 100);
      return cached(rows.slice(start, start + limit), start + limit < rows.length ? String(start + limit) : undefined);
    }
    if (method === 'GET' && p === '/v1/tims/capsule-sets') return cached(sets);
    if (method === 'GET' && p === '/v1/tims/recipes') return cached([{ id: 'recipe-1' }]);
    if (method === 'GET' && p === '/v1/tims/batches') return cached(batches.filter(b => !url.searchParams.get('capsule_set') || b.capsule_set_id === url.searchParams.get('capsule_set')).map(b => ({ ...b, capsule_map: occupied(b) })));
    if (method === 'GET' && p.startsWith('/v1/tims/batches/')) {
      const b = batches.find(b => b.id === p.split('/')[4]);
      return b ? cached({ ...b, capsule_map: occupied(b), capsule_set: sets.find(s => s.id === b.capsule_set_id), fractions: active().filter(f => f.batch_id === b.id) }) : error('Batch not found', 404, 'not_found');
    }
    if (p.startsWith('/v1/fractions') || p === '/v1/tims/batches') {
      const body = request.postDataJSON() as Record<string, unknown>, key = request.headers()['idempotency-key'];
      writes.push({ method, path: p, body, key });
      if (!key) return error('An idempotency key is required', 400);
      const signature = JSON.stringify([method, p, body]), previous = replay.get(key);
      if (previous) return previous.body === signature ? json(previous.result) : error('Key reused', 422, 'idempotency_key_reused');
      const done = (result: unknown) => { replay.set(key, { body: signature, result: structuredClone(result) }); return json(result); };
      if (p === '/v1/tims/batches') {
        const batch: BatchDetail = { id: randomUUID(), capsule_set_id: String(body.capsule_set_id), started_on: String(body.started_on), notes: String(body.notes ?? ''), operator_name: 'Synthetic operator', capsule_map: {} };
        batches.push(batch); return done(batch);
      }
      const id = p.split('/')[3], f = fractions.find(f => f.id === id);
      if (p === '/v1/fractions' || (f && method === 'PATCH')) {
        const fields = { ...(f ?? {}), ...body };
        const name = String(fields.name ?? '').toLowerCase();
        if (!/^[a-z]{1,3}[0-9]{1,3}[a-z]?$/.test(name)) return error('Use a name such as z1');
        if (active().some(row => row.id !== f?.id && row.name === name)) return error('That fraction name is already used for this sample');
        if (fields.batch_id && active().some(row => row.id !== f?.id && row.batch_id === fields.batch_id && row.capsule_number === fields.capsule_number)) return error('That capsule is already taken');
        const sources = ((body.sources ?? []) as SourceInput[]).map(s => ({ ...s, id: randomUUID(), spot_label: ui.spots.find(p => p.id === s.atlas_spot_id)?.name }));
        for (const s of sources) {
          if (s.is_fragment && !s.fragment_note?.trim()) return error('A fragment note is required');
          if (active().some(row => row.sources.some(old => old.atlas_grain_id === s.atlas_grain_id && (!s.is_fragment || !old.is_fragment)))) return error('This grain is already plucked; both sources must be fragments');
        }
        const value: FractionOut = { id: f?.id ?? randomUUID(), sample, mineral: fields.mineral as Mineral, name, notes: String(fields.notes ?? ''), batch_id: (fields.batch_id as string | null) ?? null, capsule_number: (fields.capsule_number as number | null) ?? null, status: 'picked', sources: f?.sources ?? sources, capsule_set_name: fields.batch_id ? set.name : null, capsule_label: fields.batch_id ? `K${fields.capsule_number}` : null };
        if (f) Object.assign(f, value); else fractions.push(value); return done(value);
      }
      if (!f) return error('Fraction not found', 404, 'not_found');
      if (p.endsWith('/sources')) {
        const s: SourceOut = { ...(body as unknown as SourceInput), id: randomUUID() };
        if (s.is_fragment && !s.fragment_note?.trim()) return error('A fragment note is required');
        f.sources.push(s); return done(s);
      }
      const sourceId = p.split('/')[5];
      if (sourceId) {
        const source = f.sources.find(s => s.id === sourceId); if (!source) return error('Source not found', 404, 'not_found');
        f.sources = f.sources.filter(s => s.id !== sourceId); return done({ ...source, retired_at: new Date().toISOString() });
      }
      f.retired_at = new Date().toISOString(); return done(f);
    }
    if (await mockRevisionRead(route)) return;
    const session = mockSessionRead(method, p); if (session !== undefined) return json(session);
    if (method === 'GET') {
      if (p === '/v1/mounts') return json([{ id: mountId, label: 'MOUNT_A', mount_type: 'epoxy_1in', mineral: ['zircon'], ...counts, grain_count: counts.grains, spot_count: counts.spots, can_edit: true }]);
      if (p === '/v1/atlas/projects' || p === '/v1/sessions') return json([]);
      if (p === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'MOUNT_A', name: project.name, package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1, counts, hashes, can_edit: true, current_project_uuid: projectId, project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
      if (p === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elements, mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId });
      if (p === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
      if (p === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision: 1, since: 1, changes: [] });
    }
    unexpected.push(`${method} ${p}`); return error('Unexpected request', 404, 'not_found');
  });
  return { project, ui, fractions, batches, writes, unexpected, set, sets, setOffline: (value: boolean) => { offline = value; } };
}
