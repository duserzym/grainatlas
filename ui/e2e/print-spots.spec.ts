import type { Page } from '@playwright/test';
import { test, expect } from './mock-session';
import { mockServer, problems, server, projectId, mountId } from './print-fixture';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));
const spots = Array.from({ length: 12 }, (_, i) => ({
  id: `70000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`, area_id: 'a',
  name: i === 0 ? 'MNT-A_12 <&" Ω' : `Z-${String(i + 1).padStart(3, '0')}`,
  sample: 'MNT-A', grain_id: null, grain_number: null, n: i + 1,
  x_um: 174 + i % 4 * 32, y_um: 174 + Math.floor(i / 4) * 32,
  diameter_um: 20, kind: i === 10 ? 'line' : i === 11 ? 'raster' : 'spot',
  x2_um: i >= 10 ? 174 + i % 4 * 32 + 10 : null,
  y2_um: i >= 10 ? 174 + Math.floor(i / 4) * 32 + 10 : null,
  comment: '', created: '', created_by: '',
}));

test.beforeAll(() => execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'print_edit_bundle'], { cwd: root, stdio: 'pipe' }));
async function open(page: Page, editable = true, fixture: { syncOnSave?: boolean; spots?: unknown[] } = {}) {
  const unexpected = await mockServer(page, { ...fixture, spots });
  await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
  if (editable) await page.getByTestId('edit-copy').click();
  await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
  await page.getByTestId('style-spots').check();
  if (editable) {
    await page.getByTestId('sheet-scale').fill('4');
    await page.getByLabel('denoise (NLM)').uncheck();
    await page.getByLabel('sharpen', { exact: true }).uncheck();
  }
  return unexpected;
}
async function ready(page: Page) {
  await expect(page.getByTestId('busy')).toHaveCount(0);
  await expect(page.getByTestId('export-registration')).toBeEnabled();
  await expect(page.getByTestId('export-registration')).not.toHaveAttribute('title', 'Wait for the strip preview');
}
async function registration(page: Page) {
  await ready(page); const download = page.waitForEvent('download'); await page.getByTestId('export-registration').click();
  return JSON.parse(await readFile((await (await download).path())!, 'utf8'));
}
async function expectLabelCounts(page: Page) {
  const reports = (await registration(page)).spot_labels;
  const overlaps = reports.reduce((sum: number, p: any) => sum + p.counts.still_overlapping, 0);
  const ambiguous = reports.reduce((sum: number, p: any) => sum + (p.counts.ambiguous ?? 0), 0);
  const ticks = reports.reduce((sum: number, p: any) => sum + (p.counts.with_ticks ?? 0), 0);
  const leaders = reports.reduce((sum: number, p: any) => sum + p.counts.with_leaders, 0);
  const pinnedOverlaps = reports.reduce((sum: number, p: any) => sum + (p.counts.pinned_overlapping ?? 0), 0);
  const omitted = reports.reduce((sum: number, p: any) => sum + (p.counts.context_omitted ?? 0), 0);
  expect(overlaps).toBe(0);
  const text = `${overlaps} spot labels still overlap; ${ambiguous} ambiguous; ${ticks} ticks; ${leaders} leaders`
    + (pinnedOverlaps ? `; ${pinnedOverlaps} pinned label${pinnedOverlaps === 1 ? ' overlaps' : 's overlap'}` : '')
    + (omitted ? `; ${omitted} repeated context labels omitted` : '')
    + '.'
    + (overlaps || ambiguous || pinnedOverlaps ? ' Show page' : '');
  await expect(page.getByTestId('spot-label-overlaps')).toHaveText(text);
}
async function svg(page: Page) {
  return page.getByTestId('page').evaluate(async image => {
    const text = await (await fetch((image as HTMLImageElement).src)).text();
    const doc = new DOMParser().parseFromString(text, 'image/svg+xml');
    return { markers: [...doc.querySelectorAll('[data-spot-marker]')].map(n => n.getAttribute('data-spot-marker')),
      labels: [...doc.querySelectorAll('[data-spot-label]')].map(n => n.textContent) };
  });
}

/** The image preview has its own font environment. Measure an inline copy only
 * after loading its embedded font, rather than relying on the app's fonts. */
