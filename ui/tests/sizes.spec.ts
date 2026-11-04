import { expect, test, type Page } from '@playwright/test';

// ux-conventions rule 7: every screen fits a 14-inch laptop (1280 x 800) as well as 1440 x 900.
// Nothing may stick out of the window or be cut off by a panel that does not scroll; content in
// a scrolling area counts as reachable. Panel geometry is remembered and Reset layout restores it.

/** Problems on the current screen: page scroll, panels outside the window, clipped content. */
function problems(page: Page) {
  return page.evaluate(() => {
    const W = innerWidth, H = innerHeight, out: string[] = [];
    const name = (e: Element) => `${e.tagName.toLowerCase()}.${[...e.classList].filter((c) => !c.startsWith('svelte-')).join('.')}${e.getAttribute('data-testid') ? '#' + e.getAttribute('data-testid') : ''} "${(e.textContent || '').trim().slice(0, 24)}"`;
    const inScroller = (e: Element) => {
      for (let a = e.parentElement; a; a = a.parentElement) {
        const o = getComputedStyle(a).overflowY;
        if ((o === 'auto' || o === 'scroll') && a.scrollHeight > a.clientHeight + 1) return true;
      }
      return false;
    };
    const hidden = (e: Element) => !!e.closest('details:not([open])') && !e.closest('summary') || !(e as HTMLElement).checkVisibility?.();
    const de = document.documentElement;
    if (de.scrollWidth > W + 1 || de.scrollHeight > H + 1) out.push(`page scrolls (${de.scrollWidth}x${de.scrollHeight})`);
    for (const e of document.querySelectorAll('.glass, aside, section, header, nav, main')) {
      const r = e.getBoundingClientRect();
      if (!r.width || hidden(e) || inScroller(e)) continue;
      if (r.right > W + 1 || r.bottom > H + 1 || r.left < -1 || r.top < -1) out.push(`outside the window: ${name(e)}`);
      const cs = getComputedStyle(e);
      if (cs.overflowY === 'hidden' && e.scrollHeight > e.clientHeight + 2) out.push(`cut off at the bottom: ${name(e)}`);
    }
    for (const e of document.querySelectorAll('button, input, select, textarea')) {
      const r = e.getBoundingClientRect();
      if (!r.width || !r.height || hidden(e) || inScroller(e)) continue;
      if (r.right > W + 1 || r.bottom > H + 1) out.push(`control out of reach: ${name(e)}`);
    }
    return out;
  });
}

for (const [w, h] of [
  [1440, 900],
  [1280, 800],
] as const) {
  test(`every screen fits ${w} x ${h}`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto('./');
    expect(await problems(page), 'Open').toEqual([]);
    await page.getByTestId('example').click();
    await page.getByTestId('stitch').click();
    await expect(page.getByTestId('rms')).toBeVisible();
    expect(await problems(page), 'Area').toEqual([]);
    await page.getByTestId('to-canvas').click();
    const c = (await page.locator('canvas').first().boundingBox())!;
    await page.getByTestId('tool-spot').click();
    await page.keyboard.down('Alt');
    await page.mouse.click(c.x + c.width * 0.45, c.y + c.height * 0.45);
    await page.keyboard.up('Alt');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.locator('.layers summary').click(); // Layers open: the longest side panel
    for (const t of ['Spots', 'Grains', 'Overlay', 'Laser', 'laicpmsdb']) {
      await page.locator('.side .tabs button', { hasText: t }).first().click();
      expect(await problems(page), `Canvas, ${t} tab`).toEqual([]);
    }
    await page.getByTestId('side-collapse').click();
    expect(await problems(page), 'Canvas, side panel folded').toEqual([]);
    await page.getByTestId('side-expand').click();
    await page.getByRole('button', { name: 'Layout' }).click();
    await page.getByTestId('layout').click();
    await expect(page.getByTestId('page').first()).toBeVisible();
    expect(await problems(page), 'Layout').toEqual([]);
    await page.getByRole('button', { name: 'Settings' }).click();
    expect(await problems(page), 'Settings').toEqual([]);
  });
}

test('panel geometry is remembered, kept inside a smaller window, and Reset layout restores it', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('./');
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  const c = (await page.locator('canvas').first().boundingBox())!;
  await page.getByTestId('tool-spot').click();
  await page.keyboard.down('Alt');
  await page.mouse.click(c.x + c.width * 0.45, c.y + c.height * 0.45);
  await page.keyboard.up('Alt');
  const panel = page.getByRole('dialog');
  const header = panel.locator('header');
  // Drag the spot panel far right and down; fold the side panel.
  const hb = (await header.boundingBox())!;
  await page.mouse.move(hb.x + 40, hb.y + 10);
  await page.mouse.down();
  await page.mouse.move(hb.x + 800, hb.y + 300, { steps: 5 });
  await page.mouse.up();
  const moved = (await panel.boundingBox())!;
  expect(moved.x).toBeGreaterThan(700);
  await page.getByTestId('side-collapse').click();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('grainatlas.layout')!))).toMatchObject({ sideCollapsed: true });

  // A smaller window: the panel is pulled back inside, whole.
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect.poll(async () => (await panel.boundingBox())!.x + (await panel.boundingBox())!.width).toBeLessThanOrEqual(1280);
  const inside = (await panel.boundingBox())!;
  expect(inside.y + 48).toBeLessThanOrEqual(800);

  // Remembered across a reload (the project reopens from this browser's storage).
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.reload();
  await page.getByTestId('example').click();
  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  await page.getByTestId('to-canvas').click();
  await expect(page.getByTestId('side-expand')).toBeVisible();
  await page.getByTestId('side-expand').click();
  await page.locator('.list tbody tr').first().click();
  await expect(panel).toBeVisible();
  expect((await panel.boundingBox())!.x).toBeGreaterThan(700);

  // Reset layout: defaults back.
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByTestId('reset-layout').click();
  expect(await page.evaluate(() => localStorage.getItem('grainatlas.layout'))).toBeNull();
  await page.getByRole('button', { name: 'Canvas' }).click();
  await expect(page.getByTestId('side-collapse')).toBeVisible();
});
