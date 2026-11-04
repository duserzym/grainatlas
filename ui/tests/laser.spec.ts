import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Laser registration (phase 11): three synthetic fiducials from a known affine (0.5° rotation,
// y flipped, µm → mm, an offset), the exact fit, the generic CSV, then a fourth reading 10 µm
// off that must raise the warning. The registration is stored in project.json.

const ROT = (0.5 * Math.PI) / 180;
function stage(x: number, y: number): [number, number] {
  const c = Math.cos(ROT) / 1000, s = Math.sin(ROT) / 1000;
  return [12.5 + c * x + s * y, 40.25 + s * x - c * y];
}

test('three fiducials register the laser stage; the spot list is in mm; a bad fourth warns', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  const canvas = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  for (const [fx, fy] of [[0.35, 0.3], [0.65, 0.35], [0.45, 0.75], [0.6, 0.6]]) await page.mouse.click(canvas.x + canvas.width * fx, canvas.y + canvas.height * fy);
  await page.keyboard.up('Alt');
  await expect(page.getByTestId('spot')).toHaveCount(4);
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X') || 'null')?.grainatlas_ui?.spots?.length)).toBe(4);
  const spots: { id: string; name: string; x_um: number; y_um: number }[] = await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X')!).grainatlas_ui.spots);

  await page.getByTestId('tab-laser').click();
  await expect(page.getByTestId('laser-units')).toHaveValue('mm');
  for (let i = 0; i < 3; i++) {
    await page.getByTestId('add-fiducial').click();
    const [sx, sy] = stage(spots[i].x_um, spots[i].y_um);
    await page.getByTestId('fid-x').nth(i).fill(sx.toFixed(6));
    await page.getByTestId('fid-x').nth(i).press('Tab');
    await page.getByTestId('fid-y').nth(i).fill(sy.toFixed(6));
    await page.getByTestId('fid-y').nth(i).press('Tab');
  }
  await expect(page.getByTestId('laser-fit')).toContainText('3 fiducials fit exactly');
  await expect(page.getByTestId('laser-fit')).toContainText('rotation 0.50°');
  await expect(page.getByTestId('laser-fit')).toContainText('y flipped');

  // The generic CSV: the fourth spot (not a fiducial) lands where the true map puts it.
  const dl = page.waitForEvent('download');
  await page.getByTestId('laser-export').click();
  const d = await dl;
  expect(d.suggestedFilename()).toBe('Sweden_XS_150X_001_HJ-GC1_laser_generic.csv');
  const lines = readFileSync(await d.path(), 'utf8').trim().split('\n');
  expect(lines[0]).toBe('name,x_stage,y_stage,x_um,y_um,diameter_um,kind,grain,sample,x2_stage,y2_stage');
  expect(lines).toHaveLength(5);
  const row = lines.find((l) => l.startsWith(spots[3].name + ','))!.split(',');
  const [tx, ty] = stage(spots[3].x_um, spots[3].y_um);
  expect(Math.abs(Number(row[1]) - tx)).toBeLessThan(2e-4);
  expect(Math.abs(Number(row[2]) - ty)).toBeLessThan(2e-4);
  expect(row[6]).toBe('spot');
  expect(row[8]).toBe('HJ-GC1');

  // The spot panel shows the laser position of the selected spot.
  await page.getByTestId('tool-select').click();
  const c = (await page.getByTestId('spot').nth(3).locator('circle').first().boundingBox())!;
  await page.mouse.click(c.x + c.width / 2, c.y + c.height / 2);
  await expect(page.getByTestId('panel-laser')).toContainText(`${tx.toFixed(4)}, ${ty.toFixed(4)} mm`);
  // A fourth fiducial 10 µm off in x: least squares, residuals, the warning.
  await page.getByTestId('add-fiducial').click();
  const [bx, by] = stage(spots[3].x_um, spots[3].y_um);
  await page.getByTestId('fid-x').nth(3).fill((bx + 0.01).toFixed(6));
  await page.getByTestId('fid-x').nth(3).press('Tab');
  await page.getByTestId('fid-y').nth(3).fill(by.toFixed(6));
  await page.getByTestId('fid-y').nth(3).press('Tab');
  await expect(page.getByTestId('laser-fit')).toContainText('over 4 fiducials');
  await expect(page.getByTestId('laser-warn')).toBeVisible();

  // Stored in the project file.
  await expect
    .poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X')!).laser?.areas?.['001_HJ-GC1']?.fiducials?.length))
    .toBe(4);
  const reg = await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.project.Sweden_XS_150X')!).laser.areas['001_HJ-GC1']);
  expect(reg).toMatchObject({ units: 'mm', adapter: 'generic' });
  expect(reg.fit.warn).toBe(true);
  expect(reg.fiducials[0].spot_id).toBe(spots[0].id);

  // Fewer than three complete fiducials: no fit, export off.
  await page.getByRole('button', { name: 'Remove fiducial 4' }).click();
  await page.getByRole('button', { name: 'Remove fiducial 3' }).click();
  await expect(page.getByTestId('laser-problem')).toContainText('2 of at least 3');
  await expect(page.getByTestId('laser-export')).toBeDisabled();
});
