<script lang="ts">
  // Print layout: paper, scale, packing, preview, PDF / SVG export and the
  // browser print dialog as a fallback.
  import { app } from '../lib/store.svelte';
  import { download } from '../lib/files';
  import type { Grain, LayoutParams, LayoutResult, Spot, Label, ToneParams } from '../lib/core/types';

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
  let result = $state<LayoutResult | null>(null);
  let pageIdx = $state(0);
  let urls = $state<string[]>([]);
  const hasGrains = $derived(stitchedIds.some((id) => (app.grains[id] ?? []).length > 0));

  $effect(() => {
    if (!params.title && app.pkg) params.title = app.pkg.name;
    if (!hasGrains && params.items === 'grains') params.items = 'areas';
  });

  async function generate() {
    const core = await app.core;
    const p = $state.snapshot(params) as LayoutParams;
    if (!p.area_ids.length) p.area_ids = stitchedIds;
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
    if (bytes) download(bytes, `${app.pkg?.name ?? 'grainatlas'}_${params.items}_${params.paper}.pdf`, 'application/pdf');
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
    win.document.write(`<!doctype html><title>${params.title}</title><style>@page{size:${w}mm ${h}mm;margin:0}body{margin:0}svg{display:block;width:${w}mm;height:${h}mm;page-break-after:always}</style>${result.pages.join('')}`);
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
  </aside>
  <section class="preview">
    {#if result && urls[pageIdx]}
      <div class="pager glass">
        <button class="icon" disabled={pageIdx === 0} onclick={() => pageIdx--}>‹</button>
        <span class="num">{pageIdx + 1} / {result.pages.length}</span>
        <button class="icon" disabled={pageIdx >= result.pages.length - 1} onclick={() => pageIdx++}>›</button>
      </div>
      <img class="page" src={urls[pageIdx]} alt="Page {pageIdx + 1}" style:aspect-ratio="{result.paper_mm[0]} / {result.paper_mm[1]}" data-testid="page" />
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
    overflow: auto;
    display: grid;
    justify-items: center;
    align-content: start;
    padding: var(--space-4);
    gap: var(--space-2);
  }
  .page {
    height: calc(100vh - 9rem);
    max-width: 100%;
    background: white;
    box-shadow: var(--glass-shadow);
  }
  .pager {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-1) var(--space-3);
  }
  .empty {
    max-width: 32rem;
    margin-top: 20vh;
  }
</style>
