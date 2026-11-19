import type { Page } from '@playwright/test';
import { expect, mockSessionRead, mockRevisionRead } from '../mock-session';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
export const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../..');
export const generated=path.join(root,'target/print-edit-bundle');
export const server='https://upbdb.test';
export const projectId='10000000-0000-4000-8000-000000000001';
const mountId='20000000-0000-4000-8000-000000000001';
export const projectURL=`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`;
export interface SavedFixture { saved?: string; syncOnSave?: boolean }
export async function mockImageServer(page: Page, fixture: SavedFixture = {}) {
  const project = JSON.parse(fixture.saved ?? await readFile(path.join(generated, 'project.json'), 'utf8'));
  const atlas = JSON.parse(await readFile(path.join(generated, 'atlas.json'), 'utf8'));
  const hashes = { grains: 'grains-1', spots: 'spots-1', labels: 'labels-1' };
  const unexpected: string[] = [];
  const elementsOf = (record: any) => ({
    grains: record.grainatlas_ui.grains.a.map((g: any) => ({ ...g, area_id: 'a', sample: record.areas[0].name, project_uuid: projectId, retired: false, merged_from: [] })),
    spots: record.grainatlas_ui.spots.map((s: any) => ({ ...s, project_uuid: projectId, acquisition_ids: s.acquisition_ids ?? [] })),
    labels: record.grainatlas_ui.labels.map((l: any) => ({ ...l, project_uuid: projectId })),
  });
  await page.addInitScript(({ server, syncOnSave }) => {
    const w = window as any;
    w.__LAB_SHELL__ = { module: 'grainatlas' };
    w.__TAURI_INTERNALS__ = { invoke: async (cmd: string) => {
      if (cmd === 'bridge_token') return 'synthetic-token';
      return null;
    } };
    localStorage.setItem('grainatlas.upbdb', JSON.stringify({ server, syncOnSave, quality: 100, nasRoot: '' }));
  }, { server, syncOnSave: fixture.syncOnSave ?? true });
  await page.route(`${server}/**`, async route => {
    const request = route.request(), pathname = new URL(request.url()).pathname;
    const headers = { 'access-control-allow-origin': '*' };
    const json = (value: unknown) => route.fulfill({ json: value, headers });
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { ...headers, 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    if (await mockRevisionRead(route)) return;
    const session = mockSessionRead(request.method(), pathname);
    if (session !== undefined) return json(session);
    if (request.method() === 'GET') {
      if (pathname === '/v1/mounts') return json([{ id: mountId, label: 'mount A', mount_type: 'epoxy_1in', grain_count: 30, spot_count: 0, can_edit: true }]);
      if (pathname === '/v1/atlas/projects' || pathname === '/v1/sessions') return json([]);
      if (pathname.startsWith(`/atlas/projects/${projectId}/`)) {
        const relative = pathname.slice(`/atlas/projects/${projectId}/`.length);
        if (!['atlas.json', ...atlas.files.map((f: any) => f.path)].includes(relative)) throw new Error(`Unexpected bundle path ${relative}`);
        // Reopen the downloaded project record with the same invented image bundle.
        if (relative === 'project.json' && fixture.saved) return route.fulfill({ headers, body: fixture.saved, contentType: 'application/json' });
        if (relative === 'atlas.json' && fixture.saved) return json({ ...atlas, files: atlas.files.map((f: any) => f.path === 'project.json' ? { ...f, size: Buffer.byteLength(fixture.saved!), sha256: createHash('sha256').update(fixture.saved!).digest('hex') } : f) });
        return route.fulfill({ headers, body: await readFile(path.join(generated, relative)), contentType: relative.endsWith('.webp') ? 'image/webp' : 'application/json' });
      }
      if (pathname === `/v1/atlas/projects/${projectId}`) return json({ project_uuid: projectId, mount_id: mountId, mount_label: 'mount A', name: project.name,
        package_name: project.package.name, atlas_version: 1, atlas_bundle_id: 'bundle-1', revision: 1,
        counts: { spots: 0, grains: 30, labels: 0, grains_retired: 0, grains_without_atlas_id: 0 }, hashes, can_edit: true, current_project_uuid: projectId,
        project_uuids: [{ project_uuid: projectId, name: project.name, package_name: project.package.name, is_current: true }] });
      if (pathname === `/v1/atlas/projects/${projectId}/elements`) return json({ ...elementsOf(fixture.saved ? JSON.parse(fixture.saved) : project), mount_id: mountId, revision: 1, hashes, project_uuids: [projectId], current_project_uuid: projectId });
      if (pathname === `/v1/mounts/${mountId}/results`) return json({ mount_id: mountId, spots: [] });
      if (pathname === `/v1/mounts/${mountId}/changes`) return json({ mount_id: mountId, revision: 1, since: 1, changes: [] });
    }
    unexpected.push(`${request.method()} ${pathname}`);
    return route.fulfill({ status: 404, headers, json: { detail: 'Unexpected synthetic endpoint' } });
  });
  return unexpected;
}

export async function saveImageProject(page: Page) {
  await expect(page.getByTestId('busy')).toHaveCount(0);
  const downloaded = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  // The generated project can have advisory checks; keep the usual save path.
  const checks = page.getByTestId('checks-continue');
  const confirm = await Promise.race([downloaded.then(() => false), checks.waitFor({ state: 'visible' }).then(() => true).catch(() => false)]);
  if (confirm) await checks.click();
  const file = await (await downloaded).path();
  if (!file) throw new Error('Project download has no local path');
  return readFile(file, 'utf8');
}
