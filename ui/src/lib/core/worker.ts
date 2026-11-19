// Web Worker hosting the wasm core. Messages: {id, op, args}; replies:
// {id, ok, value} or {id, ok: false, error}; progress: {id, progress}.
// Image buffers go back as transferables.

import init, * as wasm from './pkg/grainatlas.js';

// Also type the new export before the next wasm-bindgen regeneration.
declare module './pkg/grainatlas.js' {
  interface Session { check_project(project: string, labels: string): string }
}

let ready: ReturnType<typeof init> | null = null;
let session: wasm.Session | null = null;

function img(buf: Uint8Array) {
  if (buf.length < 8) return null;
  const dv = new DataView(buf.buffer, buf.byteOffset, 8);
  return { w: dv.getUint32(0, true), h: dv.getUint32(4, true), data: buf.subarray(8) };
}

type Handler = (args: any, id: number) => unknown | Promise<unknown>;

const ops: Record<string, Handler> = {
  memory_report: async () => ({ ...JSON.parse(s().memory_report()), wasmBytes: (await ready)!.memory.buffer.byteLength }),
  async open({ name, files }: { name: string; files: { path: string; file: Blob }[] }, id) {
    session?.free();
    session = new wasm.Session(name);
    let i = 0;
    for (const f of files) {
      session.add_file(f.path, new Uint8Array(await f.file.arrayBuffer()));
      if (++i % 8 === 0) post({ id, progress: { stage: 'read files', fraction: i / files.length } });
    }
    return JSON.parse(session.open_package());
  },
  blank({ name }: { name: string }) {
    session?.free();
    session = new wasm.Session(name);
  },
  openLoose: ({ spec }) => JSON.parse(s().open_loose(JSON.stringify(spec))),
  loadProject: ({ json }) => s().load_project(json),
  loadBundleArea: ({ areaId, w, h, pixels }) => JSON.parse(s().load_bundle_area(areaId, w, h, pixels)),
  setPackageDigest: ({ sha256, files }) => s().set_package_digest(sha256, files),
  bakeBundleArea: ({ areaId, displayTone }) => JSON.parse(s().bake_bundle_area(areaId, JSON.stringify(displayTone))),
  projectJson: () => s().project_json(),
  navigation: () => img(s().navigation()),
  tileThumb: ({ area, tile, max }) => img(s().tile_thumb(area, tile, max)),
  stitch: ({ areaId, params }, id) =>
    JSON.parse(s().stitch(areaId, JSON.stringify(params), (stage: string, fraction: number) => post({ id, progress: { stage, fraction } }), () => false)),
  recompute: ({ areaId, params }, id) =>
    JSON.parse(s().recompute(areaId, JSON.stringify(params), (stage: string, fraction: number) => post({ id, progress: { stage, fraction } }), () => false)),
  preview: ({ areaId, params, level }) => img(s().preview(areaId, JSON.stringify(params), level)),
  autoParams: ({ areaId, params }) => JSON.parse(s().auto_params(areaId, JSON.stringify(params))),
  setGrains: ({ areaId, grains }) => s().set_grains(areaId, JSON.stringify(grains)),
  defaultParams: () => JSON.parse(wasm.default_params()),
  pyramidTile: ({ areaId, level, col, row }) => img(s().pyramid_tile(areaId, level, col, row)),
  histogram: ({ areaId }) => s().histogram(areaId),
  retone: ({ areaId, tone }) => JSON.parse(s().retone(areaId, JSON.stringify(tone))),
  toneLut: ({ tone }) => wasm.tone_lut(JSON.stringify(tone)),
  exportPng: ({ areaId, tone }) => s().export_png(areaId, JSON.stringify(tone)),
  areaSignal: ({ areaId }) => JSON.parse(s().area_signal(areaId)),
  segmentWarning: ({ areaId, params, grains }) => JSON.parse(s().segment_warning(areaId, JSON.stringify(params), JSON.stringify(grains))),
  segment: ({ areaId, params, previous, outline }) => JSON.parse(s().segment(areaId, JSON.stringify(params), JSON.stringify(previous), JSON.stringify(outline))),
  segmentMissing: ({ areaId, params, existing, outline }) => JSON.parse(s().segment_missing(areaId, JSON.stringify(params), JSON.stringify(existing), JSON.stringify(outline))),
  grainAt: ({ areaId, x, y, params, outline }) => JSON.parse(s().grain_at(areaId, x, y, JSON.stringify(params), JSON.stringify(outline))),
  splitGrain: ({ areaId, grain, line }) => JSON.parse(s().split_grain(areaId, JSON.stringify(grain), new Float64Array(line))),
  mergeGrains: ({ areaId, grains }) => JSON.parse(s().merge_grains(areaId, JSON.stringify(grains))),
  refineGrain: ({ areaId, grain }) => JSON.parse(s().refine_grain(areaId, JSON.stringify(grain))),
  snapOutline: ({ areaId, path, widthUm, contrast }) => JSON.parse(s().snap_outline(areaId, JSON.stringify(path), widthUm, contrast)),
  outlineGrain: ({ areaId, polygon, base, op, outline }) => JSON.parse(s().outline_grain(areaId, JSON.stringify(polygon), base ? JSON.stringify(base) : '', op, JSON.stringify(outline))),
  brushGrain: ({ areaId, stroke, sizeUm, base, op }) => JSON.parse(s().brush_grain(areaId, JSON.stringify(stroke), sizeUm, base ? JSON.stringify(base) : '', op)),
  renumber: ({ grains, tolerance, from }) => JSON.parse(wasm.renumber(JSON.stringify(grains), tolerance, from ? JSON.stringify(from) : '')),
  checkProject: ({ project, labels }) => {
    const checker = session ?? new wasm.Session('checks');
    try { return JSON.parse(checker.check_project(JSON.stringify(project), JSON.stringify(labels))); }
    finally { if (!session) checker.free(); }
  },
  exportSpots: ({ spots, grains, format }) => s().export_spots(JSON.stringify(spots), JSON.stringify(grains), format),
  setLink: ({ mountId, samples }) => s().set_link(mountId, JSON.stringify(samples)),
  mosaicExportPlan: ({ project, options }) => JSON.parse(s().mosaic_export_plan(project, options)),
  mosaicExportBegin: ({ project, options }) => JSON.parse(s().mosaic_export_begin(project, options)),
  mosaicExportNext: () => s().mosaic_export_next(),
  mosaicExportAbort: () => s().mosaic_export_abort(),
  grainExportBegin: ({ project, options }) => s().grain_export_begin(JSON.stringify(project), JSON.stringify(options)),
  grainExportAbort: () => s().grain_export_abort(),
  grainExportBatch: ({ zipStream }, id) => ({ data: s().grain_export_batch((stage: string, fraction: number) => post({ id, progress: { stage, fraction } }), zipStream), done: s().grain_export_done() }),
  exportGrains: ({ areaId, grains }) => s().export_grains(areaId, JSON.stringify(grains)),
  layout: ({ params, grains, spots, labels, tone }) =>
    JSON.parse(s().layout(JSON.stringify(params), JSON.stringify(grains), JSON.stringify(spots), JSON.stringify(labels), JSON.stringify(tone))),
  pdf: ({ pages }) => wasm.pdf_from_svgs(JSON.stringify(pages)),
  register: ({ pairs }) => JSON.parse(wasm.register(JSON.stringify(pairs))),
  atlasZip: ({ project, options }) => s().atlas_zip(JSON.stringify(project), JSON.stringify(options)),
  recordPublished: ({ published }) => s().record_published(JSON.stringify(published)),
  laserFit: ({ fiducials, units }) => JSON.parse(wasm.laser_fit(JSON.stringify(fiducials), units)),
  laserAdapters: () => JSON.parse(wasm.laser_adapters()),
  setLaser: ({ areaId, reg }) => s().set_laser(areaId, reg ? JSON.stringify(reg) : ''),
  laserExport: ({ spots, areaId, adapter }) => s().laser_export(JSON.stringify(spots), areaId, adapter ?? ''),
  ownership: ({ grains }) => JSON.parse(s().ownership(JSON.stringify(grains), '')),
  printWhole: ({ params, tone }) => JSON.parse(s().print_whole(JSON.stringify(params), JSON.stringify(tone))),
  printSheets: ({ params, grains, tone, edits }) => JSON.parse(s().print_sheets_with_edits(JSON.stringify(params), JSON.stringify(grains), JSON.stringify(tone), edits ? JSON.stringify(edits) : '')),
  printPlanRender: ({ plan, edits, grains, tone }) => JSON.parse(s().print_plan_render(JSON.stringify(plan), JSON.stringify(edits), JSON.stringify(grains), JSON.stringify(tone))),
  printBandImage: ({ areaId, mosaicRectUm, tone }) => s().print_band_image(areaId, JSON.stringify(mosaicRectUm), JSON.stringify(tone)),
};

