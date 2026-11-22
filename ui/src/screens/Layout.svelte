<script lang="ts">
  // Print layout: paper, scale, packing, preview, PDF export and printing.
  // Two styles: print sheets (every grain of
  // the mount once, at a fixed scale, with a mount map and the print_sheet
  // registration for hand spot registration) and the grain / area layout.
  import { current } from '../lib/store.svelte';
  import { download, savePrintPdf } from '../lib/files';
  import { wholePrintOptions } from '../lib/whole-print';
  import type { DroppedEdit, Grain, LayoutParams, LayoutResult, SheetParams, SheetPlan, SheetResult, WholeParams, WholeResult, Spot, Label, ToneParams } from '../lib/core/types';
  import StripEditor from '../components/StripEditor.svelte';
  import { readPrintEdits, storePrintEdits, StripHistory } from '../lib/strip-edit';
  import { onDestroy, untrack } from 'svelte';
  import { printCache, printFingerprint, revision, type PrintControls, type PrintEditorState } from '../lib/print-cache';
  import { PrintQuality, printPdfName } from '../lib/print-quality';
  import { createPrintDocument, type PrintDocument } from '../lib/print-document';
  import WindowsPrintDialog, { type WindowsPrintOptions } from '../components/WindowsPrintDialog.svelte';
  import { isDesktop } from '../lib/core/core';
  import { invoke } from '@tauri-apps/api/core';
  import PanZoom from '../lib/canvas/PanZoom.svelte';
  import SpotLabelOverlay from '../components/SpotLabelOverlay.svelte';
  import { isSheetsStyle, pinLabel, spotDisplay, sheetGeometryControls, parseLabelSize } from '../lib/print-labels';
  import type { SpotLabelReport } from '../lib/core/types';

  import PickListImport from '../components/PickListImport.svelte';
  import { buildPickListRows, renderPickList, type PickListOptions } from '../lib/tims-picklist';
  import { bundlePixels } from '../lib/bundle-tone';

  const app = current();
  let moveLabels = $state(false);
  let renderedDisplay = $state('');
  let renderedGeometry = $state('');
  let displayTimer: ReturnType<typeof setTimeout> | undefined;
  let displayRequest = 0;
  const displayKey = () => JSON.stringify([style, style === 'spots' ? app.printLabels : null, style === 'spots' ? revision(app.spots) : 0]);
  const geometryKey = (c = controls()) => fingerprint(sheetGeometryControls(c));
  function savePin(label: SpotLabelReport['labels'][number], origin: [number, number] | null) {
    app.printLabels = pinLabel($state.snapshot(app.printLabels), label, origin);
    app.persistPrintLabels();
  }
  function resetLabels() {
    if (!labelPins || !confirm(`Reset ${labelPins} pinned spot label${labelPins === 1 ? '' : 's'}?`)) return;
    app.printLabels = { ...$state.snapshot(app.printLabels), pins: {} }; app.persistPrintLabels();
  }
  function labelsChanged(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const size = parseLabelSize(input.value);
    if (size === null) { input.value = app.printLabels.size_mm === 'auto' ? 'Auto' : String(app.printLabels.size_mm); return; }
    app.printLabels = { ...$state.snapshot(app.printLabels), size_mm: size };
    input.value = size === 'auto' ? 'Auto' : String(size);
    app.persistPrintLabels();
  }
  async function refreshLabels() {
    if (destroyed || app.busy || pending || !result || !isSheetsStyle(style)) return;
    if (sheetMode === 'whole') { await generateSheets(); return; }
    if (!automaticPlan || geometryKey() !== renderedGeometry) return;
    const token = ++displayRequest, key = displayKey(), c = controls();
    const generationKey = fingerprint(c);
    pending = true;
    try {
      const base = spotDisplay(automaticPlan, style === 'spots', $state.snapshot(app.spots), $state.snapshot(app.printLabels));
      const r = await (await app.core).printPlanRender(base, $state.snapshot(history.edits), $state.snapshot(app.grains), $state.snapshot(app.tone));
      if (destroyed || token !== displayRequest || key !== displayKey()) return;
      automaticPlan = base; showSheets(r); madeControls = c; madeWith = generationKey; renderedDisplay = key;
    } catch (error) { app.error = String(error); }
    finally { if (!destroyed) pending = false; }
  }
  $effect(() => {
    const key = displayKey();
    if (!result || !isSheetsStyle(style) || !renderedDisplay || renderedDisplay === key || app.busy || pending || geometryKey() !== renderedGeometry) return;
    clearTimeout(displayTimer); displayTimer = setTimeout(() => void refreshLabels(), 80);
    return () => clearTimeout(displayTimer);
  });
  let importingPickList = $state(false);
  let pick = $state<PickListOptions>({ set: '', batch: '', cropMargin: 30, cropSize: 35, paper: 'a4', title: `${app.pkg?.name ?? app.project?.package.name ?? 'Mount'} pick list ${new Date().toLocaleDateString('en-CA')}` });
  const pickStore = $derived(app.tims.store);
  const pickRows = $derived.by(() => {
    app.tims.version;
    return buildPickListRows({ fractions: pickStore?.fractions ?? [], sets: pickStore?.data.sets ?? [], batches: pickStore?.batches ?? [], outbox: pickStore?.data.outbox ?? [], grains: app.grains, spots: app.spots, areas: app.project?.areas ?? [], filter: pick });
  });
  const hasPlucks = $derived.by(() => { app.tims.version; return !!pickStore?.fractions.some(f => !f.retired_at && f.sources.some(s => !s.retired_at)); });
  const pickSets = $derived.by(() => { app.tims.version; return pickStore?.data.sets ?? []; });
  const pickBatches = $derived.by(() => { app.tims.version; return pickStore?.batches.filter(b => !pick.set || b.capsule_set_id === pick.set) ?? []; });
  const pickPixelsReady = $derived(!app.remote || bundlePixels(app));


  /** Preview pixels per paper millimetre (zooming in re-rasterises the SVG page). */
  const PX_PER_MM = 6;
  let panzoom = $state<ReturnType<typeof PanZoom>>();

  const stitchedIds = $derived(Object.keys(app.stitched));
  let params = $state<LayoutParams>({
    paper: 'tabloid',
    custom_w_mm: 279.4,
    custom_h_mm: 431.8,
    landscape: false,
    margin_mm: 12,
    um_per_mm: 0,
    items: 'areas',
    area_ids: [],
    grain_margin_um: 20,
    show_spots: true,
    show_labels: true,
    show_outlines: false,
    cut_marks: true,
    title: '',
    dpi: 300,
    by_number: false,
  });
  let style = $state<'sheets' | 'spots' | 'layout' | 'picklist'>('sheets');
  let sheetMode = $state<'grains' | 'whole'>('grains');
  let orientation = $state<WholeParams['orientation']>('auto');
  let wholeScale = $state<number | 'auto'>('auto');
  let grid = $state(true);
  let wholeArea = $state(app.areaId ?? '');
  let whole = $state.raw<WholeResult | null>(null);
  const savedPrint = readPrintEdits(structuredClone($state.snapshot(app.project?.print_sheets)));
  const unreadablePrint = $derived(app.project?.print_sheets !== undefined && !readPrintEdits(app.project.print_sheets));
  let sheet = $state<SheetParams>({
    paper: 'tabloid',
    landscape: true,
    packing: 'lines',
    margin_mm: 8,
    um_per_mm: 'auto',
    pad_um: 22,
    gap_break_um: 420,
    context_um: 0,
    min_group: 3,
    min_cluster_grains: 5,
    overlap_um: 0,
    min_grain_um: 35,
    max_grain_um: 0,
    spot_sizes_um: [15, 20, 25, 30, 35, 50],
    enhance: { denoise: true, denoise_h: 0.7, sharpen: true, sharpen_radius: 1.6, sharpen_amount: 0.55, strip: false, strip_grey: 22, strip_margin_um: 6 },
    title: '',
    lab: '',
    project: '',
    registration_file: '',
    printed: '',
    ...savedPrint?.params,
  });
  let spotText = $state((savedPrint?.params.spot_sizes_um ?? [15, 20, 25, 30, 35, 50]).join(', '));
  let sheets = $state.raw<SheetResult | null>(null);
  const labelReports = $derived((whole ?? sheets)?.registration.spot_labels ?? []);
  const labelOverlap = $derived(labelReports.reduce((sum, page) => sum + page.counts.still_overlapping, 0));
  const labelPinnedOverlap = $derived(labelReports.reduce((sum, page) => sum + (page.counts.pinned_overlapping ?? 0), 0));
  const labelTicks = $derived(labelReports.reduce((sum, page) => sum + (page.counts.with_ticks ?? 0), 0));
  const labelLeaders = $derived(labelReports.reduce((sum, page) => sum + page.counts.with_leaders, 0));
  const labelAmbiguous = $derived(labelReports.reduce((sum, page) => sum + (page.counts.ambiguous ?? 0), 0));
  const labelContextOmitted = $derived(labelReports.reduce((sum, page) => sum + (page.counts.context_omitted ?? 0), 0));
  const labelPins = $derived(Object.keys(app.printLabels.pins).length);
  let result = $state.raw<LayoutResult | null>(null);
  let pageIdx = $state(0);
  let urls = $state<string[]>([]);
  let history = $state.raw(new StripHistory());
  let automaticPlan = $state.raw<SheetPlan | null>(null);
  let editMode = $state(savedPrint?.version === 2);
  let stripEditor = $state<ReturnType<typeof StripEditor>>();
  let editor = $state<PrintEditorState>({ arrange: savedPrint?.version === 2, selections: [], snap: true, selection: null, splitting: false, axis: 'x' });
  let pending = $state(false);
  let savedDrops = $state<DroppedEdit[]>([]);
  const droppedEdits = $derived(savedDrops.length ? savedDrops : sheets?.dropped ?? []);
  let editorControls = $state<HTMLElement>();
  let quality = $state.raw<PrintQuality | null>(null);
  let qualityRevision = $state(0);
  let qualityUnsubscribe: (() => void) | undefined;
  let destroyed = false;
  let printDocument: PrintDocument | undefined;
  let windowsPrint = $state<{ doc: PrintDocument; title: string; paper: [number, number]; pages: number } | null>(null);
  let stale = $state(false);
  let qualityCancelled = $state(false);
  const qualityState = $derived.by(() => { qualityRevision; return quality ? { ready: quality.ready, next: quality.next, total: quality.total, error: quality.error } : null; });
  function watchQuality() {
    qualityUnsubscribe?.();
    qualityUnsubscribe = quality?.subscribe(() => {
      if (destroyed) return;
      if (quality?.summary) {
        if (sheets) sheets = { ...sheets, registration: quality.summary.registration, stats: quality.summary.stats as SheetResult['stats'] };
        if (whole) whole = { ...whole, registration: quality.summary.registration, stats: quality.summary.stats as WholeResult['stats'] };
      }
      qualityRevision++;
    });
    qualityRevision++;
    if (quality && !quality.ready && !qualityCancelled) void quality.start();
  }
  function beginQuality(r: { job_id?: number; pages: string[] }) {
    quality?.release();
    qualityCancelled = false;
    void app.core.then(core => {
      if (destroyed) return;
      quality = r.job_id ? new PrintQuality(core, r.job_id, r.pages.length, r.pages.reduce((n, p) => n + p.length * 3, 0)) : null;
      watchQuality();
    });
  }
  function cancelQuality() { qualityCancelled = true; quality?.stop(); qualityRevision++; }
  const exportBlocked = $derived(!!app.busy || stale || pending || !!sheets?.problems?.length);
  const exportReason = $derived(app.busy ? app.busy.label : stale ? 'Inputs changed; make sheets again' : pending ? 'Wait for the strip preview' : exportBlocked ? 'Fix strip problems before export' : '');
  function controls(): PrintControls {
    return $state.snapshot({ params, sheet, style, sheetMode, orientation, wholeScale, grid, wholeArea, spotText, pick, spotLabels: app.printLabels });
  }
  function fingerprint(c = controls()) {
    const ids = isSheetsStyle(c.style) && c.sheetMode === 'whole' ? [c.wholeArea]
      : c.style === 'layout' && c.params.area_ids.length ? c.params.area_ids : Object.keys(app.stitched);
    return printFingerprint({
      identity: [app.id, app.project?.uuid, app.projectPath, revision(app.packageSource), revision(app.pkg),
        app.project?.package, app.project?.revision, app.pixelsVersion,
        app.remote?.server, app.remote?.projectId, app.remote?.version, app.sync.baseRevision],
      controls: c.style === 'picklist' ? { ...c, rows: pickRows } : c,
      areas: app.project?.areas.filter(a => ids.includes(a.id)).map(a => [a.id, a.uuid, a.name, a.sample, a.pixel_size_nm,
        revision(a.tiles), revision(a.placement), revision(app.stitched[a.id]?.placement),
        a.stage_sign, a.mosaic, app.stitched[a.id]?.mosaic, a.params, app.params[a.id], a.area_meta]),
      tone: Object.fromEntries(ids.map(id => [id, app.tone[id]])),
      grains: Object.fromEntries(ids.map(id => [id, app.grains[id]])),
      spots: c.style === 'spots' || c.style === 'picklist' || c.style === 'layout' && c.params.show_spots ? app.spots : undefined,
      labels: c.style === 'layout' && c.params.show_labels ? app.labels : undefined,
      edits: app.project?.print_sheets,
      image_edits: app.project?.image_edits,
    });
  }
  let madeWith = $state('');
  let madeControls = $state.raw<PrintControls | null>(null);
  function restoreCached() {
    const cached = printCache.take(String(app.id));
    if (cached) {
      ({ params, sheet, style, sheetMode, orientation, wholeScale, grid, wholeArea, spotText } = cached.controls);
      if (cached.controls.pick) pick = cached.controls.pick;
      if (cached.fingerprint === fingerprint()) {
        ({ sheets, whole, result, automaticPlan, history, savedDrops, pageIdx, editMode, editor, urls, quality } = cached);
        madeWith = cached.fingerprint;
        madeControls = cached.controls;
        renderedDisplay = displayKey();
        renderedGeometry = geometryKey(cached.controls);
      } else { cached.quality?.release(); for (const url of cached.urls) url && URL.revokeObjectURL(url); }
    }
  }
  restoreCached();
  watchQuality();
  onDestroy(() => {
    clearTimeout(displayTimer); displayRequest++;
    printDocument?.cleanup();
    destroyed = true; const stopped = quality?.stop(); qualityUnsubscribe?.();
    if (result && !pending && madeWith && madeWith === fingerprint()) {
      // At most two bytes per JS code unit plus three in its UTF-8 Blob.
      const bytes = result.pages.reduce((n, p) => n + p.length * 5, 0);
      const cached = { fingerprint: madeWith, controls: controls(),
        sheets, whole, result, automaticPlan, history, savedDrops: $state.snapshot(savedDrops),
        pageIdx, editMode, quality, editor: $state.snapshot(editor), urls: $state.snapshot(urls) };
      const key = String(app.id);
      printCache.put(key, app.id, cached, bytes + (quality?.residentBytes ?? 0));
      void stopped?.then(() => printCache.reweigh(key, cached, bytes + (cached.quality?.residentBytes ?? 0)));
    } else { quality?.release(); for (const url of urls) url && URL.revokeObjectURL(url); }
  });
  function showSheets(r: SheetResult) {
    beginQuality(r);
    sheets = r;
    replacePages(r.pages);
    result = { job_id: r.job_id, pages: r.pages, um_per_mm: r.registration.um_per_mm, items: r.stats.bands, paper_mm: r.registration.paper_mm };
  }
  function storeHistory() {
    if (!automaticPlan || app.readOnly) return;
    storePrintEdits(app, $state.snapshot(automaticPlan) as SheetPlan, $state.snapshot(history.edits));
    app.persist();
    savedDrops = [];
    if (madeControls) { madeWith = fingerprint(madeControls); renderedGeometry = geometryKey(madeControls); }
  }
  function replacePages(pages: string[]) {
    const old = urls;
    urls = pages.map(() => '');
    for (const url of old) url && URL.revokeObjectURL(url);
  }
  // Only materialise a Blob for a page the viewer actually visits. Full-quality
  // pages stay in the core until PDF export or printing.
  $effect(() => {
    const svg = result?.pages[pageIdx];
    if (svg && !urls[pageIdx]) urls[pageIdx] = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  });
  const hasGrains = $derived(stitchedIds.some((id) => (app.grains[id] ?? []).length > 0));

  $effect(() => {
    if (!stitchedIds.includes(wholeArea)) wholeArea = stitchedIds[0] ?? '';
    if (!params.title) params.title = app.pkg?.name ?? app.project?.package.name ?? '';
    if (!hasGrains && params.items === 'grains') params.items = 'areas';
  });

  $effect(() => {
    const key = fingerprint();
    stale = !!madeWith && key !== madeWith;
    if (stale || pending) quality?.stop();
    else if (quality && !quality.ready && !qualityCancelled) void quality.start();
  });

  $effect(() => {
    if (style !== 'picklist' || !pickStore) return;
    const store = pickStore;
    untrack(() => { void (async () => {
      await store.loadSets();
      for (const set of store.data.sets) {
        if (destroyed || !store.enabled) return;
        await store.loadBatches(set.id);
      }
    })(); });
  });
  async function generatePickList() {
    if (app.busy || !app.project || !pickStore || !pickPixelsReady) return;
    const { liveIdle } = await import('../lib/actions');
    for (const id of new Set(pickRows.flatMap(r => r.grains.map(g => g.area)))) if (id) await liveIdle(id, app);
    const core = await app.core;
    const project = $state.snapshot({ ...app.project, grainatlas_ui: undefined });
    await core.loadProject(JSON.stringify(project));
    const generationControls = controls(), generationKey = fingerprint(generationControls);
    const rows = $state.snapshot(pickRows), options = $state.snapshot(pick);
    const rendered = await app.run('Laying out pick list', () => renderPickList({ core, project, rows, options,
      mosaics: Object.fromEntries(Object.entries(app.stitched).map(([id, s]) => [id, $state.snapshot(s.mosaic)])), tone: $state.snapshot(app.tone), mount: app.pkg?.name ?? project.package.name }));
    if (!rendered) return;
    sheets = null; whole = null; result = rendered; beginQuality(rendered);
    replacePages(rendered.pages); pageIdx = 0; madeWith = generationKey; madeControls = generationControls;
  }

  async function generateSheets() {
    if (app.busy) return;
    const { liveIdle } = await import('../lib/actions');
    for (const id of stitchedIds) await liveIdle(id);
    const core = await app.core;
    if (app.project) await core.loadProject(JSON.stringify($state.snapshot({ ...app.project, grainatlas_ui: undefined })));
    const p = $state.snapshot(sheet) as SheetParams;
    const name = app.pkg?.name ?? app.project?.package.name ?? 'grainatlas';
    p.spot_sizes_um = spotText.split(/[,\s]+/).map(Number).filter((v) => v > 0);
    const projectFile = app.projectPath?.path.split(/[/\\]/).pop() ?? `${name}.grainatlas`;
    const stem = projectFile.replace(/\.grainatlas$/i, '');
    const exportName = stem.toLowerCase() === name.toLowerCase() ? stem : name;
    p.registration_file = `${exportName}_print_registration.json`;
    p.project = projectFile;
    // The print timestamp belongs to this result; the clock is not a cache input.
    p.printed = new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
    const tones = $state.snapshot(app.tone) as Record<string, ToneParams>;
    const previousPlan = automaticPlan;
    const stored = app.project?.print_sheets !== undefined
      ? readPrintEdits($state.snapshot(app.project.print_sheets))
      : history.count && previousPlan ? { params: $state.snapshot(previousPlan.params), um_per_mm: previousPlan.um_per_mm, edits: $state.snapshot(history.edits) } : null;
    const generationControls = controls();
    const generationKey = fingerprint(generationControls);
    const generationGeometry = geometryKey(generationControls);
    const r = sheetMode === 'whole'
      ? await app.run('Laying out whole area', () => core.printWhole({ ...wholePrintOptions(p, { areaId: wholeArea, orientation, scale: wholeScale, grid }), ...(style === 'spots' ? { print_spots: $state.snapshot(app.spots), print_labels: $state.snapshot(app.printLabels), print_grains: $state.snapshot(app.grains[wholeArea] ?? []) } : {}) }, tones))
      : await app.run('Laying out print sheets', () => core.printSheets(p, $state.snapshot(app.grains) as Record<string, Grain[]>, tones));
    if (!r) return;
    if ('ownership' in r) {
      r.plan = spotDisplay(r.plan, style === 'spots', $state.snapshot(app.spots), $state.snapshot(app.printLabels));
      app.ownership = r.ownership;
      whole = null;
      const replayed = stored?.edits.length || style === 'spots'
        ? await app.run('Replaying strip edits', () => core.printPlanRender($state.snapshot(r.plan) as SheetPlan, stored ?? [], $state.snapshot(app.grains) as Record<string, Grain[]>, tones))
        : r;
      if (!replayed) return;
      history.load(stored?.edits ?? []);
      if (replayed?.dropped?.length) history.rebase(replayed.dropped.map(d => d.index));
      savedDrops = replayed.dropped ?? [];
      automaticPlan = r.plan;
      pending = false;
      showSheets({ ...replayed, dropped: [] });
    } else {
      whole = r;
      sheets = null;
      beginQuality(r);
      replacePages(r.pages);
      result = { job_id: r.job_id, pages: r.pages, um_per_mm: r.registration.um_per_mm, items: 1, paper_mm: r.registration.paper_mm };
    }
    pageIdx = 0;
    madeWith = generationKey;
    madeControls = generationControls;
    renderedDisplay = displayKey();
    renderedGeometry = generationGeometry;
  }
  async function registration() {
    if (exportBlocked) return;
    let cancelled = false;
    app.cancelTask = () => { cancelled = true; cancelQuality(); };
    try {
      const ready = await app.run('Preparing registration', async progress => { if (quality) await quality.wait(progress, () => cancelled || destroyed || stale); return true; });
      const r = whole ?? sheets;
      if (ready && r && !cancelled && !destroyed && !stale) download(JSON.stringify(r.registration, null, 1), r.registration.registration_file || r.registration.package + '_print_registration.json', 'application/json');
    } finally { app.cancelTask = null; }
  }

  async function generate() {
    if (app.busy) return;
    const { liveIdle } = await import('../lib/actions');
    for (const id of stitchedIds) await liveIdle(id);
    const core = await app.core;
    if (app.project) await core.loadProject(JSON.stringify($state.snapshot({ ...app.project, grainatlas_ui: undefined })));
    const p = $state.snapshot(params) as LayoutParams;
    if (!p.area_ids.length) p.area_ids = stitchedIds;
    sheets = null;
    whole = null;
    const generationControls = controls();
    const generationKey = fingerprint(generationControls);
    const r = await app.run('Laying out pages', () =>
      core.layout(p, $state.snapshot(app.grains) as Record<string, Grain[]>, $state.snapshot(app.spots) as Spot[], $state.snapshot(app.labels) as Label[], $state.snapshot(app.tone) as Record<string, ToneParams>),
    );
    if (!r) return;
    replacePages(r.pages);
    result = r;
    beginQuality(r);
    pageIdx = 0;
    madeWith = generationKey;
    madeControls = generationControls;
  }

  async function pdf() {
    if (!result || exportBlocked) return;
    const core = await app.core;
    if (app.project) await core.loadProject(JSON.stringify($state.snapshot({ ...app.project, grainatlas_ui: undefined })));
    let cancelled = false;
    app.cancelTask = () => { cancelled = true; cancelQuality(); };
    try {
      await app.run('Exporting PDF', async progress => {
        const bytes = quality ? await quality.pdf(progress, () => cancelled || destroyed || stale) : await core.pdf(result!.pages);
        if (cancelled || destroyed || stale) throw new Error('cancelled');
        progress({ stage: 'Saving PDF', fraction: 1 });
        const r = whole ?? sheets;
        await savePrintPdf(bytes, printPdfName(style === 'picklist' ? pick.title : app.pkg?.name ?? app.project?.package.name ?? 'grainatlas'), r ? JSON.stringify(r.registration, null, 1) : null);
      });
    } finally { app.cancelTask = null; }
  }
  async function browserPrint() {
    if (!result || exportBlocked) return;
    printDocument?.cleanup();
    printDocument = undefined;
    let cancelled = false;
    app.cancelTask = () => { cancelled = true; cancelQuality(); };
    try {
      const core = await app.core;
      if (app.project) await core.loadProject(JSON.stringify($state.snapshot({ ...app.project, grainatlas_ui: undefined })));
      const pages = await app.run('Preparing print quality', async progress => {
        if (quality) await quality.wait(progress, () => cancelled || destroyed || stale);
        return quality ? core.printPages(quality.job) : result!.pages;
      });
      if (!pages || cancelled || destroyed || stale) return;
      const doc = printDocument = createPrintDocument(pages, result.paper_mm);
      await doc.decodeImages();
      if (cancelled || destroyed || stale || printDocument !== doc) { doc.cleanup(); return; }
      const title = style === 'picklist' ? pick.title : isSheetsStyle(style) ? sheet.title || app.pkg?.name || app.project?.package.name || 'Print sheets' : params.title;
      if (isDesktop() && /^Win/i.test(navigator.platform)) {
        windowsPrint = { doc, title, paper: result.paper_mm, pages: pages.length };
      } else {
        await doc.print(title);
      }
    } catch (error) { app.error = error instanceof Error ? error.message : String(error); }
    finally { app.cancelTask = null; }
  }
  async function submitWindowsPrint(options: WindowsPrintOptions) {
    const job = windowsPrint;
    if (!job) return;
    await job.doc.print(job.title, () => invoke<void>('windows_print_document', {
      job: { ...options, paperMm: job.paper },
    }));
  }
  function closeWindowsPrint() {
    windowsPrint?.doc.cleanup();
    windowsPrint = null;
    printDocument = undefined;
  }
  function toggleArea(id: string) {
    params.area_ids = params.area_ids.includes(id) ? params.area_ids.filter((a) => a !== id) : [...params.area_ids, id];
  }
