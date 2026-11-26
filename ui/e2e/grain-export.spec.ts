// Invented large bundle only. The supervisor runs these tests with a preview server.
import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { generated, mockLargeServer, openEdit, root } from './support/perf';

test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => {
  test.setTimeout(600_000);
  if (!existsSync(path.join(generated, 'atlas.json'))) {
    execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'large_bundle'],
      { cwd: root, stdio: 'pipe', timeout: 600_000 });
  }
});

/** Names of the entries in a STORE zip (the format the export writes). */
function zipNames(bytes: Buffer): string[] {
  const names: string[] = [];
  for (let at = 0; at + 4 <= bytes.length && bytes.readUInt32LE(at) === 0x04034b50;) {
    const size = bytes.readUInt32LE(at + 18), n = bytes.readUInt16LE(at + 26), extra = bytes.readUInt16LE(at + 28);
    names.push(bytes.subarray(at + 30, at + 30 + n).toString('utf8'));
    at += 30 + n + extra + size;
  }
  return names;
}

test('grain export dialog fits 1280 x 800 and the browser zip holds the table, outlines and crops', async ({ page }) => {
  await page.addInitScript(() => { delete (window as unknown as { showDirectoryPicker?: unknown }).showDirectoryPicker; });
  await mockLargeServer(page);
  await openEdit(page);
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption('a01');
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  await page.locator('.side .tabs').getByRole('radio', { name: /^Grains / }).click();
  await page.getByRole('button', { name: 'Export grains...' }).click();
  const dialog = page.getByTestId('grain-export-dialog');
  await expect(dialog).toBeVisible();
  const box = await dialog.boundingBox();
  expect(box && box.y >= 0 && box.y + box.height <= 800 && box.x + box.width <= 1280).toBeTruthy();
  await expect(dialog.getByText('Margin (µm)')).toBeVisible();
  await page.screenshot({ path: path.join(root, 'target/e2e-results/grain-export-dialog.png') });
  const download = page.waitForEvent('download');
  await dialog.getByRole('button', { name: 'Export', exact: true }).click();
  const file = await (await download).path();
  const names = zipNames(readFileSync(file));
  expect(names.some(n => n.endsWith('grains.csv'))).toBeTruthy();
  expect(names.some(n => n.endsWith('outlines.geojson'))).toBeTruthy();
  expect(names.some(n => n.endsWith('manifest.json'))).toBeTruthy();
  expect(names.some(n => n.endsWith('README.md'))).toBeTruthy();
  expect(names.filter(n => n.includes('crops/')).length).toBeGreaterThan(10);
  expect(names.filter(n => n.includes('masks/')).length).toBe(names.filter(n => n.includes('crops/')).length);
});
