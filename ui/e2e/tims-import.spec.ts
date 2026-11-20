import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { mockTimsServer, mountId, projectId, sample, server } from './support/tims';
import type { PickNotesPlan, PickNotesRequest } from '../src/lib/tims-import';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const generated = path.join(root, 'target/remote-edit-bundle');
test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => { execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'synthetic_bundle'], { cwd: root, stdio: 'pipe' }); });

test('preview refuses a line, narrowing the range enables a confirmed import and refreshes badges', async ({ page }) => {
  const mock = await mockTimsServer(page, generated), grain = Object.values(mock.ui.grains).flat()[0];
  const notes = '### 11/14/2026\nSAMPLE1\nKrogh\nK1-z1:S 195\nK2-z2:S 999';
  let previews = 0, imports = 0, key: string | undefined;
  function plan(body: PickNotesRequest): PickNotesPlan {
    const refused = !body.last_line || body.last_line >= 5;
    return { lab_id: mock.set.lab_id, first_line: body.first_line ?? 1, last_line: body.last_line ?? 5,
      summary: { ok: 1, refused: refused ? 1 : 0, ignored: refused ? 3 : 4, skipped: 0 }, batches: [{ set_id: mock.set.id, started_on: '2026-11-14', action: 'create', id: null }],
      lines: [{ number: 1, text: '### 11/14/2026', kind: 'date', status: 'ignored', code: '', message: '', parsed: {}, resolved: {} },
        { number: 4, text: 'K1-z1:S 195', kind: 'pick', status: 'ok', code: '', message: '', parsed: { name: 'z1' }, resolved: { sample: { id: sample.id, label: 'SAMPLE1' }, mount: { id: mountId, label: 'MOUNT_A' }, spot: { id: 'spot', label: 'SAMPLE1 195' }, grain: { id: grain.id, label: 'MOUNT_A/1' }, capsule_label: 'K1' } },
        { number: 5, text: 'K2-z2:S 999', kind: 'pick', status: refused ? 'refused' : 'ignored', code: 'spot_not_found', message: 'No matching spot', parsed: { name: 'z2' }, resolved: {} }],
    };
  }
  await page.route(`${server}/v1/tims/pick-notes/**`, async route => {
    const request = route.request();
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' } });
    const body = request.postDataJSON() as PickNotesRequest, p = plan(body);
    const headers = { 'access-control-allow-origin': '*' };
    if (request.url().includes('/preview')) { previews++; return route.fulfill({ json: p, headers }); }
    imports++; key = request.headers()['idempotency-key'];
    expect(body.last_line).toBe(4); expect(key).toBeTruthy();
    mock.fractions.push({ id: 'imported', name: 'z1', sample, mineral: 'zircon', batch_id: 'batch-1', capsule_number: 1, capsule_label: 'K1', notes: '', status: 'picked', sources: [{ id: 'imported-source', atlas_grain_id: grain.id }] });
    return route.fulfill({ json: { plan: p, created: { fractions: ['imported'], batches: ['batch-1'], sources: ['imported-source'], steps: [] } }, headers });
  });
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  await page.getByTestId('edit-copy').click();
  await page.getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('import-pick-list').click();
  const dialog = page.getByTestId('pick-import-dialog');
  await dialog.getByLabel('Notes', { exact: true }).fill(notes);
  await dialog.getByTestId('pick-preview').click();
  await expect(dialog.getByTestId('pick-import-row').filter({ hasText: 'refused: No matching spot' })).toBeVisible();
  await expect(dialog.getByTestId('pick-import')).toBeDisabled(); expect(imports).toBe(0);
  await dialog.getByLabel('Last line', { exact: true }).fill('4');
  await dialog.getByTestId('pick-preview').click(); await expect(dialog.getByTestId('pick-import')).toBeEnabled();
  await dialog.getByTestId('pick-import').click();
  await expect(dialog.getByTestId('pick-import-confirm')).toContainText('Create 1 fractions in 1 batches in the database?'); expect(imports).toBe(0);
  await dialog.getByRole('button', { name: 'Confirm import', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Created 1 fractions'); expect(imports).toBe(1); expect(previews).toBe(2);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('button', { name: 'Canvas', exact: true }).click();
  await expect(page.getByTestId('tims-badge')).toHaveText('K1 · z1'); expect(mock.unexpected).toEqual([]);
});

test('Open offers an optional import, and offline notes remain in the dialog', async ({ page }) => {
  const mock = await mockTimsServer(page, generated);
  await page.goto('/'); await page.getByTestId('import-pick-list').click();
  const dialog = page.getByTestId('pick-import-dialog');
  await dialog.getByLabel('Notes', { exact: true }).fill('SAMPLE1\nK1-z1:S 195');
  mock.setOffline(true); await page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await expect(dialog).toContainText('Import needs the database; your notes stay here');
  await expect(dialog.getByTestId('pick-preview')).toBeDisabled();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click(); await page.getByTestId('import-pick-list').click();
  await expect(dialog.getByLabel('Notes', { exact: true })).toHaveValue('SAMPLE1\nK1-z1:S 195');
});
