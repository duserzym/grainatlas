// Invented large bundle only. The supervisor runs this with the preview server.
import { test, expect } from './mock-session';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { generated, mockLargeServer, openEdit, root } from './support/perf';
import { mosaicFilename } from '../src/lib/mosaic-export';
import type { Page } from '@playwright/test';

test.use({ viewport: { width: 1280, height: 800 } });
test.beforeAll(() => {
  test.setTimeout(600_000);
  if (!existsSync(path.join(generated, 'atlas.json'))) execFileSync('cargo', ['run', '-p', 'grainatlas-core', '--example', 'large_bundle'], { cwd: root, stdio: 'pipe', timeout: 600_000 });
});
async function openDialog(page: Page) {
  await mockLargeServer(page); await openEdit(page);
  await page.getByRole('combobox', { name: 'Area', exact: true }).selectOption('a01');
  for (const dismiss of await page.getByRole('button', { name: 'Dismiss', exact: true }).all()) await dismiss.click();
  await page.locator('.side .tabs').getByRole('tab', { name: /^Grains / }).click();
  await page.getByRole('button', { name: 'Export mosaic...' }).click();
  const dialog = page.getByTestId('mosaic-export-dialog');
  await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled({ timeout: 120_000 });
  return dialog;
}
test('fallback downloads full-resolution TIFF, JSON and world file together', async ({ page }) => {
  test.setTimeout(180_000);
  await page.addInitScript(() => {
    Object.defineProperty(window, 'showDirectoryPicker', { configurable: true, value: undefined });
    Object.defineProperty(window, 'showSaveFilePicker', { configurable: true, value: undefined });
  });
  const dialog = await openDialog(page);
  const box = await dialog.boundingBox(); expect(box && box.y >= 0 && box.y + box.height <= 800 && box.x >= 0 && box.x + box.width <= 1280).toBeTruthy();
  await expect(dialog.getByText(/512 MiB/).first()).toBeVisible();
  await page.screenshot({ path: path.join(root, 'target/e2e-results/mosaic-export-dialog.png') });
  const download = page.waitForEvent('download');
  await dialog.getByRole('button', { name: 'Export', exact: true }).click();
  const file = await (await download).path();
  // Independent ZIP decoder validates descriptors/CRC. Inspect the TIFF IFD
  // without an image-sized decode and compare dimensions to its JSON sidecar.
  const result = JSON.parse(execFileSync('python3', ['-c', `
import json,struct,sys,zipfile
z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None
names=z.namelist(); tif=next(n for n in names if n.endswith('.tiff')); stem=tif[:-5]
b=z.read(tif); assert b[:4]==b'II*\\x00'
o=struct.unpack_from('<I',b,4)[0]; n=struct.unpack_from('<H',b,o)[0]
tags={struct.unpack_from('<H',b,o+2+i*12)[0]:struct.unpack_from('<I',b,o+10+i*12)[0] for i in range(n)}
m=json.loads(z.read(stem+'.json')); assert m['image_size']==[tags[256],tags[257]]
assert tags[322]==512 and tags[323]==512 and tags[259]==1
assert m['bit_depth']==8 and m['values_mode']=='original'
assert len(z.read(stem+'.tfw').decode().splitlines())==6
print(json.dumps({'names':names,'width':tags[256],'height':tags[257]}))
`, file], { encoding: 'utf8' }));
  expect(result.names).toHaveLength(3); expect(result.width).toBeGreaterThan(512); expect(result.height).toBeGreaterThan(512);
  await expect(dialog).not.toBeVisible();
});
async function mockDirectory(page: Page) {
  await page.addInitScript(() => {
    type FileRecord = { bytes: number; writes: number; head: number[]; text: string; closed: boolean };
    const state = { picks: 0, files: {} as Record<string, FileRecord>, removed: [] as string[], maxChunk: 0, slow: false, active: 0, maxActive: 0 };
    (window as unknown as { mosaicTest: typeof state }).mosaicTest = state;
    const dir = (prefix: string): object => ({
      getDirectoryHandle: async (name: string) => dir(`${prefix}${name}/`),
      removeEntry: async (name: string) => { state.removed.push(name); for (const key of Object.keys(state.files)) if (key.startsWith(`${prefix}${name}/`)) delete state.files[key]; },
      getFileHandle: async (name: string) => ({ createWritable: async () => {
        const file = state.files[`${prefix}${name}`] = { bytes: 0, writes: 0, head: [], text: '', closed: false };
        return {
          write: async (b: Uint8Array) => {
            state.active++; state.maxActive = Math.max(state.maxActive, state.active);
            try {
              if (state.slow) await new Promise(r => setTimeout(r, 30));
              file.bytes += b.length; file.writes++; state.maxChunk = Math.max(state.maxChunk, b.length);
              if (!file.head.length) file.head = [...b.slice(0, 16)];
              if (name.endsWith('.json') || /\.(tfw|pgw|jgw)$/.test(name)) file.text += new TextDecoder().decode(b);
            } finally { state.active--; }
          },
          close: async () => { file.closed = true; }, abort: async () => {},
        };
      } }),
    });
    Object.defineProperty(window, 'showDirectoryPicker', { configurable: true, value: async () => { state.picks++; return dir(''); } });
  });
}
test('folder stream uses backpressure, cancels cleanly and preserves published pixels', async ({ page }) => {
  test.setTimeout(180_000); await mockDirectory(page);
  const dialog = await openDialog(page);
  await page.evaluate(() => { (window as unknown as { mosaicTest: { slow: boolean } }).mosaicTest.slow = true; });
  await dialog.getByRole('button', { name: 'Export', exact: true }).click();
  await expect(dialog.getByRole('progressbar')).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
  const cancelled = await page.evaluate(() => (window as unknown as { mosaicTest: { files: object; removed: string[] } }).mosaicTest);
  expect(Object.keys(cancelled.files)).toHaveLength(0); expect(cancelled.removed).toHaveLength(1);
  await page.evaluate(() => { (window as unknown as { mosaicTest: { slow: boolean } }).mosaicTest.slow = false; });
  await dialog.getByRole('combobox', { name: 'Pixel values' }).selectOption('displayed');
  await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Export', exact: true }).click();
  await expect(dialog).not.toBeVisible({ timeout: 120_000 });
  const state = await page.evaluate(() => (window as unknown as { mosaicTest: { maxChunk: number; maxActive: number; files: Record<string, { closed: boolean; writes: number; head: number[]; text: string }> } }).mosaicTest);
  expect(state.maxActive).toBe(1); expect(state.maxChunk).toBeLessThanOrEqual(512 * 512 * 2);
  const entries = Object.entries(state.files); expect(entries).toHaveLength(3); expect(entries.every(([, f]) => f.closed)).toBeTruthy();
  const tiff = entries.find(([name]) => name.endsWith('.tiff'))![1]; expect(tiff.head.slice(0, 4)).toEqual([73,73,42,0]); expect(tiff.writes).toBeGreaterThan(2);
  const meta = JSON.parse(entries.find(([name]) => name.endsWith('.json'))![1].text); expect(meta.values_mode).toBe('displayed'); expect(meta.bit_depth).toBe(8);
});

