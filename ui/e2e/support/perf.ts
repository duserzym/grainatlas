import { expect, type Page } from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Grain, Spot } from '../../src/lib/core/types';
import { mockSessionRead } from '../mock-session';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
export const generated = path.join(root, 'target/large-bundle');
export const server = 'https://upbdb.test';
export const projectId = '10000000-0000-4000-8000-000000000001';
export const mountId = '20000000-0000-4000-8000-000000000001';
export const projectURL = `/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`;
export const latencyMs = Number(process.env.PERF_LATENCY_MS ?? 0);
if (!Number.isFinite(latencyMs) || latencyMs < 0) throw new Error('PERF_LATENCY_MS must be a non-negative number');

export async function outputPath(kind: string, extension = 'json') {
  const label = process.env.PERF_LABEL ?? 'run';
  if (!/^[a-zA-Z0-9_-]+$/.test(label)) throw new Error('PERF_LABEL must contain only letters, digits, _ or -');
  const folder = path.join(root, 'target/perf');
  await mkdir(folder, { recursive: true });
  return path.join(folder, `${kind}-${label}.${extension}`);
}

export async function report(kind: string, value: unknown) {
  const json = JSON.stringify(value, null, 2);
  await writeFile(await outputPath(kind), `${json}\n`);
  console.log(json);
}

export function distribution(values: number[]) {
  const sorted = [...values].sort((a, b) => a - b);
  const percentile = (p: number) => sorted.length ? sorted[Math.max(0, Math.ceil(p * sorted.length) - 1)] : null;
  return { n: sorted.length, median: percentile(0.5), p90: percentile(0.9), max: sorted.at(-1) ?? null };
}

export interface TileRequest {
  path: string;
  area: string;
  level: number;
  file: string;
  bytes: number;
  served: boolean;
}

export function tileTotals(requests: TileRequest[]) {
  const levels: Record<string, { requests: number; bytes: number }> = {};
  for (const r of requests) {
    const level = levels[r.level] ??= { requests: 0, bytes: 0 };
    level.requests++;
    level.bytes += r.bytes;
  }
  return { requests: requests.length, served: requests.filter(r => r.served).length,
    bytes: requests.reduce((sum, r) => sum + r.bytes, 0), byLevel: levels, tiles: requests };
}

