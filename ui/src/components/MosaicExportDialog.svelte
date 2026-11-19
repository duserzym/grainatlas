<script lang="ts">
  import { onMount } from 'svelte';
  import { current } from '../lib/store.svelte';
  import type { Core, MosaicExportOptions, MosaicExportPlan } from '../lib/core/core';
  import type { Project } from '../lib/core/types';
  import { MOSAIC_BLOB_LIMIT, mosaicBytes, mosaicCanStream, sizeLabel } from '../lib/mosaic-export';
  import { saveMosaicExport } from '../lib/save-mosaic-export';
  import { bundlePixels } from '../lib/bundle-tone';
  import { loadPublishedPixels } from '../lib/published-pixels';
  let { areaId, oncancel }: { areaId: string | null; oncancel: () => void } = $props();
  const app = current();
  let scope = $state<'current' | 'chosen' | 'mount'>('current');
  let values = $state<'original' | 'displayed'>('original');
  let format = $state<'tiff' | 'png' | 'jpg'>('tiff');
  let pngBits = $state(16), quality = $state(90), sizeChoice = $state<'full' | 'downsample' | 'split'>('full');
  let chosen = $state<string[]>([]);
  let core = $state<Core | null>(null), project = $state<Project | null>(null);
  let plan = $state<MosaicExportPlan | null>(null), options = $state<MosaicExportOptions | null>(null);
  let error = $state(''), estimating = $state(true), exporting = $state(false), cancelling = $state(false);
  let dialog: HTMLDivElement;
  let revision = 0;
  const ids = $derived(scope === 'current' ? (areaId ? [areaId] : []) : scope === 'mount' ? (project?.areas.map(a => a.id) ?? []) : chosen);
  const browserStreaming = mosaicCanStream(window as unknown as { showDirectoryPicker?: unknown; showSaveFilePicker?: unknown });
  const streaming = $derived(core?.kind === 'tauri' || browserStreaming);
  const tooLarge = $derived(!!plan && !streaming && mosaicBytes(plan) >= MOSAIC_BLOB_LIMIT);
  const overLimit = $derived(plan?.images.some(a => a.limit_reason) ?? false);
  const progress = $derived(app.busy?.progress);
  onMount(() => {
    scope = areaId ? 'current' : 'chosen'; chosen = areaId ? [areaId] : [];
    let disposed = false;
    const previous = document.activeElement as HTMLElement | null;
    dialog.querySelector<HTMLElement>('select')?.focus();
    void (async () => {
      try {
        if (app.remote && !bundlePixels(app) && !(await loadPublishedPixels(app))) throw new Error('Load the published images before exporting the mosaic.');
        const { liveIdle } = await import('../lib/actions');
        for (const id of Object.keys(app.stitched)) await liveIdle(id, app);
        const c = await app.core, p = app.fullProject();
        if (!disposed) { core = c; project = p; if (!p) throw new Error('No project is open.'); }
      } catch (e) { if (!disposed) { error = e instanceof Error ? e.message : String(e); estimating = false; } }
    })();
    return () => { disposed = true; revision++; previous?.focus(); };
  });
  $effect(() => {
    const c = core, p = project, selected = [...ids], mode = format === 'jpg' ? 'displayed' : format === 'png' && pngBits === 16 ? 'original' : values, mount = scope === 'mount', fmt = format, depth = pngBits, q = quality, choice = sizeChoice;
    const tone = $state.snapshot(app.tone);
    if (!c || !p) return;
    const token = ++revision;
    plan = null; options = null; error = ''; estimating = true;
    if (!selected.length) { estimating = false; error = 'Choose at least one area.'; return; }
    const opts: MosaicExportOptions = { area_ids: selected, whole_mount: mount, values: mode, display_tone: tone, format: fmt, png_bits: depth, quality: q, size_choice: fmt === 'tiff' ? 'full' : choice };
    // Serial worker requests keep plans separate from an active export.
    void c.planMosaicExport!(p, opts).then(result => { if (token === revision) { plan = result; options = opts; estimating = false; } }).catch(e => { if (token === revision) { error = e instanceof Error ? e.message : String(e); estimating = false; } });
  });
  async function save() {
    if (!core || !project || !plan || !options || exporting || tooLarge || overLimit) return;
    exporting = true; error = '';
    try { if (await saveMosaicExport(core, project, options, plan)) oncancel(); else if (app.error) error = app.error; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { exporting = false; cancelling = false; }
  }
  function cancel() { if (exporting) { cancelling = true; void app.cancel(); } else oncancel(); }
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); cancel(); }
    if (e.key === 'Tab') {
      const fields = [...dialog.querySelectorAll<HTMLElement>('input:not(:disabled), select:not(:disabled), button:not(:disabled)')];
      const i = fields.indexOf(document.activeElement as HTMLElement); e.preventDefault(); fields[(i + (e.shiftKey ? -1 : 1) + fields.length) % fields.length]?.focus();
    }
  }
