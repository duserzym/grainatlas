<script lang="ts">
  // Loose images have no calibration: ask once for the scale of the whole set
  // (µm per pixel, or the width of the field of view), defaulting to the last
  // scale typed in this session, and whether the images are separate areas
  // (the default) or the tiles of one grid.
  import { app } from '../lib/store.svelte';
  import { openLoose, rememberedScale } from '../lib/actions';
  import type { GridOrder, LooseSpec } from '../lib/core/types';

  const pending = $derived(app.pendingLoose!);
  const last = rememberedScale();
  const prior = app.pendingLoose?.spec ?? null;
  const n = app.pendingLoose?.images.length ?? 0;

  let mode = $state<'um_per_px' | 'field_width'>(prior ? (prior.field_width_um ? 'field_width' : 'um_per_px') : (last?.mode ?? 'um_per_px'));
  let umPerPx = $state<number>(prior?.um_per_px || last?.um_per_px || 1);
  let fieldUm = $state<number>(prior?.field_width_um || last?.field_width_um || 1000);
  let asGrid = $state(!!prior?.grid);
  let cols = $state(prior?.grid?.cols ?? n);
  let rows = $state(prior?.grid?.rows ?? 1);
  let overlap = $state(prior?.grid?.overlap_pct ?? 15);
  let order = $state<GridOrder>(prior?.grid?.order ?? 'rows');
  let gridName = $state(prior?.grid?.name ?? app.pendingLoose?.name ?? '');

  const widths = $derived([...new Set(pending.images.map((i) => i.w))]);
  const derivedUm = $derived(mode === 'field_width' && widths[0] ? fieldUm / widths[0] : umPerPx);
  const valid = $derived((mode === 'um_per_px' ? umPerPx > 0 : fieldUm > 0) && (!asGrid || (cols >= 1 && rows >= 1 && cols * rows >= n && overlap >= 0 && overlap < 90)));

  function submit(e: Event) {
    e.preventDefault();
    if (!valid) return;
    const spec: LooseSpec = {
      um_per_px: mode === 'um_per_px' ? umPerPx : derivedUm,
      files: pending.images.map((i) => i.file),
    };
    if (mode === 'field_width') spec.field_width_um = fieldUm;
    if (asGrid) spec.grid = { cols, rows, overlap_pct: overlap, order, name: gridName.trim() };
    void openLoose(spec);
  }
  const fmt = (v: number) => (Number.isFinite(v) ? (v >= 10 ? v.toFixed(1) : v.toPrecision(4)) : '–');
</script>

<div class="backdrop" role="presentation">
  <form class="glass panel dialog" onsubmit={submit} aria-labelledby="scale-title" data-testid="scale-dialog">
    <h2 id="scale-title">Scale for {n} image{n === 1 ? '' : 's'}</h2>
    <p class="muted">These images carry no pixel size. Give one scale for the whole set; it is stored in the project and asked only once.</p>
    <ul class="files mono">
      {#each pending.images.slice(0, 6) as im (im.file)}<li>{im.file} <span class="faint">{im.w}×{im.h}</span></li>{/each}
      {#if n > 6}<li class="faint">… and {n - 6} more</li>{/if}
    </ul>

    <fieldset>
      <legend class="muted">Scale</legend>
      <label class="check"><input type="radio" bind:group={mode} value="um_per_px" /> µm per pixel</label>
      {#if mode === 'um_per_px'}
        <input type="number" min="0" step="any" bind:value={umPerPx} aria-label="Micrometres per pixel" data-testid="um-per-px" />
      {/if}
      <label class="check"><input type="radio" bind:group={mode} value="field_width" /> field width (µm across the image)</label>
      {#if mode === 'field_width'}
        <input type="number" min="0" step="any" bind:value={fieldUm} aria-label="Field width in micrometres" data-testid="field-width" />
        <span class="faint small">= {fmt(derivedUm)} µm per pixel{widths.length > 1 ? ' for the widest; each image is scaled by its own width' : ` at ${widths[0]} px`}</span>
      {/if}
    </fieldset>

    <fieldset>
      <legend class="muted">Arrangement</legend>
      <label class="check"><input type="radio" bind:group={asGrid} value={false} /> each image is its own area</label>
      <label class="check"><input type="radio" bind:group={asGrid} value={true} data-testid="as-grid" /> the images are tiles of one grid</label>
      {#if asGrid}
        <div class="grid4">
          <label class="field">Columns <input type="number" min="1" bind:value={cols} data-testid="grid-cols" /></label>
          <label class="field">Rows <input type="number" min="1" bind:value={rows} /></label>
          <label class="field">Overlap % <input type="number" min="0" max="89" step="any" bind:value={overlap} data-testid="grid-overlap" /></label>
          <label class="field">Order
            <select bind:value={order}>
              <option value="rows">rows, left to right</option>
              <option value="snake">serpentine rows</option>
              <option value="columns">columns, top to bottom</option>
            </select>
          </label>
        </div>
        <label class="field">Area name <input type="text" bind:value={gridName} /></label>
        <span class="faint small">Tiles are taken in file-name order; the overlap is a starting guess the stitcher refines.{cols * rows < n ? ` ${n} images need more than ${cols} × ${rows} cells.` : ''}</span>
      {/if}
    </fieldset>

    <div class="row">
      <button type="button" onclick={() => (app.pendingLoose = null)}>Cancel</button>
      <button type="submit" class="primary" disabled={!valid} data-testid="scale-open">Open</button>
    </div>
  </form>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg) 55%, transparent);
  }
  .dialog {
    width: min(34rem, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    overflow: auto;
    display: grid;
    gap: var(--space-3);
    background: var(--glass-strong);
  }
  .dialog p {
    margin: 0;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 0.8rem;
    display: grid;
    gap: 0.1rem;
  }
  fieldset {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    display: grid;
    gap: var(--space-2);
  }
  .grid4 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr 2fr;
    gap: var(--space-2);
    align-items: end;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
  .small {
    font-size: 0.8rem;
  }
</style>
