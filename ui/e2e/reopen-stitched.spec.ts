// Synthetic raw package only. The supervisor runs this spec (the task opens no ports).
import { test, expect } from './mock-session';
import type { Page } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { deflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Project, Grain, Spot } from '../src/lib/core/types';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const name = 'Synthetic reopen';
const directory = path.join(root, 'target/reopen-package', name);
const key = `grainatlas.project.${name}`;
type Saved = Project & { grainatlas_ui: { grains: Record<string, Grain[]>; spots: Spot[] } };
declare global { interface Window { __reopenStages: string[] } }

function crc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function png(offset: number): Buffer {
  const w = 256, h = 192, rows = Buffer.alloc((w + 1) * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const gx = x + offset;
    const grain = [65, 155, 245, 335].some(cx => ((gx - cx) / 23) ** 2 + ((y - 96) / 32) ** 2 < 1);
    rows[y * (w + 1) + x + 1] = (grain ? 185 : 28) + ((gx * 31 + y * 17 + gx * y * 3) % 23);
  }
  const header = Buffer.alloc(13); header.writeUInt32BE(w); header.writeUInt32BE(h, 4); header[8] = 8;
  const chunk = (type: string, bytes: Buffer) => {
    const payload = Buffer.concat([Buffer.from(type), bytes]);
    const length = Buffer.alloc(4), crc = Buffer.alloc(4);
    length.writeUInt32BE(bytes.length); crc.writeUInt32BE(crc32(payload));
    return Buffer.concat([length, payload, crc]);
  };
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))]);
}

test.beforeAll(async () => {
  const area = path.join(directory, 'a');
  await mkdir(area, { recursive: true });
  await writeFile(path.join(area, 'left.png'), png(0));
  await writeFile(path.join(area, 'right.png'), png(192));
  await writeFile(path.join(area, 'positions.csv'), 'file,x_um,y_um,pixel_size_nm\nleft.png,0,0,1000\nright.png,192,0,1000\n');
});

async function saved(page: Page): Promise<Saved | null> {
  return page.evaluate(async key => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open('grainatlas', 1);
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
    try {
      const value = await new Promise<string | undefined>((resolve, reject) => {
        const request = db.transaction('items', 'readonly').objectStore('items').get(key);
        request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
      });
      return value ? JSON.parse(value) : null;
    } finally { db.close(); }
  }, key);
}

