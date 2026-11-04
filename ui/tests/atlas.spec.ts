import { expect, test, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

// Atlas bundles (WORKFLOW.md 3.1): save, publish to laicpmsdb. The server is the fake under
// /fake-laicpmsdb (scripts/fake-laicpmsdb.mjs) that `pnpm preview` mounts; token test-token.

const FAKE = 'http://localhost:4173/fake-laicpmsdb';
const MOUNT = '01972c4e-0000-7000-8000-0000000000b1';

export async function useFakeServer(page: Page) {
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('server').fill(FAKE);
  await page.getByTestId('server').press('Tab');
  await page.getByTestId('token').fill('test-token');
  await page.getByTestId('token').press('Tab');
}

function unzip(buf: Buffer) {
  let end = buf.length - 22;
  while (buf.readUInt32LE(end) !== 0x06054b50) end--;
  const n = buf.readUInt16LE(end + 10);
  let at = buf.readUInt32LE(end + 16);
  const out: { name: string; data: Buffer }[] = [];
  for (let i = 0; i < n; i++) {
    const size = buf.readUInt32LE(at + 20);
    const nameLen = buf.readUInt16LE(at + 28);
    const local = buf.readUInt32LE(at + 42);
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
    out.push({ name: buf.subarray(at + 46, at + 46 + nameLen).toString(), data: buf.subarray(start, start + size) });
    at += 46 + nameLen + buf.readUInt16LE(at + 30) + buf.readUInt16LE(at + 32);
  }
  return out;
}

test('save the atlas bundle as a zip and publish it to a mount', async ({ page, request }) => {
  await request.post(`${FAKE}/_fake/reset`);
  await page.goto('./');
  await useFakeServer(page);
  await page.getByRole('button', { name: 'Open', exact: true }).click();
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  const canvas = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  await page.mouse.click(canvas.x + canvas.width * 0.3, canvas.y + canvas.height * 0.4);
  await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(1);

  // Link the mount from the server's list.
  await page.getByTestId('tab-link').click();
  await expect(page.getByTestId('mount-select')).toBeVisible();
  await page.getByTestId('mount-select').selectOption(MOUNT);

  // The zip: atlas.json first, every listed file present with its hash.
  const dl = page.waitForEvent('download');
  await page.getByTestId('save-atlas-zip').click();
  const d = await dl;
  const zip = readFileSync(await d.path());
  const files = unzip(zip);
  const projectId = d.suggestedFilename().replace('.atlas.zip', '');
  expect(files[0].name).toBe(`${projectId}.atlas/atlas.json`);
  const atlas = JSON.parse(files[0].data.toString());
  expect(atlas).toMatchObject({ format: 'grainatlas-atlas', version: 1, project_id: projectId, mount_id: MOUNT, opened_from: 'package', tile_size: 256, tile_encoding: 'lossless' });
  const byName = new Map(files.map((f) => [f.name.slice(projectId.length + 7), f.data]));
  for (const f of atlas.files) expect(createHash('sha256').update(byName.get(f.path)!).digest('hex')).toBe(f.sha256);
  expect(byName.has('pyramid/001_HJ-GC1/0/0_0.webp')).toBe(true);
  const spots = JSON.parse(byName.get('spots.json')!.toString());
  expect(spots.spots).toHaveLength(1);
  expect(spots.sample).toBe('HJ-GC1');

  // Publish: the fake checks every hash and answers with version 1, recorded in the project.
  await page.getByTestId('publish').click();
  await expect(page.locator('.toast.ok')).toContainText('Published version 1');
  await expect(page.getByTestId('published').locator('tbody tr')).toHaveCount(1);
  const state = await (await request.get(`${FAKE}/_fake/state`)).json();
  expect(state.published).toHaveLength(1);
  expect(state.published[0]).toMatchObject({ mount_id: MOUNT, project_id: projectId, version: 1, filename: `${projectId}.atlas.zip` });
  expect(state.requests.find((r: { path: string }) => r.path.endsWith('/atlas')).auth).toBe('Bearer test-token');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X') || '{}'));
  expect(saved.laicpmsdb.published[0]).toMatchObject({ version: 1, mount_id: MOUNT });
  expect(saved.laicpmsdb.published[0].zip_sha256).toBe(state.published[0].sha256);
  expect(saved.laicpmsdb.published[0].atlas_sha256).toMatch(/^[0-9a-f]{64}$/);

  // Publishing again makes version 2 (append-only).
  await page.getByTestId('publish').click();
  await expect(page.locator('.toast.ok')).toContainText('Published version 2');
});

test('Send spots posts to the linked mount when a token is there, and downloads without one', async ({ page, request }) => {
  await request.post(`${FAKE}/_fake/reset`);
  await page.goto('./');
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  const canvas = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  await page.mouse.click(canvas.x + canvas.width * 0.3, canvas.y + canvas.height * 0.4);
  await page.mouse.click(canvas.x + canvas.width * 0.5, canvas.y + canvas.height * 0.5);
  await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(2);
  // No server yet: the download, as before.
  const dl = page.waitForEvent('download');
  await page.getByTestId('send-laicpmsdb').click();
  expect((await dl).suggestedFilename()).toMatch(/_laicpmsdb-spots\.json$/);

  // Server and token in Settings, mount typed: a direct POST.
  await useFakeServer(page);
  await page.getByRole('button', { name: 'Canvas' }).click();
  await page.getByTestId('tab-link').click();
  await page.getByRole('button', { name: 'Type an id instead' }).click();
  await page.getByTestId('mount-id').fill(MOUNT);
  await page.getByTestId('mount-id').press('Tab');
  await page.getByRole('button', { name: /^Spots/ }).click();
  let downloaded = false;
  page.on('download', () => (downloaded = true));
  await page.getByTestId('send-laicpmsdb').click();
  await expect(page.locator('.toast.ok')).toContainText(`Sent to mount ${MOUNT}: 2 created`);
  expect(downloaded).toBe(false);
  const state = await (await request.get(`${FAKE}/_fake/state`)).json();
  expect(state.spots).toHaveLength(1);
  expect(state.spots[0].mount_id).toBe(MOUNT);
  expect(state.spots[0].doc.spots).toHaveLength(2);
  expect(state.spots[0].doc.sample).toBe('HJ-GC1');
  expect(state.requests.find((r: { path: string }) => r.path.endsWith('/spots')).auth).toBe('Bearer test-token');
});
