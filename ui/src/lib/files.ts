import { canWrite } from './presentation';
// Getting packages in (input, drag and drop) and files out (downloads). What
// comes in is a package folder, one area folder, or loose files; the core
// decides which (and asks for a scale when images have no calibration).

import { isDesktop, type PackageFile } from './core/core';

/**
 * Files from an <input type=file> (a folder with webkitdirectory, or chosen
 * files): paths relative to the chosen folder; loose files are named after
 * their common stem.
 */
export function fromInput(list: FileList): { name: string; files: PackageFile[] } {
  const files: PackageFile[] = [];
  let name = '';
  for (const f of Array.from(list)) {
    const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
    const parts = rel.split('/');
    if (parts.length > 1) name = parts[0];
    const path = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
    if (keep(path)) files.push({ path, file: f });
  }
  return { name: name || commonStem(files.map((f) => f.path)) || 'images', files };
}

/** Common stem of file names without a half-cut running number or trailing separators (core `loose::set_name`). */
export function commonStem(names: string[]): string {
  const stems = names.map((n) => (n.split('/').pop() ?? n).replace(/\.[^.]*$/, ''));
  if (!stems.length) return '';
  if (stems.length === 1) return stems[0];
  let n = stems[0].length;
  for (const s of stems.slice(1)) {
    let k = 0;
    while (k < n && k < s.length && s[k] === stems[0][k]) k++;
    n = k;
  }
  let p = stems[0].slice(0, n);
  if (/^\d/.test(stems[0].slice(n))) p = p.replace(/\d+$/, '');
  p = p.replace(/[_\- .]+$/, '');
  return p.length >= 3 ? p : '';
}

/** A dropped folder (DataTransferItem.webkitGetAsEntry), or dropped files. */
export async function fromDrop(dt: DataTransfer): Promise<{ name: string; files: PackageFile[] } | null> {
  const items = Array.from(dt.items).map((i) => i.webkitGetAsEntry?.()).filter(Boolean) as FileSystemEntry[];
  const dir = items.find((e) => e.isDirectory) as FileSystemDirectoryEntry | undefined;
  if (!dir) {
    const loose = Array.from(dt.files).filter((f) => keep(f.name)).map((f) => ({ path: f.name, file: f as Blob }));
    return loose.length ? { name: commonStem(loose.map((f) => f.path)) || 'images', files: loose } : null;
  }
  const files: PackageFile[] = [];
  async function walk(d: FileSystemDirectoryEntry, prefix: string, depth: number) {
    if (depth > 3) return;
    const reader = d.createReader();
    for (;;) {
      const batch: FileSystemEntry[] = await new Promise((res, rej) => reader.readEntries(res, rej));
      if (!batch.length) break;
      for (const e of batch) {
        const path = prefix ? `${prefix}/${e.name}` : e.name;
        if (e.isDirectory) await walk(e as FileSystemDirectoryEntry, path, depth + 1);
        else if (keep(path)) files.push({ path, file: await new Promise<File>((res, rej) => (e as FileSystemFileEntry).file(res, rej)) });
      }
    }
  }
  await walk(dir, '', 0);
  return { name: dir.name, files };
}

/**
 * Files UPb Lab handed over (`onOpenFiles`, a drop on its window or an OS
 * open): paths are already relative to the dropped folder, as from `fromDrop`.
 */
export function fromShell(set: { name: string; files: { path: string; file: File }[] }): { name: string; files: PackageFile[] } {
  const files = set.files.filter((f) => keep(f.path)).map((f) => ({ path: f.path, file: f.file as Blob }));
  return { name: set.name || commonStem(files.map((f) => f.path)) || 'images', files };
}

/** Skip hidden files and large files that no reader needs (Zigzag's own stitch, office files). */
function keep(path: string): boolean {
  const n = path.split('/').pop() || '';
  if (n.startsWith('.')) return false;
  if (/_stitch\.jpe?g$/i.test(n)) return false;
  if (/\.(pptx?|docx?|xlsx?|pdf|zip)$/i.test(n)) return false;
  return true;
}

export function download(data: BlobPart | Uint8Array, name: string, type = 'application/octet-stream') {
  if (!canWrite()) return;
  if (isDesktop()) {
    // The desktop app saves through the native dialog.
    import('./core/tauri').then(({ saveFile }) => saveFile(name, typeof data === 'string' ? data : data instanceof Uint8Array ? data : new Uint8Array(0)));
    return;
  }
  const url = URL.createObjectURL(new Blob([data as BlobPart], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

export function rawToCanvas(img: { w: number; h: number; data: Uint8Array }, lut?: Uint8Array): HTMLCanvasElement {
  const c = document.createElement('canvas');
  c.width = img.w;
  c.height = img.h;
  const ctx = c.getContext('2d')!;
  const id = ctx.createImageData(img.w, img.h);
  for (let i = 0, j = 0; i < img.data.length; i++, j += 4) {
    const v = lut ? lut[img.data[i]] : img.data[i];
    id.data[j] = id.data[j + 1] = id.data[j + 2] = v;
    id.data[j + 3] = 255;
  }
  ctx.putImageData(id, 0, 0);
  return c;
}
