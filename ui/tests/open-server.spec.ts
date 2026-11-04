import { expect, test, type Page } from '@playwright/test';

// Open from server (WORKFLOW.md 3.1): the fixture bundle (`make atlas-fixture`, fixed ids, three
// spots, linked to a mount) served by the fake laicpmsdb of `pnpm preview` at
// /fake-laicpmsdb/atlas/projects/<id>/.

const FAKE = 'http://localhost:4173/fake-laicpmsdb/ns/open-server';
const PROJECT = '01972c4e-0000-7000-8000-00000000a7a5';
const SPOT2 = '01972c4e-0000-7000-8000-00000000a8a6';

async function center(page: Page, sel: string) {
  const b = (await page.locator(sel).first().boundingBox())!;
  return [b.x + b.width / 2, b.y + b.height / 2];
}

test('?server=&project=&spot= opens the published bundle read-only, selects and centres the spot', async ({ page, request }) => {
  await request.post(`${FAKE}/_fake/reset`);
  const tiles: string[] = [];
  page.on('response', (r) => {
    if (r.url().includes('.webp') && r.ok()) tiles.push(r.url());
  });
  // `access` is laicpmsdb's signed read-only token for the project's files (from GET /v1/mounts/{id}/atlas).
  await page.goto(`./?server=${encodeURIComponent(FAKE)}&project=${PROJECT}&spot=${SPOT2}&access=test-access`);
  await expect(page.getByTestId('remote-name')).toContainText('read-only');
  await expect(page.getByTestId('spot')).toHaveCount(3);
  // 20 grains with the joint background correction and the artifact classifier together (artifacts left out;
  // 21 with per-tile leveling, 25 outlines before the classifier).
  await expect(page.locator('polygon.grain:not(.artifact)')).toHaveCount(20);
  await expect(page.getByRole('dialog', { name: 'Spot HJ-GC1-2' })).toBeVisible();
  await expect(page.locator('[data-testid=spot].sel')).toHaveCount(1);
  expect(new URL(page.url()).search).toBe('');
  await expect
    .poll(async () => {
      const [cx, cy] = await center(page, '[data-testid=spot].sel circle');
      const [vx, vy] = await center(page, '.viewer');
      return Math.hypot(cx - vx, cy - vy);
    })
    .toBeLessThan(40);
  // Pixels come from the server's WebP tiles, drawn on the canvas.
  await expect.poll(() => tiles.length).toBeGreaterThan(0);
  expect(tiles[0]).toContain(`${FAKE}/atlas/projects/${PROJECT}/pyramid/001_HJ-GC1/`);
  expect(tiles[0]).toContain('?access=test-access');
  await expect
    .poll(() =>
      page.evaluate(() => {
        const c = document.querySelector('canvas')!;
        const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
        let lit = 0;
        for (let i = 0; i < d.length; i += 4 * 97) if (d[i] > 40) lit++;
        return lit;
      }),
    )
    .toBeGreaterThan(50);

  // Read-only: select and measure only, the panels cannot edit, Delete does nothing.
  await expect(page.getByTestId('read-only')).toBeVisible();
  await expect(page.getByTestId('tool-spot')).toHaveCount(0);
  await expect(page.getByTestId('tool-measure')).toBeVisible();
  await expect(page.getByTestId('panel-name')).toBeDisabled();
  await page.locator('.viewer').click({ position: { x: 5, y: 5 } }).catch(() => {});
  await page.keyboard.press('Delete');
  await expect(page.getByTestId('spot')).toHaveCount(3);
  await expect(page.getByRole('button', { name: 'Area' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Layout' })).toBeDisabled();
  // Nothing of the server's project is stored as this browser's.
  expect(await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('grainatlas.project.')))).toEqual([]);

  // Edit a copy: a local project with the same id, editable, kept in the browser.
  await page.getByTestId('edit-copy').click();
  await expect(page.getByTestId('remote-name')).toContainText('local copy');
  await expect(page.getByTestId('read-only')).toHaveCount(0);
  await expect(page.getByTestId('find-grains')).toHaveCount(0); // on the Grains tab, disabled there
  const canvas = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  await page.mouse.click(canvas.x + canvas.width * 0.25, canvas.y + canvas.height * 0.25);
  await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(4);
  await expect
    .poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X') || 'null')?.grainatlas_ui?.spots?.length))
    .toBe(4);
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X')!));
  expect(saved.uuid).toBe(PROJECT);
  expect(saved.laicpmsdb.mount_id).toBe('01972c4e-0000-7000-8000-0000000000b1');
});

test('Settings opens a published project by id with the token; without one, or a missing one, says so', async ({ page }) => {
  await page.goto('./');
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('server').fill(FAKE);
  await page.getByTestId('server').press('Tab');
  await page.getByTestId('open-project-id').fill(PROJECT);
  await page.getByTestId('open-from-server').click();
  await expect(page.getByRole('alert')).toContainText('atlas.json: 401');
  await page.getByRole('alert').getByRole('button', { name: 'Dismiss' }).click();
  await page.getByTestId('token').fill('test-token');
  await page.getByTestId('token').press('Tab');
  await page.getByTestId('open-project-id').fill('01972c4e-0000-7000-8000-0000000fffff');
  await page.getByTestId('open-from-server').click();
  await expect(page.getByRole('alert')).toContainText('atlas.json: 404');
  await page.getByRole('alert').getByRole('button', { name: 'Dismiss' }).click();
  await page.getByTestId('open-project-id').fill(PROJECT);
  await page.getByTestId('open-from-server').click();
  await expect(page.getByTestId('spot')).toHaveCount(3);
  await expect(page.getByTestId('remote-name')).toContainText('read-only');
});
