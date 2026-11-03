// Getting package folders in (input, drag and drop) and files out (downloads).

import type { PackageFile } from './core/core';

/** Files from an <input type=file webkitdirectory>: paths relative to the chosen folder. */
export function fromInput(list: FileList): { name: string; files: PackageFile[] } {
  const files: PackageFile[] = [];
  let name = 'package';
  for (const f of Array.from(list)) {
    const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
    const parts = rel.split('/');
    if (parts.length > 1) name = parts[0];
    const path = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
    if (keep(path)) files.push({ path, file: f });
  }
  return { name, files };
}

/** Files from a dropped folder (DataTransferItem.webkitGetAsEntry). */
export async function fromDrop(dt: DataTransfer): Promise<{ name: string; files: PackageFile[] } | null> {
  const items = Array.from(dt.items).map((i) => i.webkitGetAsEntry?.()).filter(Boolean) as FileSystemEntry[];
  const dir = items.find((e) => e.isDirectory) as FileSystemDirectoryEntry | undefined;
  if (!dir) return null;
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

/** Skip hidden files and large files that no reader needs (Zigzag's own stitch, office files). */
function keep(path: string): boolean {
  const n = path.split('/').pop() || '';
  if (n.startsWith('.')) return false;
  if (/_stitch\.jpe?g$/i.test(n)) return false;
  if (/\.(pptx?|docx?|xlsx?|pdf|zip)$/i.test(n)) return false;
  return true;
}

export function download(data: BlobPart | Uint8Array, name: string, type = 'application/octet-stream') {
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