async function labelFontMetrics(page: Page, boxes: any[] = []) {
  return page.getByTestId('page').evaluate(async (image, boxes) => {
    const source = await (await fetch((image as HTMLImageElement).src)).text();
    const doc = new DOMParser().parseFromString(source, 'image/svg+xml');
    const copy = document.importNode(doc.documentElement, true) as unknown as SVGSVGElement;
    copy.style.position = 'absolute'; copy.style.visibility = 'hidden';
    document.body.append(copy);
    try {
      await document.fonts.load('500 16px "IBM Plex Sans Condensed"');
      await document.fonts.load('500 16px "DejaVu Sans"', 'Ω');
      await document.fonts.ready;
      const labels = [...copy.querySelectorAll<SVGTextElement>('[data-spot-label]')];
      return { faces: source.split('@font-face').length - 1,
        embedded: source.includes('data:font/ttf;base64,'),
        errors: labels.map(text => {
          const plate = text.parentElement?.querySelector('rect');
          const box = boxes.find(l => l.spot_id === text.getAttribute('data-spot-label') && l.occurrence === text.parentElement?.getAttribute('data-occurrence'));
          const left = box ? box.box_mm[0] : Number(plate?.getAttribute('x'));
          const right = box ? box.box_mm[2] : left + Number(plate?.getAttribute('width'));
          const x = text.x.baseVal.getItem(0).value;
          // The renderer centres text with equal insets. Read the actual SVG
          // inset instead of assuming how much padding/halo the plate reserves.
          const expected = right - x - (x - left);
          return Math.abs(text.getComputedTextLength() - expected) / expected;
        }) };
    } finally { copy.remove(); }
  }, boxes);
}

async function localLabelPreferences(page: Page) {
  return page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open('grainatlas', 1);
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
    try {
      const values = await new Promise<string[]>((resolve, reject) => {
        const request = db.transaction('items').objectStore('items').getAll();
        request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
      });
      return values.map(v => JSON.parse(v)).map(v => (v?.project ?? v)?.grainatlas_ui)
        .filter(ui => ui?.print_label_pins && Object.keys(ui.print_label_pins).length)
        .map(ui => ({ pins: ui.print_label_pins, size: ui.print_label_size_mm }));
    } finally { db.close(); }
  });
}
async function saveProjectFile(page: Page) {
  const downloaded = page.waitForEvent('download');
  await page.getByTestId('save-project').click();
  const checks = page.getByTestId('checks-continue');
  const confirm = await Promise.race([downloaded.then(() => false), checks.waitFor({ state: 'visible' }).then(() => true).catch(() => false)]);
  if (confirm) await checks.click();
  return readFile((await (await downloaded).path())!, 'utf8');
}