export async function mockLargeServer(page: Page, options: { measuredSpotIndex?: number } = {}) {
  // Generate explicitly before running specs, so fixture creation never contaminates timings.
  const project = JSON.parse(await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const ui = project.grainatlas_ui as { grains: Record<string, Grain[]>; spots: Spot[]; labels: object[] };
  if (options.measuredSpotIndex !== undefined) {
    ui.spots[options.measuredSpotIndex].acquisition_id = '60000000-0000-4000-8000-000000000001';
  }
  const grains = Object.entries(ui.grains).flatMap(([area_id, gs]) => gs.map(g => ({
    ...g, area_id, project_uuid: projectId, retired: false, merged_from: [],
  })));
  const elements = { grains, spots: ui.spots.map(s => ({ ...s, project_uuid: projectId,
    acquisition_ids: s.acquisition_id ? [s.acquisition_id] : [] })),
    labels: ui.labels.map(l => ({ ...l, project_uuid: projectId })) };
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const counts = { grains: grains.length, spots: ui.spots.length, labels: ui.labels.length,
    grains_retired: 0, grains_without_atlas_id: 0 };
  const files = new Map<string, Buffer>();
  for (const relative of ['atlas.json', ...atlas.files.map((f: { path: string }) => f.path)]) {
    files.set(relative, await readFile(path.join(generated, relative)));
  }
  await page.addInitScript(({ server }) => {
    const w = window as typeof window & { __LAB_SHELL__?: object; __TAURI_INTERNALS__?: object };
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => cmd === 'bridge_token' ? 'synthetic-token' : null };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave: false, quality: 100, nasRoot: '' }));
  }, { server });

  const requests: TileRequest[] = [];
  const unexpected: string[] = [];
  // Playwright page.route disables HTTP cache. CDP interception keeps the same
  // hosted mock protocol while leaving browser cache settings and IndexedDB intact.
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Fetch.enable', { patterns: [{ urlPattern: `${server}/*`, requestStage: 'Request' }] });
  cdp.on('Fetch.requestPaused', async event => {
    try {
      const { request, requestId } = event;
      const pathname = new URL(request.url).pathname;
      const headers = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*',
        'access-control-allow-methods': '*', 'cache-control': 'no-store' };
      const fulfill = async (body: Buffer, contentType: string, responseCode = 200, cacheable = false) => {
        if (latencyMs) await new Promise(resolve => setTimeout(resolve, latencyMs));
        await cdp.send('Fetch.fulfillRequest', { requestId, responseCode, body: body.toString('base64'),
          responseHeaders: Object.entries({ ...headers, 'content-type': contentType,
            ...(cacheable ? { 'cache-control': 'public, max-age=3600' } : {}) }).map(([name, value]) => ({ name, value })) });
      };
      const json = (value: unknown) => fulfill(Buffer.from(JSON.stringify(value)), 'application/json');
      if (request.method === 'OPTIONS') { await fulfill(Buffer.alloc(0), 'text/plain', 204); return; }
      const session = mockSessionRead(request.method, pathname);
      if (session !== undefined) { await json(session); return; }
      if (request.method === 'GET') {
        if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
          const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
          const body = files.get(relative);
          if (!body) throw new Error(`Unexpected bundle path ${relative}`);
          const tile = /^pyramid\/([^/]+)\/(\d+)\/([^/]+\.webp)$/.exec(relative);
          const record: TileRequest | null = tile ? { path: relative, area: tile[1], level: Number(tile[2]),
            file: tile[3], bytes: 0, served: false } : null;
          if (record) requests.push(record);
          await fulfill(body, tile ? 'image/webp' : 'application/json', 200, true);
          if (record) { record.bytes = body.length; record.served = true; }
          return;
        }
        if (pathname === '/v1/mounts') { await json([{ id: mountId, label: 'Synthetic mount', mount_type: 'epoxy_1in',
          grain_count: counts.grains, spot_count: counts.spots, can_edit: true }]); return; }
        if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') { await json([]); return; }
        if (pathname === `/v1/atlas/projects/${projectId}`) {
          await json({ project_uuid: projectId, mount_id: mountId, mount_label: 'Synthetic mount', name: project.name,
            package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1,
            counts, hashes, can_edit: true, current_project_uuid: projectId,
            project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] }); return;
        }
        if (pathname === `/v1/atlas/projects/${projectId}/elements`) {
          await json({ ...elements, mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId }); return;
        }
        if (pathname === `/v1/mounts/${mountId}/results`) { await json({ mount_id: mountId, spots: [] }); return; }
        if (pathname === `/v1/mounts/${mountId}/changes`) { await json({ mount_id: mountId, revision: 1, since: 1, changes: [] }); return; }
      }
      unexpected.push(`${request.method} ${pathname}`);
      await fulfill(Buffer.from('{"detail":"Unexpected synthetic endpoint"}'), 'application/json', 404);
    } catch (error) {
      unexpected.push(String(error));
      await cdp.send('Fetch.failRequest', { requestId: event.requestId, errorReason: 'Failed' }).catch(() => {});
    }
  });
  return { project, atlas, ui, requests, unexpected };
}

export async function openEdit(page: Page) {
  await page.goto(projectURL);
  await expect(page.getByTestId('edit-copy')).toHaveText('Edit');
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('edit-copy')).toHaveText('View');
  await expect(page.getByTestId('busy')).toHaveCount(0);
}

export async function savedProject(page: Page) {
  const downloaded = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  // A missing acquisition label is advisory in this invented fixture.
  await Promise.race([
    downloaded.then(() => {}),
    page.getByTestId('checks-continue').waitFor({ state: 'visible' }).then(() => page.getByTestId('checks-continue').click()),
  ]);
  const file = await (await downloaded).path();
  if (!file) throw new Error('Saved project has no download path');
  return JSON.parse(await readFile(file, 'utf8'));
}