function s(): wasm.Session {
  if (!session) throw new Error('no package open');
  return session;
}

function post(msg: unknown, transfer: Transferable[] = []) {
  (self as unknown as Worker).postMessage(msg, transfer);
}

function transferables(v: unknown): Transferable[] {
  if (v instanceof Uint8Array || v instanceof Uint32Array) return [v.buffer];
  if (v && typeof v === 'object' && 'data' in v && (v as { data: unknown }).data instanceof Uint8Array) return [(v as { data: Uint8Array }).data.buffer];
  return [];
}

self.onmessage = async (e: MessageEvent) => {
  const { id, op, args } = e.data;
  try {
    if (op === 'init') {
      ready = init({ module_or_path: e.data.module as WebAssembly.Module });
      await ready;
      return;
    }
    if (!ready) throw new Error('worker is not initialized');
    await ready;
    const h = ops[op];
    if (!h) throw new Error(`unknown op ${op}`);
    let value = await h(args, id);
    // Copy out of wasm memory so the buffer can be transferred.
    if (value instanceof Uint8Array) value = value.slice();
    else if (value instanceof Uint32Array) value = value.slice();
    else if (value && typeof value === 'object' && 'data' in value && (value as { data: unknown }).data instanceof Uint8Array) {
      const v = value as { data: Uint8Array };
      value = { ...v, data: v.data.slice() };
    }
    post({ id, ok: true, value }, transferables(value));
  } catch (err) {
    post({ id, ok: false, error: err instanceof Error ? err.message : String(err) });
  }
};