test('PNG chosen areas pick one folder and name every file automatically', async ({ page }) => {
  test.setTimeout(240_000); await mockDirectory(page); const dialog = await openDialog(page);
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).selectOption('png');
  await dialog.getByRole('combobox', { name: 'Bit depth' }).selectOption('16');
  await dialog.getByRole('combobox', { name: 'Areas', exact: true }).selectOption('chosen');
  const checks = dialog.locator('.areas input'); await checks.nth(0).check(); await checks.nth(1).check();
  await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Export', exact: true }).click(); await expect(dialog).not.toBeVisible({ timeout: 180_000 });
  const state = await page.evaluate(() => (window as unknown as { mosaicTest: { picks: number; files: Record<string, { head: number[]; text: string; closed: boolean }> } }).mosaicTest);
  expect(state.picks).toBe(1); const entries = Object.entries(state.files); expect(entries).toHaveLength(6);
  const project = JSON.parse(readFileSync(path.join(generated, 'project.json'), 'utf8'));
  for (const area of project.areas.slice(0, 2)) {
    const filename = mosaicFilename(project.name, area.name, 'png'); const entry = entries.find(([name]) => name.endsWith(`/${filename}`)); expect(entry).toBeTruthy();
    expect(entry![1].head.slice(0, 8)).toEqual([137,80,78,71,13,10,26,10]); expect(entry![1].closed).toBe(true);
    const stem = filename.slice(0, -4); const meta = JSON.parse(entries.find(([name]) => name.endsWith(`/${stem}.json`))![1].text);
    expect(meta.area_name).toBe(area.name); expect(meta.bit_depth).toBe(16); expect(meta.values_mode).toBe('original');
    expect(entries.some(([name]) => name.endsWith(`/${stem}.pgw`))).toBe(true);
  }
});
test('JPG defaults and over-limit preflight offer reduction and splitting', async ({ page }) => {
  // Inflate only the plan reply to exercise the dialog without generating or
  // decoding a 65k image. Rust tests exercise real limit/affine planning.
  await page.addInitScript(() => {
    const Native = window.Worker;
    window.Worker = class extends Native {
      pending = new Map<number, { format?: string; size_choice?: string }>();
      constructor(url: string | URL, options?: WorkerOptions) {
        super(url, options); this.addEventListener('message', event => {
          const option = this.pending.get(event.data.id); if (!option) return; this.pending.delete(event.data.id);
          if (option.format !== 'jpg' || !event.data.ok) return;
          for (const image of event.data.value.images) {
            image.width = option.size_choice === 'downsample' ? 35000 : 70000; image.height = 1000;
            image.downsample = option.size_choice === 'downsample' ? 2 : 1;
            image.limit_reason = option.size_choice === 'full' ? 'JPG exceeds 65,535 px per side. Choose downsampling or split files.' : null;
          }
        });
      }
      postMessage(message: { id: number; op: string; args: { options: string } }, transfer?: Transferable[]) {
        if (message.op === 'mosaicExportPlan') this.pending.set(message.id, JSON.parse(message.args.options));
        super.postMessage(message, transfer ?? []);
      }
    };
  });
  const dialog = await openDialog(page);
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).selectOption('jpg');
  await expect(dialog.getByRole('spinbutton', { name: 'Quality' })).toHaveValue('90');
  await expect(dialog.getByRole('alert')).toContainText('65,535'); await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeDisabled();
  await dialog.getByRole('combobox', { name: 'Image size' }).selectOption('downsample');
  await expect(dialog.getByText(/Downsampled 2×/)).toBeVisible(); await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
  await dialog.getByRole('combobox', { name: 'Image size' }).selectOption('split'); await expect(dialog.getByRole('button', { name: 'Export', exact: true })).toBeEnabled();
});
