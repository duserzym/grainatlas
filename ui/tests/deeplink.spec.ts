import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Deep links as UPb Studio sends them: grainatlas://project/<id>/spot/<id>
// becomes index.html?project=<id>&spot=<id>. Runs against the static build.

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

async function center(page: Page, sel: string) {
  const b = (await page.locator(sel).first().boundingBox())!;
  return [b.x + b.width / 2, b.y + b.height / 2];
}

test('?project=&spot= reopens the example, selects the spot and centers on it', async ({ page }) => {
  // Make a project with one spot off-centre, then export it for laicpmsdb.
  await page.goto('./');
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  const canvas = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  await page.mouse.click(canvas.x + canvas.width * 0.22, canvas.y + canvas.height * 0.3);
  await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(1);
  // Not hosted: the laicpmsdb action is there but disabled.
  await expect(page.getByTestId('open-laicpmsdb')).toBeDisabled();

  const dl = page.waitForEvent('download');
  await page.getByTestId('send-laicpmsdb').click();
  const body = JSON.parse(readFileSync(await (await dl).path(), 'utf8'));
  expect(body.project_id).toMatch(UUID);
  expect(body.sample).toBe('HJ-GC1');
  expect(body.spots).toHaveLength(1);
  const s = body.spots[0];
  expect(Object.keys(s).sort()).toEqual(['diameter_um', 'grain', 'id', 'kind', 'name', 'x_um', 'y_um']);
  expect(s.id).toMatch(UUID);
  expect(s.kind).toBe('spot');
  expect(typeof s.grain).toBe('string');
  expect(s.x_um).toBeGreaterThan(0);
  expect(s.diameter_um).toBe(25);

  // The browser kept the project (no pixels) and the recent list knows its id.
  const stored = await page.evaluate(() => {
    const recent = JSON.parse(localStorage.getItem('grainatlas.recent') || '[]');
    const keys = Object.keys(localStorage);
    return { recent, keys };
  });
  expect(stored.recent[0]).toMatchObject({ id: body.project_id, name: 'Sweden_XS_150X' });
  expect(stored.keys.every((k: string) => k.startsWith('grainatlas.'))).toBe(true);

  // Follow the link in a fresh page load.
  await page.goto(`./?project=${body.project_id}&spot=${s.id}`);
  await expect(page.getByRole('dialog', { name: `Spot ${s.name}` })).toBeVisible();
  await expect(page.locator('[data-testid=spot].sel')).toHaveCount(1);
  await expect(page.locator('.list tr.selected td').first()).toHaveText(s.name);
  expect(new URL(page.url()).search).toBe('');
  await expect
    .poll(async () => {
      const [cx, cy] = await center(page, '[data-testid=spot].sel circle');
      const [vx, vy] = await center(page, '.viewer');
      return Math.hypot(cx - vx, cy - vy);
    })
    .toBeLessThan(40);
});

test('a link to a project this browser does not know names it and offers Open', async ({ page }) => {
  const id = '01972c4e-0000-7000-8000-000000000001';
  await page.goto(`./?project=${id}&spot=01972c4e-0000-7000-8000-000000000002`);
  const box = page.getByTestId('deeplink');
  await expect(box).toBeVisible();
  await expect(box).toContainText(id);
  await expect(box).toContainText('not available on this computer');
  await expect(page.getByTestId('deeplink-open')).toBeEnabled();
});

test('no parameters: the Open screen as usual', async ({ page }) => {
  await page.goto('./');
  await expect(page.getByTestId('example')).toBeVisible();
  await expect(page.getByTestId('deeplink')).toHaveCount(0);
});
