<script lang="ts">
  // Print layout: paper, scale, packing, preview, PDF / SVG export and the
  // browser print dialog as a fallback. Two styles: print sheets (every grain of
  // the mount once, at a fixed scale, with a mount map and the print_sheet
  // registration for hand spot registration) and the grain / area layout.
  import { app } from '../lib/store.svelte';
  import { download } from '../lib/files';
  import { wholePrintOptions } from '../lib/whole-print';
  import type { Grain, LayoutParams, LayoutResult, SheetParams, SheetPlan, SheetResult, WholeParams, WholeResult, Spot, Label, ToneParams } from '../lib/core/types';
  import StripEditor from '../components/StripEditor.svelte';
  import { StripHistory } from '../lib/strip-edit';
  import { onDestroy } from 'svelte';
  import PanZoom from '../lib/canvas/PanZoom.svelte';

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
  let whole = $state<WholeResult | null>(null);
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
  });
  let spotText = $state('15, 20, 25, 30, 35, 50');
  let sheets = $state<SheetResult | null>(null);
  let result = $state<LayoutResult | null>(null);
  let pageIdx = $state(0);
  let urls = $state<string[]>([]);
  const history = new StripHistory();
  let automaticPlan = $state<SheetPlan | null>(null);
  let editMode = $state(false);
  let pending = $state(false);
  let editorControls = $state<HTMLElement>();
  const exportBlocked = $derived(pending || !!sheets?.problems?.length);
  const exportReason = $derived(pending ? 'Wait for the strip preview' : exportBlocked ? 'Fix strip problems before export' : '');
  onDestroy(() => { for (const url of urls) URL.revokeObjectURL(url); });
  function showSheets(r: SheetResult) {
    sheets = r;
    replacePages(r.pages);
    result = { pages: r.pages, um_per_mm: r.registration.um_per_mm, items: r.stats.bands, paper_mm: r.registration.paper_mm };
  }
  function replacePages(pages: string[]) {
    const old = urls;
    urls = pages.map(s => URL.createObjectURL(new Blob([s], { type: 'image/svg+xml' })));
    for (const url of old) URL.revokeObjectURL(url);
  }
  const hasGrains = $derived(stitchedIds.some((id) => (app.grains[id] ?? []).length > 0));

  $effect(() => {
    if (!stitchedIds.includes(wholeArea)) wholeArea = stitchedIds[0] ?? '';
    if (!params.title) params.title = app.pkg?.name ?? app.project?.package.name ?? '';
    if (!hasGrains && params.items === 'grains') params.items = 'areas';
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
    p.printed = new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
    const tones = $state.snapshot(app.tone) as Record<string, ToneParams>;
    const previousPlan = automaticPlan;
    const r = sheetMode === 'whole'
      ? await app.run('Laying out whole area', () => core.printWhole(wholePrintOptions(p, { areaId: wholeArea, orientation, scale: wholeScale, grid }), tones))
      : await app.run('Laying out print sheets', () => core.printSheets(p, $state.snapshot(app.grains) as Record<string, Grain[]>, tones));
    if (!r) return;
    if ('ownership' in r) {
      app.ownership = r.ownership;
      whole = null;
      const replayed = history.count && previousPlan
        ? await app.run('Replaying strip edits', () => core.printPlanRender($state.snapshot(r.plan) as SheetPlan, { params: $state.snapshot(previousPlan.params) as SheetParams, um_per_mm: previousPlan.um_per_mm, edits: $state.snapshot(history.edits) }, $state.snapshot(app.grains) as Record<string, Grain[]>, tones))
        : r;
      if (replayed?.dropped?.length) history.rebase(replayed.dropped.map(d => d.index));
      automaticPlan = r.plan;
      pending = false;
      showSheets(replayed ?? r);
    } else {
      whole = r;
      sheets = null;
      replacePages(r.pages);
      result = { pages: r.pages, um_per_mm: r.registration.um_per_mm, items: 1, paper_mm: r.registration.paper_mm };
    }
    pageIdx = 0;
  }
  function registration() {
    const r = whole ?? sheets;
    if (r && !exportBlocked) download(JSON.stringify(r.registration, null, 1), r.registration.registration_file || r.registration.package + '_print_registration.json', 'application/json');
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
    const r = await app.run('Laying out pages', () =>
      core.layout(p, $state.snapshot(app.grains) as Record<string, Grain[]>, $state.snapshot(app.spots) as Spot[], $state.snapshot(app.labels) as Label[], $state.snapshot(app.tone) as Record<string, ToneParams>),
    );
    if (!r) return;
    for (const u of urls) URL.revokeObjectURL(u);
    urls = r.pages.map((s) => URL.createObjectURL(new Blob([s], { type: 'image/svg+xml' })));
    result = r;
    pageIdx = 0;
  }

  async function pdf() {
    if (!result || exportBlocked) return;
    const core = await app.core;
    const pages = $state.snapshot(result.pages) as string[];
    const bytes = await app.run('Writing PDF', () => core.pdf(pages));
    const what = style === 'sheets' ? `${whole ? 'whole_area' : 'grain_sheets'}_${sheet.paper}` : `${params.items}_${params.paper}`;
    if (bytes) download(bytes, `${app.pkg?.name ?? app.project?.package.name ?? 'grainatlas'}_${what}.pdf`, 'application/pdf');
  }
  function svgs() {
    if (exportBlocked) return;
    result?.pages.forEach((s, i) => download(s, `${app.pkg?.name ?? app.project?.package.name ?? 'grainatlas'}_page${i + 1}.svg`, 'image/svg+xml'));
  }
  function browserPrint() {
    if (!result || exportBlocked) return;
    const [w, h] = result.paper_mm;
    const win = window.open('', '_blank');
    if (!win) {
      app.error = 'The print window was blocked; allow pop-ups or use the PDF.';
      return;
    }
    win.document.write(`<!doctype html><title>${style === 'sheets' ? sheet.title || app.pkg?.name || app.project?.package.name : params.title}</title><style>@page{size:${w}mm ${h}mm;margin:0}body{margin:0}svg{display:block;width:${w}mm;height:${h}mm;page-break-after:always}</style>${result.pages.join('')}`);
    win.document.close();
    win.focus();
    setTimeout(() => win.print(), 300);
  }
  function toggleArea(id: string) {
    params.area_ids = params.area_ids.includes(id) ? params.area_ids.filter((a) => a !== id) : [...params.area_ids, id];
  }
</script>

<div class="layout" class:folded={app.layout.printCollapsed}>
  {#if app.layout.printCollapsed}
    <aside class="glass folded-side"><button class="icon" title="Show print controls" aria-label="Show print controls" onclick={() => { app.layout.printCollapsed = false; app.saveLayout(); }}>»</button></aside>
  {/if}
  <aside class="glass side" class:concealed={app.layout.printCollapsed}>
    <button class="icon fold" title="Fold print controls" aria-label="Fold print controls" onclick={() => { app.layout.printCollapsed = true; app.saveLayout(); }}>«</button>
    {#if sheets && style === 'sheets' && sheetMode === 'grains'}
      <label class="check"><input type="checkbox" bind:checked={editMode} data-testid="edit-strips" /> Edit strips</label>
    {/if}
    <div bind:this={editorControls}></div>
    <h2>Print layout</h2>
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
        {#if sheets.dropped?.length}<div class="faint small" data-testid="dropped-edits">{sheets.dropped.length} edits dropped.{#each sheets.dropped as dropped}<p>Edit {dropped.index + 1}: {dropped.reason}</p>{/each}</div>{/if}
        <p class="faint small">Header {st.header_mm[0]} mm on sheet 1, {st.header_mm[1]} mm after; band fill {(st.fill.band_area * 100).toFixed(1)}%{packed ? '' : `, line width ${(st.fill.line_width * 100).toFixed(1)}%`}, repeated {(st.fill.repeated * 100).toFixed(1)}% (worst band {(st.worst_repeated_fraction * 100).toFixed(1)}%), empty rows inside bands {(st.fill.empty_epoxy * 100).toFixed(1)}%; {#if packed}blank on this sheet {((st.height_budget[pageIdx]?.blank_mm2 ?? 0) / 100).toFixed(1)} cm²{:else}blank below this sheet's last line {st.height_budget[pageIdx]?.blank_below_mm.toFixed(1) ?? '0'} mm{/if}.</p>
      {/if}
      {#if sheets || whole}
        <div class="two">
          <button onclick={pdf} disabled={exportBlocked} title={exportReason} data-testid="export-pdf">Export PDF</button>
          <button onclick={registration} disabled={exportBlocked} title={exportReason} data-testid="export-registration">Registration</button>
          <button onclick={svgs} disabled={exportBlocked} title={exportReason}>Export SVG</button>
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
        <button onclick={svgs} disabled={exportBlocked} title={exportReason}>Export SVG</button>
        <button onclick={browserPrint} disabled={exportBlocked} title={exportReason}>Print…</button>
      </div>
    {/if}
    {/if}
  </aside>
  <section class="preview" data-testid="layout-view">
    {#if sheets && automaticPlan && editorControls && style === 'sheets' && sheetMode === 'grains'}
      {#key automaticPlan}
        <StripEditor base={automaticPlan} result={sheets} {urls} controls={editorControls} {history} bind:page={pageIdx} enabled={editMode} onresult={showSheets} onpending={v => pending = v} />
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
        <button class="icon" disabled={pageIdx >= result.pages.length - 1} onclick={() => pageIdx++}>›</button>
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