for (const packing of ['lines', 'packed']) {
  test(`spot sheets ${packing} preserve names, pins and hidden-window printing`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    const fixture = { spots, syncOnSave: false };
    const unexpected = await open(page, true, fixture);
    await page.getByTestId('sheet-packing').selectOption(packing);
    await page.getByTestId('sheets').click(); await ready(page);
    const before = await registration(page);
    const printed = await svg(page);
    await expect(page.getByTestId('spot-label-size')).toHaveValue('Auto');
    await expect(page.getByTestId('spot-label-chosen-size')).toContainText('mm on this page');
    const metrics = await labelFontMetrics(page, before.spot_labels.flatMap((p: any) => p.labels));
    expect(metrics.faces).toBe(2); expect(metrics.embedded).toBe(true);
    expect(metrics.errors.length).toBeGreaterThan(0);
    expect(Math.max(...metrics.errors)).toBeLessThan(0.03);
    expect(printed.markers).toHaveLength(spots.length);
    expect(printed.labels.sort()).toEqual(spots.map(s => s.name).sort());
    await expectLabelCounts(page);
    await page.getByTestId('spot-label-size').fill('3'); await page.getByTestId('spot-label-size').press('Tab');
    await expect.poll(async () => (await registration(page)).spot_labels[0].labels[0].box_mm[3] - before.spot_labels[0].labels[0].box_mm[3]).not.toBe(0);
    await expectLabelCounts(page);
    await page.getByTestId('move-labels').check();
    const handle = page.getByTestId('spot-label-handle').first(), b = await handle.boundingBox();
    await page.mouse.move(b!.x + b!.width / 2, b!.y + b!.height / 2); await page.mouse.down();
    await page.mouse.move(b!.x + b!.width / 2 + 15, b!.y + b!.height / 2 + 12, { steps: 6 }); await page.mouse.up();
    await expect(page.getByTestId('spot-label-handle').first()).toHaveAttribute('data-pin', 'true');
    const pinned = await registration(page);
    await expectLabelCounts(page);
    expect(pinned.spot_labels[0].counts.pinned).toBe(1);
    expect(pinned.spot_labels[0].counts.pinned_overlapping).toBe(0);
    await page.getByTestId('sheets').click(); await ready(page);
    expect((await registration(page)).spot_labels[0].labels.find((l: any) => l.pinned).box_mm).toEqual(pinned.spot_labels[0].labels.find((l: any) => l.pinned).box_mm);
    await page.getByTestId('style-sheets').check(); await ready(page);
    const off = await registration(page); expect(off.spot_labels).toBeUndefined();
    expect((await labelFontMetrics(page)).faces).toBe(0); expect(off.bands).toEqual(pinned.bands);
    await page.getByTestId('style-spots').check(); await ready(page);
    expect((await registration(page)).spot_labels[0].counts.pinned).toBe(1);
    expect(await problems(page)).toEqual([]);
    await page.evaluate(() => { (window as any).__printed = 0; window.open = () => { throw Error('Unexpected popup'); }; window.print = () => { (window as any).__printed++; }; });
    await page.getByRole('button', { name: 'Print…', exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as any).__printed)).toBe(1);
    await expect(page.locator('#ga-print-root')).toBeHidden();
    expect(await page.locator('#ga-print-root [data-spot-marker]').count()).toBeGreaterThanOrEqual(spots.length);
    await page.evaluate(() => window.dispatchEvent(new Event('afterprint')));
    // Database-copy Save persists locally; it does not produce a download.
    await page.getByTestId('save-project').click();
    const pinnedLabel = pinned.spot_labels[0].labels.find((l: any) => l.pinned);
    const preferences = { pins: { [`${pinnedLabel.spot_id}/${pinnedLabel.occurrence}`]: [pinnedLabel.box_mm[0] - pinnedLabel.anchor_mm[0], pinnedLabel.box_mm[1] - pinnedLabel.anchor_mm[1]] }, size: 3 };
    await expect.poll(async () => (await localLabelPreferences(page)).length).toBeGreaterThan(0);
    const storedPreferences = (await localLabelPreferences(page))[0];
    expect(Object.keys(storedPreferences.pins)).toEqual(Object.keys(preferences.pins));
    const key = Object.keys(preferences.pins)[0];
    expect(storedPreferences.size).toBe(3);
    for (let axis = 0; axis < 2; axis++) expect(storedPreferences.pins[key][axis]).toBeCloseTo(preferences.pins[key][axis], 8);
    // Reload against the original server bundle, so this must recover the local copy.
    await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
    await page.getByTestId('edit-copy').click();
    await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
    await page.getByTestId('style-spots').check();
    await expect(page.getByTestId('spot-label-size')).toHaveValue('3');
    await page.getByTestId('sheet-scale').fill('4');
    await page.getByTestId('sheet-packing').selectOption(packing);
    await page.getByLabel('denoise (NLM)').uncheck();
    await page.getByLabel('sharpen', { exact: true }).uncheck();
    await page.getByTestId('sheets').click(); await ready(page);
    expect((await registration(page)).spot_labels[0].labels.find((l: any) => l.pinned).box_mm).toEqual(pinnedLabel.box_mm);
    // The explicit project-file action still downloads a local .gaproj record.
    const saved = JSON.parse(await saveProjectFile(page));
    expect(saved.grainatlas_ui.print_label_pins).toEqual(storedPreferences.pins);
    expect(saved.grainatlas_ui.print_label_size_mm).toBe(3);
    expect(saved.grainatlas_ui.spots.map((s: any) => s.name)).toEqual(spots.map(s => s.name));
    // Remove recovery records before reopening the published server copy.
    // The downloaded file keeps local presentation; the server bundle has none.
    await page.evaluate(async () => {
      for (const key of Object.keys(localStorage)) if (/^grainatlas\.(project\.|copy\.|draft\.|journal\.|drafts$)/.test(key)) localStorage.removeItem(key);
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open('grainatlas', 1);
        request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
      });
      try {
        await new Promise<void>((resolve, reject) => {
          const tx = db.transaction('items', 'readwrite'); tx.objectStore('items').clear();
          tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
        });
      } finally { db.close(); }
    });
    await page.goto(`/?server=${encodeURIComponent(server)}&project=${projectId}&mount=${mountId}`);
    await page.getByTestId('edit-copy').click();
    await page.getByRole('navigation', { name: 'Screens' }).getByRole('button', { name: 'Layout', exact: true }).click();
    await page.getByTestId('style-spots').check();
    await expect(page.getByTestId('spot-label-size')).toHaveValue('Auto');
    await expect(page.getByTestId('reset-labels')).toHaveText('Reset labels (0)');
    await expect(page.getByTestId('reset-labels')).toBeDisabled();
    await page.getByTestId('sheet-scale').fill('4');
    await page.getByTestId('sheet-packing').selectOption(packing);
    await page.getByLabel('denoise (NLM)').uncheck();
    await page.getByLabel('sharpen', { exact: true }).uncheck();
    await page.getByTestId('sheets').click(); await ready(page);
    const reopened = (await registration(page)).spot_labels;
    for (const report of reopened) {
      expect(report.counts.pinned).toBe(0);
      expect(report.labels.some((l: any) => l.pinned)).toBe(false);
    }
    expect((await svg(page)).labels.sort()).toEqual(spots.map(s => s.name).sort());
    await expectLabelCounts(page);
    expect(unexpected).toEqual([]);
  });
}

