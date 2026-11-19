<script lang="ts">
  // Print layout: paper, scale, packing, preview, PDF export and the
  // browser print dialog as a fallback. Two styles: print sheets (every grain of
  // the mount once, at a fixed scale, with a mount map and the print_sheet
  // registration for hand spot registration) and the grain / area layout.
  import { current } from '../lib/store.svelte';
  import { download, savePrintPdf } from '../lib/files';
  import { wholePrintOptions } from '../lib/whole-print';
  import type { DroppedEdit, Grain, LayoutParams, LayoutResult, SheetParams, SheetPlan, SheetResult, WholeParams, WholeResult, Spot, Label, ToneParams } from '../lib/core/types';
  import StripEditor from '../components/StripEditor.svelte';
  import { readPrintEdits, storePrintEdits, StripHistory } from '../lib/strip-edit';
  import { onDestroy } from 'svelte';
  import { printCache, printFingerprint, revision, type PrintControls, type PrintEditorState } from '../lib/print-cache';
  import Busy from '../components/Busy.svelte';
  import { PrintQuality, printPdfName } from '../lib/print-quality';
  import PanZoom from '../lib/canvas/PanZoom.svelte';

  const app = current();

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
  let style = $state<'sheets' | 'layout'>('sheets');
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
  let result = $state.raw<LayoutResult | null>(null);
  let pageIdx = $state(0);
  let urls = $state<string[]>([]);
  let history = $state.raw(new StripHistory());
  let automaticPlan = $state.raw<SheetPlan | null>(null);
  let editMode = $state(false);
  let editor = $state<PrintEditorState>({ selection: null, splitting: false, axis: 'x' });
  let pending = $state(false);
  let savedDrops = $state<DroppedEdit[]>([]);
  const droppedEdits = $derived(savedDrops.length ? savedDrops : sheets?.dropped ?? []);
  let editorControls = $state<HTMLElement>();
  let quality = $state.raw<PrintQuality | null>(null);
  let qualityRevision = $state(0);
  let qualityUnsubscribe: (() => void) | undefined;
  let destroyed = false;
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
    return $state.snapshot({ params, sheet, style, sheetMode, orientation, wholeScale, grid, wholeArea, spotText });
  }
  function fingerprint(c = controls()) {
    const ids = c.style === 'sheets' && c.sheetMode === 'whole' ? [c.wholeArea]
      : c.style === 'layout' && c.params.area_ids.length ? c.params.area_ids : Object.keys(app.stitched);
    return printFingerprint({
      identity: [app.id, app.project?.uuid, app.projectPath, revision(app.packageSource), revision(app.pkg),
        app.project?.package, app.project?.revision, app.pixelsVersion,
        app.remote?.server, app.remote?.projectId, app.remote?.version, app.sync.baseRevision],
      controls: c,
      areas: app.project?.areas.filter(a => ids.includes(a.id)).map(a => [a.id, a.uuid, a.name, a.sample, a.pixel_size_nm,
        revision(a.tiles), revision(a.placement), revision(app.stitched[a.id]?.placement),
        a.stage_sign, a.mosaic, app.stitched[a.id]?.mosaic, a.params, app.params[a.id], a.area_meta]),
      tone: Object.fromEntries(ids.map(id => [id, app.tone[id]])),
      grains: Object.fromEntries(ids.map(id => [id, app.grains[id]])),
      spots: c.style === 'layout' && c.params.show_spots ? app.spots : undefined,
      labels: c.style === 'layout' && c.params.show_labels ? app.labels : undefined,
      edits: app.project?.print_sheets,
    });
  }
  let madeWith = $state('');
  let madeControls: PrintControls | null = null;
  function restoreCached() {
    const cached = printCache.take(String(app.id));
    if (cached) {
      ({ params, sheet, style, sheetMode, orientation, wholeScale, grid, wholeArea, spotText } = cached.controls);
      if (cached.fingerprint === fingerprint()) {
        ({ sheets, whole, result, automaticPlan, history, savedDrops, pageIdx, editMode, editor, urls, quality } = cached);
        madeWith = cached.fingerprint;
        madeControls = cached.controls;
      } else { cached.quality?.release(); for (const url of cached.urls) url && URL.revokeObjectURL(url); }
    }
  }
  restoreCached();
  watchQuality();
  onDestroy(() => {
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
    if (madeControls) madeWith = fingerprint(madeControls);
  }
  function replacePages(pages: string[]) {
    const old = urls;
    urls = pages.map(() => '');
    for (const url of old) url && URL.revokeObjectURL(url);
  }
  // Only materialise a Blob for a page the viewer actually visits. Full-quality
  // pages stay in the core until PDF export or browser printing.
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

  async function generateSheets() {
    if (app.busy) return;
    const { liveIdle } = await import('../lib/actions');
    for (const id of stitchedIds) await liveIdle(id);
    const core = await app.core;
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
    const r = sheetMode === 'whole'
      ? await app.run('Laying out whole area', () => core.printWhole(wholePrintOptions(p, { areaId: wholeArea, orientation, scale: wholeScale, grid }), tones))
      : await app.run('Laying out print sheets', () => core.printSheets(p, $state.snapshot(app.grains) as Record<string, Grain[]>, tones));
    if (!r) return;
    if ('ownership' in r) {
      app.ownership = r.ownership;
      whole = null;
      const replayed = stored?.edits.length
        ? await app.run('Replaying strip edits', () => core.printPlanRender($state.snapshot(r.plan) as SheetPlan, stored, $state.snapshot(app.grains) as Record<string, Grain[]>, tones))
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
    let cancelled = false;
    app.cancelTask = () => { cancelled = true; cancelQuality(); };
    try {
      await app.run('Exporting PDF', async progress => {
        const bytes = quality ? await quality.pdf(progress, () => cancelled || destroyed || stale) : await core.pdf(result!.pages);
        if (cancelled || destroyed || stale) throw new Error('cancelled');
        progress({ stage: 'Saving PDF', fraction: 1 });
        const r = whole ?? sheets;
        await savePrintPdf(bytes, printPdfName(app.pkg?.name ?? app.project?.package.name ?? 'grainatlas'), r ? JSON.stringify(r.registration, null, 1) : null);
      });
    } finally { app.cancelTask = null; }
  }
  async function browserPrint() {
    if (!result || exportBlocked) return;
    const win = window.open('', '_blank');
    if (!win) { app.error = 'The print window was blocked; allow pop-ups or use the PDF.'; return; }
    let cancelled = false;
    app.cancelTask = () => { cancelled = true; cancelQuality(); };
    try {
      const core = await app.core;
      const pages = await app.run('Preparing print quality', async progress => {
        if (quality) await quality.wait(progress, () => cancelled || destroyed || stale);
        return quality ? core.printPages(quality.job) : result!.pages;
      });
      if (!pages || cancelled || destroyed || stale) { win.close(); return; }
      const [w, h] = result.paper_mm;
      win.document.title = style === 'sheets' ? sheet.title || app.pkg?.name || app.project?.package.name || 'Print sheets' : params.title;
      win.document.body.innerHTML = `<style>@page{size:${w}mm ${h}mm;margin:0}body{margin:0}svg{display:block;width:${w}mm;height:${h}mm;page-break-after:always}</style>${pages.join('')}`;
      await Promise.all(Array.from(win.document.querySelectorAll('svg image'), async node => {
        const img = win.document.createElement('img');
        const loaded = new Promise<void>((resolve, reject) => { img.onload = () => resolve(); img.onerror = () => reject(new Error('A print image could not be loaded')); });
        void loaded.catch(() => {});
        img.src = node.getAttribute('href') ?? node.getAttributeNS('http://www.w3.org/1999/xlink', 'href') ?? '';
        await (img.decode ? img.decode() : loaded);
      }));
      if (cancelled || destroyed || stale) { win.close(); return; }
      win.focus(); win.print();
    } catch (error) { win.close(); app.error = error instanceof Error ? error.message : String(error); }
    finally { app.cancelTask = null; }
  }
  function toggleArea(id: string) {
    params.area_ids = params.area_ids.includes(id) ? params.area_ids.filter((a) => a !== id) : [...params.area_ids, id];
  }
</script>

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

{#if qualityState && !qualityState.ready && !qualityCancelled && !qualityState.error && !stale && !pending && !app.busy}
  <Busy task={{ label: 'Preparing print quality', progress: { stage: `page ${Math.min(qualityState.next + 1, qualityState.total)} of ${qualityState.total}`, fraction: qualityState.next / qualityState.total } }} onCancel={cancelQuality} testId="print-quality-busy" />
{/if}
<div class="layout" class:folded={app.layout.printCollapsed}>
  {#if app.layout.printCollapsed}
    <aside class="glass folded-side"><button class="icon" title="Show print controls" aria-label="Show print controls" onclick={() => { app.layout.printCollapsed = false; app.saveLayout(); }}>»</button></aside>
  {/if}
  <aside class="glass side" class:concealed={app.layout.printCollapsed}>
    <button class="icon fold" title="Fold print controls" aria-label="Fold print controls" onclick={() => { app.layout.printCollapsed = true; app.saveLayout(); }}>«</button>
    {#if sheets && style === 'sheets' && sheetMode === 'grains'}
      <label class="check"><input type="checkbox" bind:checked={editMode} disabled={app.readOnly} data-testid="edit-strips" /> Edit strips</label>
    {/if}
    <div bind:this={editorControls}></div>
    <h2>Print layout</h2>
    {#if result}
      <p class="faint small" data-testid="preview-note">Preview uses reduced resolution without export enhancement. PDF and print use full quality.</p>
      {#if stale}<p class="faint small">Inputs changed; make sheets again.</p>{/if}
      <p class="faint small" data-testid="print-quality" data-ready={qualityState?.ready ?? true}>{qualityState?.error ? qualityState.error.message : qualityState?.ready || !qualityState ? 'Print quality ready' : qualityCancelled ? 'Print quality paused; export resumes it.' : `Preparing print quality: page ${Math.min(qualityState.next + 1, qualityState.total)} of ${qualityState.total}`}</p>
    {/if}
    {#if sheets?.problems?.length}
      <p class="faint small" role="status" data-testid="export-blocked">Export blocked: {sheets.problems[0].message}. {sheets.problems.some(p => p.kind === 'outside_body') ? 'Increase the scale value or paper size, or edit the strips.' : 'Fix the strip problems before exporting.'}</p>
    {/if}
    {#if unreadablePrint}<p class="faint small" data-testid="unreadable-print-edits">Stored strip edits could not be read. They will be kept when saving. A new strip edit replaces them.</p>{/if}
    <div class="seg" role="radiogroup" aria-label="Style">
      <label class="check"><input type="radio" bind:group={style} value="sheets" data-testid="style-sheets" /> Print sheets</label>
      <label class="check"><input type="radio" bind:group={style} value="layout" data-testid="style-layout" /> Grains or areas</label>
    </div>
    {#if style === 'sheets'}
      <label class="field">Print
        <select bind:value={sheetMode} data-testid="sheet-mode" onchange={() => { sheets = null; whole = null; result = null; }}>
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
          <select bind:value={sheet.paper}>
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
        <select bind:value={wholeArea} data-testid="whole-area">
          {#each stitchedIds as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? app.project?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
        </select>
      </label>
      <div class="two">
        <label class="field">Orientation
          <select bind:value={orientation} data-testid="whole-orientation">
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
        <select bind:value={params.paper} data-testid="paper">
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
      <select bind:value={params.items} data-testid="items">
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
  </aside>
  <section class="preview" data-testid="layout-view">
    {#if sheets && automaticPlan && editorControls && style === 'sheets' && sheetMode === 'grains'}
      {#key automaticPlan}
        <StripEditor base={automaticPlan} result={sheets} {urls} controls={editorControls} {droppedNotice} {history} bind:editorState={editor} storedDropped={savedDrops.length} bind:page={pageIdx} enabled={editMode && !app.readOnly} onresult={showSheets} onpending={v => pending = v} onchange={storeHistory} />
      {/key}
    {/if}
    {#if result && urls[pageIdx] && !(editMode && sheets && style === 'sheets' && sheetMode === 'grains')}
      {@const pw = result.paper_mm[0] * PX_PER_MM}
      {@const ph = result.paper_mm[1] * PX_PER_MM}
      <PanZoom bind:this={panzoom} w={pw} h={ph} fitKey="{pageIdx}/{urls[pageIdx]}" label="Page preview">
        <img class="page" src={urls[pageIdx]} alt="Page {pageIdx + 1}" width={pw} height={ph} data-testid="page" />
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
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: auto;
  }
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
