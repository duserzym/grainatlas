import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WasmCore } from './core/wasm';
import type { MosaicExportOptions, MosaicExportPlan } from './core/core';
import type { Project } from './core/types';
import { neutralTone } from './bundle-tone';
import { mosaicFolderSink, mosaicZipSink, writeMosaicExport } from './mosaic-export';

// Reproduce $state's relevant transport behaviour, including nested proxies.
// Native structuredClone rejects these, even though JSON can read their data.
function stateProxy<T extends object>(data: T): T {
  return new Proxy(data, {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver);
      return value && typeof value === 'object' ? stateProxy(value) : value;
    },
  });
}

type Request = { id?: number; op: string; args?: Record<string, string> };
let requests: Request[];
let plan: MosaicExportPlan;
class CloneCheckingWorker {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  postMessage(payload: Request) {
    // Check the actual envelope sent by WasmCore, not a test-built DTO.
    expect(() => structuredClone(payload)).not.toThrow();
    requests.push(structuredClone(payload));
    if (payload.op === 'init') return;
    const value = payload.op === 'mosaicExportNext'
      ? { name: plan.images[0].name, data: new Uint8Array([1, 2, 3]), last: true, done: true, tiles_done: 1, tiles_total: 1 }
      : payload.op === 'mosaicExportAbort' ? undefined : plan;
    queueMicrotask(() => this.onmessage?.({ data: { id: payload.id, ok: true, value } }));
  }
  terminate() {}
}

beforeEach(() => {
  requests = [];
  vi.stubGlobal('Worker', CloneCheckingWorker);
  vi.stubGlobal('fetch', vi.fn(async () => new Response(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))));
});
afterEach(() => vi.unstubAllGlobals());

describe('mosaic worker requests contain plain data only', () => {
  for (const format of ['tiff', 'png', 'jpg'] as const) {
    for (const destination of ['folder', 'zip'] as const) {
      it(`${format} planning and ${destination} streaming accept proxy-backed state`, async () => {
        const project = stateProxy({ name: 'Synthetic mount', areas: [{ id: 'a', name: 'Synthetic area' }] }) as unknown as Project;
        const options = stateProxy<MosaicExportOptions>({
          area_ids: ['a'], whole_mount: false, values: format === 'jpg' ? 'displayed' : 'original',
          display_tone: { a: neutralTone() },
          format, png_bits: 16, quality: 90, size_choice: 'full',
        });
        expect(() => structuredClone(project)).toThrow();
        expect(() => structuredClone(options)).toThrow();
        expect(() => structuredClone({ ...options })).toThrow();
        plan = { images: [{ name: `Synthetic_mount_Synthetic_area_mosaic.${format}` }], whole_mount_reason: null } as MosaicExportPlan;
        const writes: Uint8Array[] = [];
        const stream = { write: async (b: Uint8Array) => { writes.push(b); }, close: vi.fn(async () => {}), abort: vi.fn(async () => {}) };
        const directory = { getFileHandle: async () => ({ createWritable: async () => stream }), getDirectoryHandle: async () => directory, removeEntry: async () => {} };
        // Functions make these uncloneable. They must remain in the main-thread sink.
        expect(() => structuredClone(directory)).toThrow();
        expect(() => structuredClone(stream)).toThrow();
        const sink = destination === 'folder'
          ? mosaicFolderSink(directory, async () => {})
          : mosaicZipSink(b => stream.write(b), () => stream.close(), () => stream.abort());
        const core = new WasmCore();
        try {
          const estimate = await core.planMosaicExport(project, options);
          await writeMosaicExport(core, project, options, sink, vi.fn(), () => false, estimate);
          expect(requests.map(r => r.op)).toEqual(['init', 'mosaicExportPlan', 'mosaicExportBegin', 'mosaicExportNext', 'mosaicExportAbort']);
          for (const request of requests.filter(r => r.op === 'mosaicExportPlan' || r.op === 'mosaicExportBegin')) {
            expect(Object.keys(request.args!).sort()).toEqual(['options', 'project']);
            expect(JSON.parse(request.args!.project)).toEqual(JSON.parse(JSON.stringify(project)));
            expect(JSON.parse(request.args!.options)).toEqual(JSON.parse(JSON.stringify(options)));
          }
          expect(writes.length).toBeGreaterThan(0);
          expect(stream.close).toHaveBeenCalledOnce();
          expect(stream.abort).not.toHaveBeenCalled();
        } finally { await core.dispose(); }
      });
    }
  }
});