test('whole-area spots and local pins work on an editable database copy', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const unexpected = await open(page, true);
  await page.getByTestId('sheet-mode').selectOption('whole');
  await page.getByTestId('sheets').click(); await ready(page);
  expect((await svg(page)).labels.sort()).toEqual(spots.map(s => s.name).sort());
  await expectLabelCounts(page);
  await page.getByTestId('move-labels').check();
  const label = page.getByTestId('spot-label-handle').first(), b = await label.boundingBox();
  await page.mouse.move(b!.x + 2, b!.y + 2); await page.mouse.down(); await page.mouse.move(b!.x + 12, b!.y + 12); await page.mouse.up();
  await expect(page.getByTestId('reset-labels')).toHaveText('Reset labels (1)');
  await expectLabelCounts(page);
  expect((await registration(page)).spot_labels[0].counts.pinned).toBe(1);
  expect(await problems(page)).toEqual([]); expect(unexpected).toEqual([]);
});

test('a deliberate pinned marker overlap is reported separately', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await open(page, true);
  await page.getByTestId('spot-label-size').fill('3');
  await page.getByTestId('sheets').click(); await ready(page);
  const before = await registration(page);
  const label = before.spot_labels[0].labels.at(-1);
  await page.getByTestId('move-labels').check();
  const handle = page.getByTestId('spot-label-handle').last(), b = await handle.boundingBox();
  const scale = await page.getByTestId('spot-label-overlay').evaluate(e => (e as SVGSVGElement).getScreenCTM()!.a);
  await page.mouse.move(b!.x + b!.width / 2, b!.y + b!.height / 2); await page.mouse.down();
  await page.mouse.move(b!.x + b!.width / 2 + (label.anchor_mm[0] - label.box_mm[0]) * scale,
    b!.y + b!.height / 2 + (label.anchor_mm[1] - label.box_mm[1]) * scale, { steps: 6 });
  await page.mouse.up();
  await expect(handle).toHaveAttribute('data-pin', 'true');
  await expectLabelCounts(page);
  const pinned = (await registration(page)).spot_labels[0];
  expect(pinned.counts.pinned).toBe(1);
  expect(pinned.counts.pinned_overlapping).toBe(1);
  await expect(page.getByTestId('spot-label-overlaps')).toContainText('1 pinned label overlaps');
});