async function mosaicVisible(page: Page) {
  await expect(page.getByTestId('canvas-um-space')).toBeVisible();
  // Inspect the bitmap canvas, independent of the SVG grains and spots over it.
  await expect.poll(() => page.locator('.canvas .viewer > canvas').evaluate((canvas: HTMLCanvasElement) => {
    const small = document.createElement('canvas'); small.width = small.height = 48;
    const context = small.getContext('2d')!; context.drawImage(canvas, 0, 0, 48, 48);
    const data = context.getImageData(0, 0, 48, 48).data;
    return new Set(Array.from({ length: 48 * 48 }, (_, i) => data[i * 4])).size;
  })).toBeGreaterThan(8);
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 1280, height: 800 }]) {
  test(`reopen stitched raw package with spots and pending settings at ${viewport.width} x ${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.addInitScript(() => {
      window.__reopenStages = [];
      const NativeWorker = window.Worker;
      // Observe real wasm pipeline progress without changing the core or application state.
      window.Worker = class extends NativeWorker {
        constructor(url: string | URL, options?: WorkerOptions) {
          super(url, options);
          this.addEventListener('message', event => {
            if (event.data.progress?.stage) window.__reopenStages.push(event.data.progress.stage);
          });
        }
      };
    });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Screens' });
    await page.getByTestId('package-input').setInputFiles(directory);
    await expect(page.getByTestId('stitch')).toHaveText('Stitch');
    await page.getByTestId('stitch').click();
    await expect.poll(async () => (await saved(page))?.grainatlas_ui.grains.a?.filter(g => !g.artifact).length ?? 0).toBeGreaterThan(1);
    await expect(page.getByTestId('busy')).toHaveCount(0);
    await page.getByTestId('to-canvas').click();
    await mosaicVisible(page);
    const initial = (await saved(page))!;
    const grains = initial.grainatlas_ui.grains.a.filter(g => !g.artifact).slice(0, 2);
    await page.getByTestId('tool-spot').click();
    for (const grain of grains) {
      const point = await page.getByTestId('canvas-um-space').evaluate((g: SVGGElement, [x, y]) => {
        const m = g.getScreenCTM(); if (!m) throw new Error('Missing micrometre transform');
        const p = new DOMPoint(x, y).matrixTransform(m); return { x: p.x, y: p.y };
      }, grain.centroid_um);
      await page.mouse.click(point.x, point.y);
    }
    await expect.poll(async () => (await saved(page))?.grainatlas_ui.spots.length ?? 0).toBe(2);
    const placed = (await saved(page))!;
    expect(await page.evaluate(() => window.__reopenStages)).toContain('stitch');
    await nav.getByRole('button', { name: 'Open', exact: true }).click();
    await page.reload();
    // Browser files must be reselected after reload; the saved recipe survives in IndexedDB.
    await page.getByTestId('package-input').setInputFiles(directory);
    await mosaicVisible(page);
    await expect(nav.getByRole('button', { name: 'Canvas', exact: true })).toBeEnabled();
    await expect(page.getByTestId('spot')).toHaveCount(2);
    const centres = await page.getByTestId('spot').locator('circle').evaluateAll(circles =>
      circles.map(c => [Number(c.getAttribute('cx')), Number(c.getAttribute('cy'))]));
    expect(centres).toEqual(expect.arrayContaining(placed.grainatlas_ui.spots.map(s => [s.x_um, s.y_um])));
    expect(await page.evaluate(() => window.__reopenStages)).toContain('pyramid');
    expect(await page.evaluate(() => window.__reopenStages)).not.toContain('stitch');
    expect(await page.evaluate(() => window.__reopenStages)).not.toContain('correlate');
    const reopened = (await saved(page))!;
    expect(reopened.areas[0].placement).toEqual(placed.areas[0].placement);
    expect(reopened.areas[0].mosaic?.sha256).toBe(placed.areas[0].mosaic?.sha256);
    expect(reopened.grainatlas_ui.grains).toEqual(placed.grainatlas_ui.grains);
    expect(reopened.grainatlas_ui.spots).toEqual(placed.grainatlas_ui.spots);
    await nav.getByRole('button', { name: 'Area', exact: true }).click();
    await expect(page.getByTestId('to-canvas')).toBeEnabled();
    await expect(page.getByTestId('stitch-pending')).toHaveCount(0);
    const setting = page.getByLabel('Min. peak-to-sidelobe');
    await setting.fill(String(placed.areas[0].params.stitch.min_psr + 1)); await setting.blur();
    await expect(page.getByTestId('stitch-pending')).toHaveText('(Re-stitch to apply)');
    await expect(page.getByTestId('to-canvas')).toBeEnabled();
    await page.getByTestId('to-canvas').click();
    await mosaicVisible(page);
    await expect(page.getByTestId('spot')).toHaveCount(2);
    // Persist and reopen the pending setting too: it must never become applied implicitly.
    await expect.poll(async () => (await saved(page))?.grainatlas_ui).toHaveProperty('stitch_settings');
    await page.reload(); await page.getByTestId('package-input').setInputFiles(directory);
    await mosaicVisible(page);
    await expect(page.getByTestId('spot')).toHaveCount(2);
    await nav.getByRole('button', { name: 'Area', exact: true }).click();
    await expect(page.getByTestId('stitch-pending')).toBeVisible();
    await expect(setting).toHaveValue(String(placed.areas[0].params.stitch.min_psr + 1));
    expect((await saved(page))!.grainatlas_ui.spots).toEqual(placed.grainatlas_ui.spots);
  });
}
