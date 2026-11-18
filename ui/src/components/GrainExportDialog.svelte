<script lang="ts">
  import { onMount } from 'svelte';
  import type { ExportChoice, ExportScope } from '../lib/grain-export';
  let { selectedCount, hasArea, onexport, oncancel }: { selectedCount: number; hasArea: boolean; onexport: (choice: ExportChoice) => void; oncancel: () => void } = $props();
  let scope = $state<ExportScope>('area');
  let pixel_values = $state<'original' | 'displayed'>('original');
  let format = $state<'png' | 'tiff'>('png');
  let margin_um = $state(10);
  let include_crops = $state(true);
  let include_masks = $state(true);
  let dialog: HTMLDivElement;
  onMount(() => { const previous = document.activeElement as HTMLElement | null; if (!hasArea) scope = 'mount'; dialog.querySelector<HTMLElement>('select')?.focus(); return () => previous?.focus(); });
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); oncancel(); }
    if (e.key === 'Tab') {
      const fields = [...dialog.querySelectorAll<HTMLElement>('input:not(:disabled), select:not(:disabled), button:not(:disabled)')];
      const i = fields.indexOf(document.activeElement as HTMLElement);
      e.preventDefault(); fields[(i + (e.shiftKey ? -1 : 1) + fields.length) % fields.length]?.focus();
    }
  }
</script>
<div class="backdrop" role="presentation">
  <div bind:this={dialog} class="glass panel dialog" role="dialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-labelledby="grain-export-title" data-testid="grain-export-dialog">
    <h2 id="grain-export-title">Export grains</h2>
    <label class="field">Scope <select bind:value={scope}><option value="selected" disabled={!selectedCount}>Selected grains ({selectedCount})</option><option value="area" disabled={!hasArea}>This area</option><option value="mount">Whole mount</option></select></label>
    <label class="field">Pixel values <select bind:value={pixel_values}><option value="original">Original, 16-bit</option><option value="displayed">As displayed, 8-bit</option></select></label>
    <p class="faint small">Original values include stitching and photometric corrections. Published areas have only 8-bit pixels.</p>
    <label class="field">Image format <select bind:value={format}><option value="png">PNG</option><option value="tiff">TIFF</option></select></label>
    <label class="field">Margin (µm) <input type="number" min="0" step="1" bind:value={margin_um} /></label>
    <label class="check"><input type="checkbox" bind:checked={include_crops} /> Include crops</label>
    <label class="check"><input type="checkbox" bind:checked={include_masks} /> Include masks</label>
    <p class="faint small">The table, outlines and provenance are always included. Reference values use all non-artifact grains, even for a selection.</p>
    <div class="row"><button onclick={oncancel}>Cancel</button><button class="primary" disabled={!Number.isFinite(margin_um) || margin_um < 0 || (scope === 'selected' && !selectedCount)} onclick={() => onexport({ scope, pixel_values, format, margin_um, include_crops, include_masks })}>Export</button></div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(30rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  .row { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-3); }
</style>
