import type { PrintQuality } from './print-quality';
import type { StripKey, DroppedEdit, LayoutParams, LayoutResult, SheetParams, SheetPlan, SheetResult, WholeParams, WholeResult } from './core/types.ts';
import type { StripHistory } from './strip-edit.ts';

// Grain/spot/label collections are immutable in AppState. Their identities are
// exact revisions for this session, including undo, without walking polygons.
const identities = new WeakMap<object, number>();
let serial = 0;
export function revision(value: object | null | undefined): number {
  if (!value) return 0;
  let id = identities.get(value);
  if (!id) { id = ++serial; identities.set(value, id); }
  return id;
}
export function printFingerprint(inputs: {
  identity: unknown; controls: unknown; areas: unknown; tone: unknown;
  grains: Record<string, object | undefined>; spots?: object; labels?: object; edits: unknown; image_edits?: unknown;
}): string {
  return JSON.stringify([inputs.identity, inputs.controls, inputs.areas, inputs.tone,
    Object.keys(inputs.grains).sort().map(id => [id, revision(inputs.grains[id])]),
    revision(inputs.spots), revision(inputs.labels), inputs.edits, inputs.image_edits]);
}

/** A taken entry belongs to the mounted screen; only idle results consume this budget. */
export class PrintLRU<T> {
  private entries = new Map<string, { value: T; bytes: number; owner: number }>();
  private closed = new Set<number>();
  bytes = 0;
  readonly limit: number;
  readonly budget: number;
  private dispose: (value: T) => void;
  constructor(limit = 2, budget = 300 * 1024 * 1024, dispose: (value: T) => void = () => {}) {
    this.limit = limit; this.budget = budget; this.dispose = dispose;
  }
  take(key: string): T | undefined {
    const entry = this.entries.get(key);
    if (!entry) return;
    this.entries.delete(key); this.bytes -= entry.bytes;
    return entry.value;
  }
  put(key: string, owner: number, value: T, bytes: number): boolean {
    this.remove(key);
    if (this.closed.has(owner) || bytes > this.budget || this.limit < 1) { this.dispose(value); return false; }
    this.entries.set(key, { value, bytes, owner }); this.bytes += bytes;
    while (this.entries.size > this.limit || this.bytes > this.budget) this.remove(this.entries.keys().next().value!);
    return true;
  }
  remove(key: string) {
    const e = this.entries.get(key);
    if (e) { this.entries.delete(key); this.bytes -= e.bytes; this.dispose(e.value); }
  }
  /** An in-flight page may finish after a screen has put its paused job here. */
  reweigh(key: string, value: T, bytes: number) {
    const entry = this.entries.get(key);
    if (!entry || entry.value !== value) return;
    this.bytes += bytes - entry.bytes; entry.bytes = bytes;
    if (bytes > this.budget) this.remove(key);
    while (this.bytes > this.budget) this.remove(this.entries.keys().next().value!);
  }
  clearOwner(owner: number, closing = false) {
    if (closing) this.closed.add(owner);
    for (const [key, e] of this.entries) if (e.owner === owner) this.remove(key);
  }
}
export interface PrintControls {
  params: LayoutParams; sheet: SheetParams; style: 'sheets' | 'layout'; sheetMode: 'grains' | 'whole';
  orientation: WholeParams['orientation']; wholeScale: number | 'auto'; grid: boolean; wholeArea: string; spotText: string;
}
export interface PrintEditorState { arrange?: boolean; selections?: StripKey[]; snap?: boolean; selection: StripKey | null; splitting: boolean; axis: 'x' | 'y' }
export interface CachedPrint {
  quality: PrintQuality | null;
  fingerprint: string; controls: PrintControls;
  sheets: SheetResult | null; whole: WholeResult | null; result: LayoutResult | null;
  automaticPlan: SheetPlan | null; history: StripHistory; savedDrops: DroppedEdit[];
  pageIdx: number; editMode: boolean; editor: PrintEditorState; urls: string[];
}
export const printCache = new PrintLRU<CachedPrint>(2, 300 * 1024 * 1024, entry => {
  entry.quality?.release();
  for (const url of entry.urls) if (url) URL.revokeObjectURL(url);
});