</script>
<div class="backdrop" role="presentation">
  <div bind:this={dialog} class="glass panel dialog" role="dialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-labelledby="mosaic-export-title" data-testid="mosaic-export-dialog">
    <h2 id="mosaic-export-title">Export mosaic</h2>
    <fieldset disabled={exporting}>
      <label class="field">Areas <select bind:value={scope}><option value="current" disabled={!areaId}>Current area</option><option value="chosen">Chosen areas, one file each</option><option value="mount">Whole mount</option></select></label>
      {#if scope === 'chosen'}
        <div class="areas">{#each project?.areas ?? [] as area}<label class="check"><input type="checkbox" value={area.id} bind:group={chosen} />{area.name}</label>{/each}</div>
      {/if}
      <label class="field">Format <select bind:value={format}><option value="tiff">TIFF</option><option value="png">PNG</option><option value="jpg">JPG</option></select></label>
      {#if format === 'png'}<label class="field">Bit depth <select bind:value={pngBits}><option value={16}>16-bit, original values</option><option value={8}>8-bit</option></select></label>{/if}
      {#if format === 'jpg'}<label class="field">Quality <input type="number" min="1" max="100" step="1" bind:value={quality} /></label><p class="faint small">JPG uses As displayed, 8-bit.</p>{:else if format !== 'png' || pngBits === 8}
      <label class="field">Pixel values <select bind:value={values}><option value="original">Original values</option><option value="displayed">As displayed, 8-bit</option></select></label>{/if}
      {#if format !== 'tiff'}<label class="field">Image size <select bind:value={sizeChoice}><option value="full">Full resolution</option><option value="downsample">Downsample by 2×, 4×, … to fit</option><option value="split">Split files to fit</option></select></label>{/if}
    </fieldset>
    <p class="faint small">Original values include stitching corrections without a tone curve. 8-bit original PNG scales [0, 1] to [0, 255]. Published images retain their available 8-bit values.</p>
    {#if estimating}<p class="faint small" role="status">Estimating mosaic size...</p>{/if}
    {#if plan}
      <p>{plan.images.length} {format.toUpperCase()} {plan.images.length === 1 ? 'file' : 'files'} · {sizeLabel(mosaicBytes(plan))} estimated · {format === 'tiff' ? (plan.images.some(a => a.big_tiff) ? 'BigTIFF' : 'Classic TIFF') : 'streamed rows'} · {plan.images.some(a => a.bit_depth === 16) ? '16-bit' : '8-bit'}</p>
      <p class="faint small">Grayscale. {format === 'tiff' ? 'Uncompressed, 512 × 512 tiles.' : ''} JSON and world files use µm.</p>
      {#if overLimit}<p role="alert">{plan.images.find(a => a.limit_reason)?.limit_reason}</p>{/if}
      {#if plan.images.some(a => (a.downsample ?? 1) > 1)}<p class="faint small">Downsampled {Math.max(...plan.images.map(a => a.downsample ?? 1))}×. Pixel sizes and coordinates are updated.</p>{/if}
      {#if plan.whole_mount_reason}<p class="faint small">{plan.whole_mount_reason}</p>{/if}
      {#if plan.images.some(a => a.coordinate_reason)}<p class="faint small">Some stage transforms are missing or inconsistent; those files use area-local coordinates.</p>{/if}
    {/if}
    {#if !streaming}<p class="faint small">This browser downloads a ZIP under 512 MiB. Use the desktop app or Chromium for larger exports.</p>{/if}
    {#if tooLarge}<p role="alert">The mosaic exceeds this browser's 512 MiB download limit.</p>{/if}
    {#if exporting}<progress max="1" value={progress?.fraction ?? 0} aria-label="Mosaic export progress"></progress><p class="faint small" role="status">{cancelling ? 'Cancelling export...' : progress?.stage ?? 'Preparing export...'}</p>{/if}
    {#if error}<p role="alert">{error}</p>{/if}
    <div class="row"><button onclick={cancel} disabled={cancelling}>Cancel</button><button class="primary" disabled={!plan || estimating || exporting || tooLarge || overLimit} onclick={save}>Export</button></div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(32rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  .areas { max-height: 10rem; overflow: auto; margin-block: var(--space-2); }
  .row { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-3); }
  progress { width: 100%; }
</style>