</script>

<PickListImport bind:open={importingPickList} />

{#if windowsPrint}
  <WindowsPrintDialog paper={windowsPrint.paper} pages={windowsPrint.pages}
    submit={submitWindowsPrint} close={closeWindowsPrint} />
{/if}

{#snippet droppedNotice(editing: boolean)}
  {#if droppedEdits.length}
    <div class="faint small" data-testid="dropped-edits" role="status">
      {#if savedDrops.length}
        {savedDrops.length} edit{savedDrops.length === 1 ? '' : 's'} from the saved project could not be applied. Keep the stored edits by making no changes; the next strip edit replaces the stored list.
      {:else if !editing}
        {droppedEdits.length} edit{droppedEdits.length === 1 ? '' : 's'} could not be applied.
      {/if}
      {#each droppedEdits as dropped}
        <p>{editing && !savedDrops.length ? 'Not applied' : `Edit ${dropped.index + 1}`}: {dropped.reason}</p>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet qualityProgress()}
  {#if qualityState && !qualityState.ready && !qualityCancelled && !qualityState.error && !stale && !pending && !app.busy}
    <div class="quality-progress" role="status" data-testid="print-quality-busy">
      <progress aria-label="Preparing print quality" value={qualityState.next} max={qualityState.total}></progress>
      <button onclick={cancelQuality}>Pause</button>
    </div>
  {/if}
{/snippet}
<div class="layout" class:folded={app.layout.printCollapsed}>
  {#if app.layout.printCollapsed}
    <aside class="glass folded-side"><button class="icon" title="Show print controls" aria-label="Show print controls" onclick={() => { app.layout.printCollapsed = false; app.saveLayout(); }}>»</button></aside>
  {/if}
  <aside class="glass side" class:arranging={sheets && isSheetsStyle(style) && sheetMode === 'grains' && editMode && editor.arrange} class:concealed={app.layout.printCollapsed}>
    <div class="side-content" class:arranging={sheets && isSheetsStyle(style) && sheetMode === 'grains' && editMode && editor.arrange} data-testid="print-controls-scroll">
    <button class="icon fold" title="Fold print controls" aria-label="Fold print controls" onclick={() => { app.layout.printCollapsed = true; app.saveLayout(); }}>«</button>
    {#if sheets && isSheetsStyle(style) && sheetMode === 'grains'}
      <div class="seg" role="group" aria-label="Print arrangement">
        <button aria-label="Switch to automatic layout" aria-pressed={!editor.arrange} onclick={()=>{if(!editor.arrange)return;if(history.count)stripEditor?.confirmAutomatic();else{editMode=false;editor.arrange=false;}}}>Automatic</button>
        <button data-testid="arrange-mode" aria-pressed={!!editor.arrange} disabled={app.readOnly} onclick={()=>{editor.arrange=true;editMode=true;moveLabels=false;}}>Arrange</button>
      </div>
      {#if !editor.arrange}<label class="check"><input type="checkbox" bind:checked={editMode} onchange={() => { if (editMode) moveLabels = false; }} disabled={app.readOnly} data-testid="edit-strips" /> Edit strips</label>{/if}
    {/if}
    {#if style === 'spots'}
      <datalist id="spot-label-sizes"><option value="Auto"></option>{#each [1.6, 1.8, 2, 2.2, 2.4, 2.6, 2.8, 3, 3.2, 3.4, 3.6, 3.8, 4] as size}<option value={size}></option>{/each}</datalist>
      <label class="field">Label size (mm) <input aria-label="Label size (mm)" data-testid="spot-label-size" type="text" list="spot-label-sizes" value={app.printLabels.size_mm === 'auto' ? 'Auto' : app.printLabels.size_mm} onchange={labelsChanged} /></label>
      <label class="check"><input type="checkbox" bind:checked={moveLabels} onchange={() => { if (moveLabels) editMode = false; }} data-testid="move-labels" /> Move labels</label>
      <button onclick={resetLabels} disabled={!labelPins} data-testid="reset-labels">Reset labels ({labelPins})</button>
      {#if app.printLabels.size_mm === 'auto' && labelReports.find(p => p.sheet === pageIdx + 1)}
        <p class="faint small" data-testid="spot-label-chosen-size">Labels {labelReports.find(p => p.sheet === pageIdx + 1)?.size_mm.toFixed(1)} mm on this page</p>
      {/if}
      <p class="faint small" role="status" data-testid="spot-label-overlaps">{labelOverlap} spot labels still overlap; {labelAmbiguous} ambiguous; {labelTicks} ticks; {labelLeaders} leaders{#if labelPinnedOverlap}; {labelPinnedOverlap} pinned label{labelPinnedOverlap === 1 ? ' overlaps' : 's overlap'}{/if}{#if labelContextOmitted}; {labelContextOmitted} repeated context labels omitted{/if}.{#if labelOverlap || labelAmbiguous || labelPinnedOverlap}{' '}<button onclick={() => { pageIdx = Math.max(0, (labelReports.find(p => p.counts.still_overlapping || p.counts.ambiguous || p.counts.pinned_overlapping)?.sheet ?? 1) - 1); editMode = false; moveLabels = true; }}>Show page</button>{/if}</p>
    {/if}
    <div bind:this={editorControls}></div>
    {#if !(sheets && isSheetsStyle(style) && sheetMode === 'grains' && editMode && editor.arrange)}
    <h2>Print layout</h2>
    {#if result}
      <p class="faint small" data-testid="preview-note">Preview uses reduced resolution without export enhancement. PDF and print use full quality.</p>
      {#if stale}<p class="faint small">Inputs changed; make sheets again.</p>{/if}
      <p class="faint small" data-testid="print-quality" data-ready={qualityState?.ready ?? true}>{qualityState?.error ? qualityState.error.message : qualityState?.ready || !qualityState ? 'Print quality ready' : qualityCancelled ? 'Print quality paused; export resumes it.' : `Preparing print quality: page ${Math.min(qualityState.next + 1, qualityState.total)} of ${qualityState.total}`}</p>
      {@render qualityProgress()}
    {/if}
    {#if sheets?.problems?.length}
      <p class="faint small" role="status" data-testid="export-blocked">Export blocked: {sheets.problems[0].message}. {sheets.problems.some(p => p.kind === 'outside_body') ? 'Increase the scale value or paper size, or edit the strips.' : 'Fix the strip problems before exporting.'}</p>
    {/if}
    {#if unreadablePrint}<p class="faint small" data-testid="unreadable-print-edits">Stored strip edits could not be read. They will be kept when saving. A new strip edit replaces them.</p>{/if}
    <div class="seg" role="radiogroup" aria-label="Style">
      <label class="check"><input type="radio" bind:group={style} value="sheets" data-testid="style-sheets" /> Print sheets</label>
      <label class="check"><input type="radio" bind:group={style} value="spots" data-testid="style-spots" /> Grains with spots</label>
      <label class="check"><input type="radio" bind:group={style} value="layout" data-testid="style-layout" /> Grains or areas</label>
      <label class="check"><input type="radio" bind:group={style} value="picklist" disabled={!hasPlucks} data-testid="style-picklist" /> Pick list</label>
    </div>
    <button onclick={() => importingPickList = true} data-testid="import-pick-list">Import pick list…</button>
    {#if style === 'picklist'}
      <label class="field">Capsule set <select aria-label="Capsule set" bind:value={pick.set} onchange={() => pick.batch = ''}><option value="">All sets</option>{#each pickSets as set}<option value={set.id}>{set.name} ({set.code})</option>{/each}</select></label>
      <label class="field">Batch <select aria-label="Batch" bind:value={pick.batch}><option value="">All batches on this mount</option>{#each pickBatches.filter(b => pickStore?.fractions.some(f => !f.retired_at && f.batch_id === b.id)) as batch}<option value={batch.id}>{pickSets.find(s => s.id === batch.capsule_set_id)?.code} batch started {batch.started_on}</option>{/each}</select></label>
      <label class="field">Crop margin (µm) <input type="number" min="0" bind:value={pick.cropMargin} /></label>
      <label class="field">Crop size on paper (mm) <input type="number" min="15" max="60" bind:value={pick.cropSize} /></label>
      <label class="field">Paper <select aria-label="Paper" bind:value={pick.paper}><option value="tabloid">Tabloid 11 × 17 in</option><option value="letter">Letter</option><option value="a3">A3</option><option value="a4">A4</option></select></label>
      <label class="field">Title <input bind:value={pick.title} /></label>
      <p class="faint small">{pickRows.length} rows. Pending plucks are marked “not yet in the database”.</p>
      {#if !pickPixelsReady}<p class="faint small">Open an editable copy and let the mount images finish loading to make local grain crops.</p>{/if}
      <button class="primary" onclick={generatePickList} disabled={!!app.busy || !pickRows.length || !pickPixelsReady} data-testid="picklist-preview">Preview pick list</button>
      {#if result && madeControls?.style === 'picklist'}
        <p class="faint small" data-testid="picklist-count">{result.items} rows · {result.pages.length} pages</p>
        <div class="two"><button onclick={pdf} disabled={exportBlocked} title={exportReason} data-testid="export-pdf">Save PDF</button><button onclick={browserPrint} disabled={exportBlocked} title={exportReason}>Print…</button></div>
      {/if}
    {:else if isSheetsStyle(style)}
      <label class="field">Print
        <select aria-label="Print" bind:value={sheetMode} data-testid="sheet-mode" onchange={() => { sheets = null; whole = null; result = null; }}>
          <option value="grains">Grain sheets</option>
          <option value="whole">Whole area, one sheet</option>
        </select>
      </label>
      {#if sheetMode === 'grains'}
      <p class="muted small-text">Grains are printed in continuous pieces of the mosaic with their neighbours. Where a piece must be cut, a greyed strip repeats the other side.</p>
      {/if}
      <label class="field">Title <input type="text" bind:value={sheet.title} placeholder={app.pkg?.name ?? app.project?.package.name ?? ''} /></label>
      <label class="field">Lab and operator <input type="text" bind:value={sheet.lab} placeholder="e.g. lab name · operator" /></label>
      <div class="two">
        <label class="field">Paper
          <select aria-label="Paper" bind:value={sheet.paper}>
            <option value="tabloid">Tabloid 11 × 17 in</option>
            <option value="letter">Letter</option>
            <option value="a3">A3</option>
            <option value="a4">A4</option>
          </select>
        </label>
        {#if sheetMode === 'whole'}
        <label class="field">Scale (µm per mm) <input type="number" min="0" step="0.5" bind:value={() => typeof wholeScale === 'number' ? wholeScale : undefined, (v) => wholeScale = v && v > 0 ? v : 'auto'} placeholder="auto (fit area)" data-testid="whole-scale" /></label>
        {:else}
        <label class="field">Scale (µm per mm) <input type="number" min="0" step="0.5" bind:value={() => (typeof sheet.um_per_mm === 'number' && sheet.um_per_mm > 0 ? sheet.um_per_mm : undefined), (value) => (sheet.um_per_mm = value && value > 0 ? value : 'auto')} placeholder="auto (from grain size)" data-testid="sheet-scale" title="Empty: the median grain prints about 15 mm across, on a round scale." /></label>
        <label class="field" style="grid-column: 1 / -1">Arrangement
          <select bind:value={sheet.packing} data-testid="sheet-packing">
            <option value="lines">In lines, mount order</option>
            <option value="packed">Packed, fewer sheets</option>
          </select>
        </label>
        {/if}
      </div>
      {#if sheetMode === 'whole'}
      <label class="field">Area
        <select aria-label="Area" bind:value={wholeArea} data-testid="whole-area">
          {#each stitchedIds as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? app.project?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
        </select>
      </label>
      <div class="two">
        <label class="field">Orientation
          <select aria-label="Orientation" bind:value={orientation} data-testid="whole-orientation">
            <option value="auto">Auto</option><option value="landscape">Landscape</option><option value="portrait">Portrait</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={grid} data-testid="whole-grid" /> grid</label>
      </div>
      {:else}
      <div class="two">
        <label class="field">Smallest grain (µm) <input type="number" min="0" bind:value={sheet.min_grain_um} /></label>
        <label class="field">Keep grains together within (µm) <input type="number" min="0" step="10" bind:value={() => sheet.context_um || undefined, (value) => sheet.context_um = value ?? 0} placeholder="auto (3 grain sizes)" data-testid="sheet-context" title="Grains closer than this stay in one band, and a larger value costs paper." /></label>
      </div>
      <label class="field">Laser spot sizes (µm) <input type="text" bind:value={spotText} /></label>
      <div class="two">
        <label class="check"><input type="checkbox" bind:checked={sheet.enhance.denoise} /> denoise (NLM)</label>
        <label class="check"><input type="checkbox" bind:checked={sheet.enhance.sharpen} /> sharpen</label>
      </div>
      <label class="check" title="Only where every grain in a band has a closed outline and nothing else stands out; otherwise that band keeps its background"><input type="checkbox" bind:checked={sheet.enhance.strip} data-testid="strip-bg" /> strip background to one grey</label>
      {#if sheet.enhance.strip}
        <label class="field">Grey <span class="num">{sheet.enhance.strip_grey}</span>
          <input type="range" min="0" max="255" bind:value={sheet.enhance.strip_grey} />
          <span class="swatch" style="background: rgb({sheet.enhance.strip_grey},{sheet.enhance.strip_grey},{sheet.enhance.strip_grey})"></span>
        </label>
      {/if}
      {/if}
      <button class="primary" onclick={generateSheets} disabled={!!app.busy || (sheetMode === 'whole' ? !wholeArea : !hasGrains)} title={sheetMode === 'grains' && !hasGrains ? 'Find grains first' : ''} data-testid="sheets">{sheetMode === 'whole' ? 'Make whole-area sheet' : 'Make print sheets'}</button>
      {#if whole}
        <p class="faint small" data-testid="whole-stats">1 sheet, {whole.stats.orientation}, <b>1 mm = {whole.registration.um_per_mm} µm</b>. Source pixel {whole.stats.source_pixel_um.toFixed(2)} µm; smallest detail about {Math.round(whole.stats.smallest_detail_300_um)} µm at 300 dpi, {Math.round(whole.stats.smallest_detail_600_um)} µm at 600 dpi.</p>
      {/if}
      {#if sheets}
        {@const st = sheets.stats}
        {@const packed = sheets.registration.layout === 'packed' || sheets.registration.layout === 'edited'}
        <p class="muted" data-testid="sheet-stats">{st.sheets} sheet{st.sheets === 1 ? '' : 's'}, {sheets.registration.layout === 'edited' ? 'edited' : packed ? 'packed' : 'in lines'}, {st.bands} bands, {st.grains} grains at <b>1 mm = {sheets.registration.um_per_mm} µm</b>{sheets.registration.auto_scale ? ` (auto: median grain ${Math.round(sheets.registration.auto_scale.median_um)} µm prints ${(sheets.registration.auto_scale.median_um / sheets.registration.um_per_mm).toFixed(1)} mm)` : ''}{sheets.registration.spot_sizes_left_off_um?.length ? `, spot key without ${sheets.registration.spot_sizes_left_off_um.join(', ')} µm` : ''}; {st.duplicates} duplicates left to their owner area{st.artifacts ? `, ${st.artifacts} flagged artifacts left off` : ''}{st.filtered ? `, ${st.filtered} below the size limit` : ''}{st.strays ? `, ${st.strays} strays left off` : ''}{st.isolated ? `, ${st.isolated} grains with fewer than two neighbours in their band` : ''}{st.cuts ? `, ${st.cuts} cuts with repeated strips` : ''}{st.forced_cuts ? `, ${st.forced_cuts} forced cuts` : ''}{sheet.enhance.strip ? `; background stripped in ${st.stripped} bands, kept in ${st.strip_refused} for safety` : ''}. Check: {st.grains_cut ? `${st.grains_cut} grains cut by a band edge` : 'every grain whole in a band'}{st.grains_twice ? `, ${st.grains_twice} printed twice` : ''}{st.grains_unmarked ? `, ${st.grains_unmarked} grains visible but not marked` : ''}{st.bands_outside_body ? `, ${st.bands_outside_body} bands outside the sheet body` : ''}{st.bands_overlapping ? `, ${st.bands_overlapping} bands overlap` : ''}. {sheets.left_off_summary.cropped} grains left off by crop, {sheets.left_off_summary.hidden} in hidden strips.</p>
        {#if !editMode || app.readOnly}{@render droppedNotice(false)}{/if}
        <p class="faint small">Header {st.header_mm[0]} mm on sheet 1, {st.header_mm[1]} mm after; band fill {(st.fill.band_area * 100).toFixed(1)}%{packed ? '' : `, line width ${(st.fill.line_width * 100).toFixed(1)}%`}, repeated {(st.fill.repeated * 100).toFixed(1)}% (worst band {(st.worst_repeated_fraction * 100).toFixed(1)}%), empty rows inside bands {(st.fill.empty_epoxy * 100).toFixed(1)}%; {#if packed}blank on this sheet {((st.height_budget[pageIdx]?.blank_mm2 ?? 0) / 100).toFixed(1)} cm²{:else}blank below this sheet's last line {st.height_budget[pageIdx]?.blank_below_mm.toFixed(1) ?? '0'} mm{/if}.</p>
      {/if}
      {#if sheets || whole}
        <div class="two">
          <button onclick={pdf} disabled={exportBlocked} title={exportReason} data-testid="export-pdf">Export PDF</button>
          <button onclick={registration} disabled={exportBlocked} title={exportReason} data-testid="export-registration">Registration</button>
          <button onclick={browserPrint} disabled={exportBlocked} title={exportReason}>Print…</button>
        </div>
      {/if}
    {:else}
    <label class="field">Title <input type="text" bind:value={params.title} /></label>
    <div class="two">
      <label class="field">Paper
        <select aria-label="Paper" bind:value={params.paper} data-testid="paper">
          <option value="tabloid">Tabloid 11 × 17 in</option>
          <option value="letter">Letter</option>
          <option value="a4">A4</option>
          <option value="a3">A3</option>
          <option value="custom">custom</option>
        </select>
      </label>
      <label class="check"><input type="checkbox" bind:checked={params.landscape} /> landscape</label>
    </div>
    {#if params.paper === 'custom'}
      <div class="two">
        <label class="field">Width mm <input type="number" min="50" bind:value={params.custom_w_mm} /></label>
        <label class="field">Height mm <input type="number" min="50" bind:value={params.custom_h_mm} /></label>
      </div>
    {/if}
    <div class="two">
      <label class="field">Scale (µm per mm, 0 = fit) <input type="number" min="0" step="1" bind:value={params.um_per_mm} data-testid="scale" /></label>
      <label class="field">Margin mm <input type="number" min="0" bind:value={params.margin_mm} /></label>
    </div>
    <label class="field">Items
      <select aria-label="Items" bind:value={params.items} data-testid="items">
        <option value="grains" disabled={!hasGrains}>single grains, cropped alone (no neighbours)</option>
        <option value="areas">whole areas</option>
      </select>
    </label>
    {#if params.items === 'grains'}
      <label class="field">Margin around grains (µm) <input type="number" min="0" bind:value={params.grain_margin_um} /></label>
      <label class="check"><input type="checkbox" bind:checked={params.by_number} /> in grain-number order (else packed by height)</label>
    {/if}
    <fieldset>
      <legend class="muted">Areas</legend>
      {#each stitchedIds as id (id)}
        <label class="check"><input type="checkbox" checked={!params.area_ids.length || params.area_ids.includes(id)} onchange={() => toggleArea(id)} /> {app.pkg?.areas.find((a) => a.id === id)?.name ?? app.project?.areas.find((a) => a.id === id)?.name ?? id}</label>
      {/each}
    </fieldset>
    <div class="two">
      <label class="check"><input type="checkbox" bind:checked={params.show_spots} /> spots</label>
      <label class="check"><input type="checkbox" bind:checked={params.show_labels} /> labels</label>
      <label class="check"><input type="checkbox" bind:checked={params.show_outlines} /> outlines</label>
      <label class="check"><input type="checkbox" bind:checked={params.cut_marks} /> cut marks</label>
    </div>
    <button class="primary" onclick={generate} disabled={!!app.busy} data-testid="layout">Lay out pages</button>
    {#if result}
      <p class="muted">{result.pages.length} page{result.pages.length === 1 ? '' : 's'}, {result.items} items at <b>1 mm = {result.um_per_mm} µm</b></p>
      <div class="two">
        <button onclick={pdf} disabled={exportBlocked} title={exportReason} data-testid="export-pdf">Export PDF</button>
        <button onclick={browserPrint} disabled={exportBlocked} title={exportReason}>Print…</button>
      </div>
    {/if}
    {/if}
    {/if}
    </div>
    {#if sheets && isSheetsStyle(style) && sheetMode === 'grains' && editMode && editor.arrange}
      <footer class="arrange-export" data-testid="arrange-footer">
        <p class="faint small" data-testid="print-quality" data-ready={qualityState?.ready??true}>{pending?'Rendering pieces':qualityState?.error?qualityState.error.message:qualityState?.ready||!qualityState?'Print quality ready':qualityCancelled?'Print quality paused; export resumes it.':`Preparing print quality: page ${Math.min(qualityState.next+1,qualityState.total)} of ${qualityState.total}`}</p>
        {@render qualityProgress()}
        <div class="two"><button class="primary" onclick={pdf} disabled={exportBlocked} title={exportReason} data-testid="export-pdf">Save PDF</button><button onclick={browserPrint} disabled={exportBlocked} title={exportReason}>Print…</button></div>
        <button onclick={registration} disabled={exportBlocked} title={exportReason} data-testid="export-registration">Registration</button>
      </footer>
    {/if}
  </aside>
  <section class="preview" data-testid="layout-view">
    {#if sheets && automaticPlan && editorControls && isSheetsStyle(style) && sheetMode === 'grains'}
      {#key automaticPlan}
        <StripEditor bind:this={stripEditor} onautomatic={()=>{if(editor.arrange){editor.arrange=false;editMode=false;}}} base={automaticPlan} result={sheets} {urls} controls={editorControls} {droppedNotice} {history} bind:editorState={editor} storedDropped={savedDrops.length} bind:page={pageIdx} enabled={editMode && !moveLabels && !app.readOnly} onresult={showSheets} onpending={v => pending = v} onchange={storeHistory} />
      {/key}
    {/if}
    {#if result && urls[pageIdx] && !(editMode && sheets && isSheetsStyle(style) && sheetMode === 'grains')}
      {@const pw = result.paper_mm[0] * PX_PER_MM}
      {@const ph = result.paper_mm[1] * PX_PER_MM}
      <PanZoom bind:this={panzoom} w={pw} h={ph} fitKey="{pageIdx}/{result.paper_mm.join(',')}" label="Page preview">
        <img class="page" src={urls[pageIdx]} alt="Page {pageIdx + 1}" width={pw} height={ph} data-testid="page" />
        {#if style === 'spots' && moveLabels}
          <SpotLabelOverlay report={labelReports.find(p => p.sheet === pageIdx + 1)} paper={result.paper_mm} pixelsPerMM={PX_PER_MM} onpin={savePin} />
        {/if}
      </PanZoom>
      <div class="pager glass" data-ui>
        <button class="icon" disabled={pageIdx === 0} onclick={() => pageIdx--}>‹</button>
        <span class="num">{pageIdx + 1} / {result.pages.length}</span>
        <button class="icon" data-testid="next-page" disabled={pageIdx >= result.pages.length - 1} onclick={() => pageIdx++}>›</button>
        <button class="icon" title="Zoom out" onclick={() => panzoom?.zoomBy(1 / 1.5)}>−</button>
        <button class="icon" title="Zoom in" onclick={() => panzoom?.zoomBy(1.5)}>+</button>
        <button class="icon" title="Fit (double-click)" onclick={() => panzoom?.fitAll()}>Fit</button>
      </div>
    {:else if !result}
      <div class="glass panel empty"><p class="muted">Choose paper and scale, then lay out pages. Every page carries a 10 mm bar at paper scale and a µm bar at image scale, so the print scale can be checked with a ruler.</p></div>
    {/if}
  </section>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 21rem minmax(0, 1fr);
    min-height: 0;
    gap: var(--space-2);
    height: 100%;
    padding: 0 var(--space-2) var(--space-2);
  }
  .layout.folded { grid-template-columns: 2.5rem minmax(0, 1fr); }
  .concealed { display: none !important; }
  .folded-side { padding: var(--space-1); }
  .fold { align-self: flex-end; }
  .side {
    min-width: 0;
    min-height: 0;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: auto;
  }
  /* Automatic retains the original sidebar scroll owner. Arrange alone needs
     an inner scroller so its export footer stays outside the scrolling content. */
  .side.arranging {padding:0;gap:0;overflow:hidden;}
  .side-content {display:contents;}
  .side-content.arranging {
    min-height: 0;
    flex: 1;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: auto;
  }
  .side-content > :global(*) { flex-shrink:0; }
  .arrange-export {padding:var(--space-2) var(--space-4) var(--space-4);display:grid;gap:var(--space-1);flex:none;background:var(--bg);border-top:1px solid var(--hairline);}
  .arrange-export p {margin:var(--space-1) 0;}
  .quality-progress {display:flex;align-items:center;gap:var(--space-2);}
  .quality-progress progress {flex:1;min-width:0;accent-color:var(--accent);}
  .quality-progress button {padding:var(--space-1) var(--space-2);}
  .two {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-2);
    align-items: end;
  }
  fieldset {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
    display: grid;
    gap: var(--space-1);
  }
  .preview {
    position: relative;
    overflow: hidden;
    display: grid;
    justify-items: center;
    align-content: start;
    border-radius: var(--radius);
  }
  .page {
    display: block;
    background: white;
    box-shadow: var(--glass-shadow);
  }
  .seg {
    display: flex;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .swatch {
    display: inline-block;
    width: 1.2rem;
    height: 1.2rem;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
    vertical-align: middle;
  }
  .pager {
    position: absolute;
    top: var(--space-2);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-1) var(--space-3);
    background: var(--glass-strong);
  }
  .empty {
    max-width: 32rem;
    margin-top: 20vh;
  }
</style>
