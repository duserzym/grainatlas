import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mockTimsServer, mountId, projectId, sample, server } from './support/tims';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => { execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' }); });

test('pick list previews four plucks in capsule-set order, with local crops and page counts', async ({ page }) => {
  const mock = await mockTimsServer(page, generated);
  const other = { ...mock.set, id: 'set-2', name: 'Becquerel', code: 'B', capsule_labels: mock.set.capsule_labels.map(label => label.replace('K', 'B')) };
  mock.sets.push(other);
  mock.batches.push({ ...mock.batches[0], id: 'batch-2', capsule_set_id: other.id, started_on: '2026-02-01' });
  const grains = Object.values(mock.ui.grains).flat();
  for (const [i, n] of [3, 1, 4, 2].entries()) {
    const b = i < 2 ? mock.batches[0] : mock.batches[1], set = i < 2 ? mock.set : other;
    mock.fractions.push({ id: `fraction-${i}`, sample, name: `z${i + 1}`, mineral: 'zircon', batch_id: b.id, capsule_number: n, capsule_label: `${set.code}${n}`, notes: '', status: 'picked', sources: [{ id: `source-${i}`, atlas_grain_id: grains[i % grains.length].id, is_fragment: true, fragment_note: 'Tip' }] });
  }
  const databaseRows: string[] = [];
  page.on('request', request => { if (new URL(request.url()).pathname.startsWith('/v1/')) databaseRows.push(request.url()); });
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('edit-copy').click();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('style-picklist').check();
  await expect(page.getByLabel('Capsule set', { exact: true }).locator('option')).toHaveCount(3);
  await expect(page.getByLabel('Batch', { exact: true }).locator('option')).toHaveCount(3);
  const before = databaseRows.length;
  await page.getByTestId('picklist-preview').click();
  await expect(page.getByTestId('picklist-count')).toHaveText('4 rows · 2 pages');
  const svg = async () => page.getByTestId('page').evaluate(async (img: HTMLImageElement) => (await fetch(img.src)).text());
  const first = await svg();
  expect(first.match(/data-pick-row=/g)).toHaveLength(2);
  expect(first.indexOf('K1')).toBeLessThan(first.indexOf('K3'));
  expect(first).toContain('1 / 2'); expect(first).toContain('data:image/png;base64,'); expect(first).toContain('<polygon');
  await page.getByTestId('next-page').click();
  await expect(page.getByTestId('page')).toHaveAttribute('alt', 'Page 2');
  const second = await svg();
  expect(second.match(/data-pick-row=/g)).toHaveLength(2); expect(second.indexOf('B2')).toBeLessThan(second.indexOf('B4')); expect(second).toContain('2 / 2');
  expect(databaseRows.length).toBe(before); expect(mock.unexpected).toEqual([]);
  await page.getByTestId('export-pdf').scrollIntoViewIfNeeded(); await expect(page.getByTestId('export-pdf')).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
