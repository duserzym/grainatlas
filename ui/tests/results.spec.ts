import { expect, test } from '@playwright/test';

// Results overlay (WORKFLOW.md 5) from GET /v1/mounts/{id}/results, in laicpmsdb's shape
// (its docs/workflow_api.md): the fake laicpmsdb of `pnpm preview` answers with
// tests/fixtures/mount-results.json, whose entries are the fixture bundle's three spots (two by
// atlas_spot_id, one by label only), a second acquisition without values at the first spot,
// and a spot not measured yet.

const FAKE = 'http://localhost:4173/fake-laicpmsdb/ns/results';
const PROJECT = '01972c4e-0000-7000-8000-00000000a7a5';
const SPOT2 = '01972c4e-0000-7000-8000-00000000a8a6';

test('results of the linked mount in the spot panel and as a coloured layer', async ({ page, request, context }) => {
  await request.post(`${FAKE}/_fake/reset`);
  await page.goto('./');
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('token').fill('test-token');
  await page.getByTestId('token').press('Tab');
  await page.goto(`./?server=${encodeURIComponent(FAKE)}&project=${PROJECT}&spot=${SPOT2}`);
  await expect(page.getByTestId('spot')).toHaveCount(3);

  // The selected spot's values, its flag, and the link to its acquisition.
  const box = page.getByTestId('spot-results');
  await expect(box).toContainText('1488.9 ± 12.6 Ma');
  await expect(box).toContainText('206Pb/238U · 2se · fails filters');
  await expect(box).toContainText('discordant');
  await expect(box).toContainText('high ratio scatter');
  await expect(box).toContainText('0.25981 ± 0.00230 (2se)');
  await expect(box).toContainText('958 ± 20 ppm');
  await expect(box).toContainText('Th/U');
  await expect(box).toContainText('Zircon_03Nov26');

  // The layer: all three spots coloured (one matched by label), one flagged, a legend.
  await expect(page.getByTestId('result-dot')).toHaveCount(3);
  await expect(page.getByTestId('result-flag')).toHaveCount(1);
  const legend = page.getByTestId('results-legend');
  await expect(legend).toContainText('Preferred date (Ma)');
  await expect(legend).toContainText('1012');
  await expect(legend).toContainText('2011');
  await expect(legend).toContainText('3 of 3 spots');
  const fills = await page.getByTestId('result-dot').evaluateAll((els) => els.map((e) => e.getAttribute('fill')));
  expect(new Set(fills).size).toBe(3);

  // Colour by a concentration; the status in the laicpmsdb tab; layer off.
  await page.getByTestId('tab-link').click();
  await expect(page.getByTestId('results-status')).toContainText('3 of 3 spots have results (preferred run)');
  await page.getByTestId('colour-by').selectOption('U');
  await expect(legend).toContainText('U (ppm)');
  await expect(legend).toContainText('233');
  await page.getByTestId('refresh-results').click();
  await expect(page.getByTestId('results-status')).toContainText('3 of 3');
  const st = await (await request.get(`${FAKE}/_fake/state`)).json();
  const calls = st.requests.filter((r: { path: string }) => r.path.endsWith('/results'));
  expect(calls.length).toBe(2);
  expect(calls[0].auth).toBe('Bearer test-token');
  expect(calls[0].path).toBe('/v1/mounts/01972c4e-0000-7000-8000-0000000000b1/results');

  // Open acquisition: not inside UPb Studio, so laicpmsdb's own page opens.
  const popup = context.waitForEvent('page');
  await page.getByTestId('open-acquisition').click();
  expect((await popup).url()).toBe(`${FAKE}/app/#/acquisitions/01972c4f-0000-7000-8000-0000000000a2`);

  await page.getByText('show on the canvas').click();
  await expect(page.getByTestId('result-dot')).toHaveCount(0);
  await expect(legend).toHaveCount(0);
});

test("laicpmsdb's viewer link: results with the access token alone, for the published mount only", async ({ page, request }) => {
  // viewer_url as laicpmsdb 1bf6ad4 frames it: /atlas/?server=&project=&mount=&access=; no bearer token.
  await request.post(`${FAKE}/_fake/reset`);
  await page.goto(`./?server=${encodeURIComponent(FAKE)}&project=${PROJECT}&mount=01972c4e-0000-7000-8000-0000000000b1&access=test-access`);
  await expect(page.getByTestId('spot')).toHaveCount(3);
  await expect(page.getByTestId('result-dot')).toHaveCount(3);
  const st = await (await request.get(`${FAKE}/_fake/state`)).json();
  const call = st.requests.find((r: { path: string }) => r.path.endsWith('/results'));
  expect(call).toMatchObject({ path: '/v1/mounts/01972c4e-0000-7000-8000-0000000000b1/results', auth: null, access: true });

  // A mount the token's project is not published on: 404, shown next to the feature, nothing drawn.
  await page.goto(`./?server=${encodeURIComponent(FAKE)}&project=${PROJECT}&mount=01972c4e-0000-7000-8000-0000000000b2&access=test-access`);
  await expect(page.getByTestId('spot')).toHaveCount(3);
  await page.getByTestId('tab-link').click();
  await expect(page.getByTestId('results-status')).toContainText('404');
  await expect(page.getByTestId('result-dot')).toHaveCount(0);
});
