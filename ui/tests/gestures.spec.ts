import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

// Every image view uses lib/canvas/gestures.ts: the same events must do the same thing, with
// the same feel, in the Area view, the Canvas and the Layout preview. Wheel events are built
// the way each engine reports them (Chromium, WebKit / WKWebView, Firefox).
const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));

type V = { zoom: number; e: number; f: number };

async function view(page: Page, host: string): Promise<V> {
  return page.locator(host).evaluate((el) => {
    const g = el.querySelector('svg.overlay > g');
    const raw = g ? g.getAttribute('transform')! : (el.querySelector('.content') as HTMLElement).style.transform;
    const [a, b, , , e, f] = raw.replace(/^matrix\(|\)$/g, '').split(/[\s,]+/).map(Number);
    return { zoom: Math.hypot(a, b), e, f };
  });
}

/** Dispatches a wheel event at the host's centre (+ offset), with an engine-specific wheelDeltaY. */
async function wheel(page: Page, host: string, init: WheelEventInit & { wheelDeltaY?: number }, at: [number, number] = [0, 0]) {
  await page.locator(host).evaluate((el, [init, at]) => {
    const r = el.getBoundingClientRect();
    const { wheelDeltaY, ...rest } = init as WheelEventInit & { wheelDeltaY?: number };
    const ev = new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: r.left + r.width / 2 + at[0], clientY: r.top + r.height / 2 + at[1], ...rest });
    if (wheelDeltaY !== undefined) Object.defineProperty(ev, 'wheelDeltaY', { value: wheelDeltaY });
    el.dispatchEvent(ev);
  }, [init, at] as const);
}

/** WebKit trackpad pinch: gesturestart / gesturechange / gestureend. */
async function webkitPinch(page: Page, host: string, scale: number) {
  await page.locator(host).evaluate((el, scale) => {
    const r = el.getBoundingClientRect();
    const mk = (type: string, s: number) => Object.assign(new Event(type, { bubbles: true, cancelable: true }), { scale: s, rotation: 0, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 });
    el.dispatchEvent(mk('gesturestart', 1));
    el.dispatchEvent(mk('gesturechange', scale));
    el.dispatchEvent(mk('gestureend', scale));
  }, scale);
}

async function corner(page: Page, host: string): Promise<[number, number]> {
  const box = (await page.locator(host).boundingBox())!;
  return [box.x + box.width * 0.4, box.y + box.height * 0.6];
}

/** The content point under a screen offset from the host centre stays put while zooming there. */
async function anchored(page: Page, host: string, before: V, after: V, at: [number, number]) {
  const box = (await page.locator(host).boundingBox())!;
  const sx = box.width / 2 + at[0], sy = box.height / 2 + at[1];
  const p0 = [(sx - before.e) / before.zoom, (sy - before.f) / before.zoom];
  const p1 = [(sx - after.e) / after.zoom, (sy - after.f) / after.zoom];
  expect(Math.abs(p0[0] - p1[0]) * after.zoom).toBeLessThan(0.5);
  expect(Math.abs(p0[1] - p1[1]) * after.zoom).toBeLessThan(0.5);
}

/** Waits until the view stops changing (first layout and resize done). */
async function steady(page: Page, host: string) {
  let last = '';
  await expect.poll(async () => {
    const v = JSON.stringify(await view(page, host));
    const same = v === last;
    last = v;
    return same;
  }, { intervals: [250] }).toBe(true);
}

