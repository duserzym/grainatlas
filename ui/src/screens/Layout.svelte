<script lang="ts">
  // Print layout: paper, scale, packing, preview, PDF / SVG export and the
  // browser print dialog as a fallback. Two styles: print sheets (every grain of
  // the mount once, at a fixed scale, with a mount map and the print_sheet
  // registration for hand spot registration) and the grain / area layout.
  import { app } from '../lib/store.svelte';
  import { download } from '../lib/files';
  import type { Grain, LayoutParams, LayoutResult, SheetParams, SheetResult, Spot, Label, ToneParams } from '../lib/core/types';
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
    items: 'grains',
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
  let sheet = $state<SheetParams>({
    paper: 'tabloid',
    landscape: true,
    margin_mm: 8,
    um_per_mm: 8,
    pad_um: 22,
    gap_break_um: 420,
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
  const hasGrains = $derived(stitchedIds.some((id) => (app.grains[id] ?? []).length > 0));

  $effect(() => {
    if (!params.title && app.pkg) params.title = app.pkg.name;
    if (!hasGrains && params.items === 'grains') params.items = 'areas';
  });

  async function generateSheets() {
    const core = await app.core;
    const p = $state.snapshot(sheet) as SheetParams;
    const name = app.pkg?.name ?? app.project?.package.name ?? 'grainatlas';
    p.spot_sizes_um = spotText.split(/[,\s]+/).map(Number).filter((v) => v > 0);
    p.registration_file = `${name}_print_registration.json`;
    p.project = `${name}.grainatlas`;
    p.printed = new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
    const r = await app.run('Laying out print sheets', () => core.printSheets(p, $state.snapshot(app.grains) as Record<string, Grain[]>, $state.snapshot(app.tone) as Record<string, ToneParams>));
    if (!r) return;
    app.ownership = r.ownership;
    for (const u of urls) URL.revokeObjectURL(u);
    urls = r.pages.map((s) => URL.createObjectURL(new Blob([s], { type: 'image/svg+xml' })));
    sheets = r;
    result = { pages: r.pages, um_per_mm: r.registration.um_per_mm, items: r.stats.bands, paper_mm: r.registration.paper_mm };
    pageIdx = 0;
  }
  function registration() {
    if (sheets) download(JSON.stringify(sheets.registration, null, 1), sheets.registration.package + '_print_registration.json', 'application/json');
  }

  async function generate() {
    const core = await app.core;
    const p = $state.snapshot(params) as LayoutParams;
    if (!p.area_ids.length) p.area_ids = stitchedIds;
    sheets = null;
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
    if (!result) return;
    const core = await app.core;
    const pages = $state.snapshot(result.pages) as string[];
    const bytes = await app.run('Writing PDF', () => core.pdf(pages));
    const what = style === 'sheets' ? `grain_sheets_${sheet.paper}` : `${params.items}_${params.paper}`;
    if (bytes) download(bytes, `${app.pkg?.name ?? 'grainatlas'}_${what}.pdf`, 'application/pdf');
  }
  function svgs() {
    result?.pages.forEach((s, i) => download(s, `${app.pkg?.name ?? 'grainatlas'}_page${i + 1}.svg`, 'image/svg+xml'));
  }
  function browserPrint() {
    if (!result) return;
    const [w, h] = result.paper_mm;
    const win = window.open('', '_blank');
    if (!win) {
      app.error = 'The print window was blocked; allow pop-ups or use the PDF.';
      return;
    }
    win.document.write(`<!doctype html><title>${style === 'sheets' ? sheet.title || app.pkg?.name : params.title}</title><style>@page{size:${w}mm ${h}mm;margin:0}body{margin:0}svg{display:block;width:${w}mm;height:${h}mm;page-break-after:always}</style>${result.pages.join('')}`);
    win.document.close();
    win.focus();
    setTimeout(() => win.print(), 300);
  }
  function toggleArea(id: string) {
    params.area_ids = params.area_ids.includes(id) ? params.area_ids.filter((a) => a !== id) : [...params.area_ids, id];
  }
</script>

<div class="layout">
  <aside class="glass side">
    <h2>Print layout</h2>
    <div class="seg" role="radiogroup" aria-label="Style">
      <label class="check"><input type="radio" bind:group={style} value="sheets" data-testid="style-sheets" /> Print sheets</label>
      <label class="check"><input type="radio" bind:group={style} value="layout" data-testid="style-layout" /> Grains or areas</label>
    </div>
    {#if style === 'sheets'}
      <p class="muted small-text">Every grain of the mount once (duplicates in overlapping areas go to the area they lie deepest in), sample by sample, at one scale, as bands cut straight out of the mosaic. Each sheet has the instrument header, a mount map and laser spot sizes; the registration file maps every band on paper back to the mosaic.</p>
      <label class="field">Title <input type="text" bind:value={sheet.title} placeholder={app.pkg?.name ?? ''} /></label>
      <label class="field">Lab and operator <input type="text" bind:value={sheet.lab} placeholder="e.g. Boise State Isotope Geology Lab · Y. Zhang" /></label>
      <div class="two">
        <label class="field">Paper
          <select bind:value={sheet.paper}>
            <option value="tabloid">Tabloid 11 × 17 in</option>
            <option value="letter">Letter</option>
            <option value="a3">A3</option>
            <option value="a4">A4</option>
          </select>
        </label>
        <label class="field">Scale (µm per mm) <input type="number" min="1" step="1" bind:value={sheet.um_per_mm} data-testid="sheet-scale" /></label>
      </div>
      <div class="two">
        <label class="field">Smallest grain (µm) <input type="number" min="0" bind:value={sheet.min_grain_um} /></label>
        <label class="field">Mark gaps over (µm) <input type="number" min="50" step="10" bind:value={sheet.gap_break_um} /></label>
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
      <button class="primary" onclick={generateSheets} disabled={!hasGrains} title={hasGrains ? '' : 'Find grains first'} data-testid="sheets">Make print sheets</button>
      {#if sheets}
        {@const st = sheets.stats}
        <p class="muted" data-testid="sheet-stats">{st.sheets} sheet{st.sheets === 1 ? '' : 's'}, {st.bands} bands, {st.grains} grains at <b>1 mm = {sheets.registration.um_per_mm} µm</b>; {st.duplicates} duplicates left to their owner area{st.filtered ? `, ${st.filtered} below the size limit` : ''}{sheet.enhance.strip ? `; background stripped in ${st.stripped} bands, kept in ${st.strip_refused} for safety` : ''}.</p>
        <div class="two">
          <button onclick={pdf} data-testid="export-pdf">Export PDF</button>
          <button onclick={registration} data-testid="export-registration">Registration</button>
          <button onclick={svgs}>Export SVG</button>
          <button onclick={browserPrint}>Print…</button>
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
      <select bind:value={params.items}>
        <option value="grains" disabled={!hasGrains}>grains {hasGrains ? '' : '(find grains first)'}</option>
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
        <label class="check"><input type="checkbox" checked={!params.area_ids.length || params.area_ids.includes(id)} onchange={() => toggleArea(id)} /> {app.pkg?.areas.find((a) => a.id === id)?.name ?? id}</label>
      {/each}
    </fieldset>
    <div class="two">
      <label class="check"><input type="checkbox" bind:checked={params.show_spots} /> spots</label>
      <label class="check"><input type="checkbox" bind:checked={params.show_labels} /> labels</label>
      <label class="check"><input type="checkbox" bind:checked={params.show_outlines} /> outlines</label>
      <label class="check"><input type="checkbox" bind:checked={params.cut_marks} /> cut marks</label>
    </div>
    <button class="primary" onclick={generate} data-testid="layout">Lay out pages</button>
    {#if result}
      <p class="muted">{result.pages.length} page{result.pages.length === 1 ? '' : 's'}, {result.items} items at <b>1 mm = {result.um_per_mm} µm</b></p>
      <div class="two">
        <button onclick={pdf} data-testid="export-pdf">Export PDF</button>
        <button onclick={svgs}>Export SVG</button>
        <button onclick={browserPrint}>Print…</button>
      </div>
    {/if}
    {/if}
  </aside>
  <section class="preview" data-testid="layout-view">
    {#if result && urls[pageIdx]}
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
    {:else}
      <div class="glass panel empty"><p class="muted">Choose paper and scale, then lay out pages. Every page carries a 10 mm bar at paper scale and a µm bar at image scale, so the print scale can be checked with a ruler.</p></div>
    {/if}
  </section>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 21rem 1fr;
    gap: var(--space-2);
    height: 100%;
    padding: 0 var(--space-2) var(--space-2);
  }
  .side {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: auto;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
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
