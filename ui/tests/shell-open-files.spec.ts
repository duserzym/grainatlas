import { expect, test, type Page } from '@playwright/test';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

// UPb Studio hands grainatlas the files dropped on its window (bridge protocol v2,
// `open-files`). The web shell posts { tag: 'lab-bridge', v: 2, push: 'open-files' }
// to the module iframe; the bridge asks with an `open-files` request and the reply
// is an OpenFilesTransfer ({ path, file: File } entries). This page plays the shell.
const FIXTURE = fileURLToPath(new URL('../../tests/fixtures/tm4000/Sweden_XS_150X', import.meta.url));
const AREA = join(FIXTURE, '001_HJ-GC1');
const FRAME = 'iframe[name="lab-shell:images"]';

interface Entry { path: string; b64: string; type: string }

function walk(dir: string, root = dir): Entry[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return walk(p, root);
    const type = /\.jpe?g$/i.test(n) ? 'image/jpeg' : /\.png$/i.test(n) ? 'image/png' : '';
    return [{ path: relative(root, p).split('\\').join('/'), b64: readFileSync(p).toString('base64'), type }];
  });
}

// The shell's side of the wire: answers before the iframe loads, as createFrameHost does.
const HOST = `<script>
  window.offer = null;
  window.taken = [];
  window.addEventListener('message', (ev) => {
    const d = ev.data;
    if (!d || d.tag !== 'lab-bridge' || typeof d.id !== 'string' || 'ok' in d || d.op === 'notify') return;
    let result = null;
    if (d.op === 'open-files') {
      result = window.offer;
      if (window.offer && d.take !== false) {
        window.offer = null;
        window.taken.push(Date.now() - window.pushedAt);
      }
    }
    ev.source.postMessage({ tag: 'lab-bridge', v: 2, id: d.id, ok: true, result }, '*');
  });
</script>`;

async function shell(page: Page, baseURL: string) {
  await page.goto('./');
  await page.setContent(`${HOST}<iframe name="lab-shell:images" src="${baseURL}" style="width:1400px;height:860px"></iframe>`);
  await expect(page.frameLocator(FRAME).getByTestId('choose-folder')).toBeVisible();
}

/** Offer files as the web shell does: keep the transfer, then nudge the iframe. */
async function offer(page: Page, kind: string, name: string, files: Entry[]) {
  await page.evaluate(
    ({ kind, name, files }) => {
      const w = window as unknown as { offer: unknown; pushedAt: number };
      const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      w.offer = {
        id: `t-${Date.now()}`,
        kind,
        name,
        files: files.map((f) => ({ path: f.path, file: new File([bytes(f.b64)], f.path.split('/').pop()!, { type: f.type, lastModified: 1730700000000 }) })),
      };
      w.pushedAt = Date.now();
      document.querySelector('iframe')!.contentWindow!.postMessage({ tag: 'lab-bridge', v: 2, push: 'open-files' }, '*');
    },
    { kind, name, files },
  );
}

const taken = (page: Page) => page.evaluate(() => (window as unknown as { taken: number[] }).taken);

test('a package dropped on UPb Studio opens in the Images tab; a second one asks before replacing', async ({ page, baseURL }) => {
  await shell(page, baseURL!);
  const f = page.frameLocator(FRAME);
  const pkg = walk(FIXTURE).filter((e) => !e.path.endsWith('mount-results.json'));
  await offer(page, 'package', 'Sweden_XS_150X', pkg);
  await expect(f.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await expect(f.locator('.pkgname')).toHaveText('Sweden_XS_150X');
  await expect(f.getByTestId('replace-dialog')).toHaveCount(0);
  // Taken at once, well before the shell's three-second fallback.
  expect(await taken(page)).toHaveLength(1);
  expect((await taken(page))[0]).toBeLessThan(3000);

  // A project is open: the next set asks first, and Cancel keeps the open one.
  await offer(page, 'area', '001_HJ-GC1', walk(AREA));
  await expect(f.getByTestId('replace-dialog')).toBeVisible();
  await expect(f.getByTestId('replace-dialog')).toContainText('Open 001_HJ-GC1?');
  await expect(f.getByTestId('replace-dialog')).toContainText('Sweden_XS_150X');
  // Cancel is the default: Enter (or a stray keystroke) keeps the open project.
  await expect(f.getByTestId('replace-cancel')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(f.getByTestId('replace-dialog')).toHaveCount(0);
  await expect(f.locator('.pkgname')).toHaveText('Sweden_XS_150X');
  expect(await taken(page)).toHaveLength(2);
});

test('three loose images from UPb Studio ask for the scale once, and Open replaces them with a package', async ({ page, baseURL }) => {
  await shell(page, baseURL!);
  const f = page.frameLocator(FRAME);
  const top = walk(AREA).filter((e) => /_000[012]\.jpg$/.test(e.path));
  expect(top).toHaveLength(3);
  await offer(page, 'images', '001_HJ-GC1', top);
  await expect(f.getByTestId('scale-dialog')).toBeVisible();
  await expect(f.getByTestId('scale-dialog')).toContainText('Scale for 3 images');
  await f.getByTestId('um-per-px').fill('0.6614583');
  await f.getByTestId('scale-open').click();
  for (const i of [0, 1, 2]) await expect(f.getByTestId(`area-001_HJ-GC1_000${i}`)).toBeVisible();
  await expect(f.getByTestId('open-kind')).toContainText('0.6614583 µm per pixel');

  // The package replaces the loose images after Open in the dialog.
  await offer(page, 'package', 'Sweden_XS_150X', walk(FIXTURE));
  await expect(f.getByTestId('replace-dialog')).toBeVisible();
  await f.getByTestId('replace-open').click();
  await expect(f.getByTestId('replace-dialog')).toHaveCount(0);
  await expect(f.getByTestId('area-001_HJ-GC1')).toBeVisible();
  await expect(f.locator('.pkgname')).toHaveText('Sweden_XS_150X');
  await expect(f.getByTestId('area-001_HJ-GC1_0000')).toHaveCount(0);
});