async function exercise(page: Page, host: string): Promise<Record<string, number>> {
  const ratios: Record<string, number> = {};
  await steady(page, host);
  await page.mouse.dblclick(...(await corner(page, host)));
  const fit = await view(page, host);
  const at: [number, number] = [40, -25];

  // Mouse wheel notches zoom about the cursor: Chromium (deltaY 100), WebKit (~4), Firefox (lines).
  let a = await view(page, host);
  await wheel(page, host, { deltaY: -100, wheelDeltaY: 120 }, at);
  let b = await view(page, host);
  expect(b.zoom).toBeGreaterThan(a.zoom);
  await anchored(page, host, a, b, at);
  ratios.chromiumMouse = b.zoom / a.zoom;

  a = b;
  await wheel(page, host, { deltaY: 4.000244140625, wheelDeltaY: -120 }, at);
  b = await view(page, host);
  expect(b.zoom).toBeLessThan(a.zoom);
  await anchored(page, host, a, b, at);
  ratios.webkitMouse = b.zoom / a.zoom;

  a = b;
  await wheel(page, host, { deltaY: 3, deltaMode: 1 }, at);
  b = await view(page, host);
  expect(b.zoom).toBeLessThan(a.zoom);
  ratios.firefoxMouse = b.zoom / a.zoom;

  // Trackpad pinch: Chromium as ctrl+wheel, WebKit as gesture events.
  a = b;
  await wheel(page, host, { deltaY: -8, ctrlKey: true }, at);
  b = await view(page, host);
  expect(b.zoom).toBeGreaterThan(a.zoom);
  await anchored(page, host, a, b, at);
  ratios.chromiumPinch = b.zoom / a.zoom;
  a = b;
  await webkitPinch(page, host, 1.5);
  b = await view(page, host);
  expect(b.zoom / a.zoom).toBeCloseTo(1.5, 5);

  // Trackpad two-finger scroll pans (wheelDeltaY = -3 × deltaY), no zoom.
  a = b;
  await wheel(page, host, { deltaX: 12, deltaY: 30, wheelDeltaY: -90 });
  b = await view(page, host);
  expect(b.zoom).toBe(a.zoom);
  expect(b.e - a.e).toBeCloseTo(-12, 3);
  expect(b.f - a.f).toBeCloseTo(-30, 3);

  // Drag pans.
  const [x0, y0] = await corner(page, host);
  a = await view(page, host);
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  await page.mouse.move(x0 - 70, y0 - 20, { steps: 5 });
  await page.mouse.up();
  b = await view(page, host);
  expect(b.e - a.e).toBeCloseTo(-70, 0);
  expect(b.f - a.f).toBeCloseTo(-20, 0);

  // Arrow keys nudge the view under the pointer; + zooms.
  a = b;
  await page.keyboard.press('ArrowLeft');
  b = await view(page, host);
  expect(b.e - a.e).toBeCloseTo(60, 3);
  await page.keyboard.press('Shift+ArrowUp');
  const c = await view(page, host);
  expect(c.f - b.f).toBeCloseTo(240, 3);
  await page.keyboard.press('+');
  expect((await view(page, host)).zoom / c.zoom).toBeCloseTo(1.5, 5);

  // Double-click on empty space fits again.
  await page.mouse.dblclick(x0, y0);
  const d = await view(page, host);
  expect(d.zoom).toBeCloseTo(fit.zoom, 6);
  expect(d.e).toBeCloseTo(fit.e, 3);
  return ratios;
}

test('the Area view, the Canvas and the Layout preview pan and zoom the same way', async ({ page }) => {
  await page.goto('./');
  await page.getByTestId('package-input').setInputFiles(FIXTURE);
  // Before stitching (tile thumbnails) the Area view already has the gestures.
  const area = '[data-testid=area-view] [data-view]';
  await expect(page.locator(area)).toBeVisible();
  const r0 = await exercise(page, area);

  await page.getByTestId('stitch').click();
  await expect(page.getByTestId('rms')).toBeVisible();
  const r1 = await exercise(page, area);

  await page.getByTestId('to-canvas').click();
  const canvas = '.canvas [data-view]';
  const r2 = await exercise(page, canvas);

  await page.getByRole('button', { name: 'Layout' }).click();
  await page.getByTestId('style-layout').check();
  await page.getByTestId('layout').click();
  const layout = '[data-testid=layout-view] [data-view]';
  await expect(page.locator(layout)).toBeVisible();
  const r3 = await exercise(page, layout);

  // Same feel: identical zoom per notch / pinch step everywhere.
  for (const r of [r1, r2, r3]) for (const k of Object.keys(r0)) expect(r[k]).toBeCloseTo(r0[k], 5);
});
