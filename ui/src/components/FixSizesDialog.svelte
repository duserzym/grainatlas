<script lang="ts">
  import { onMount } from 'svelte';
  import type { Spot } from '../lib/core/types';
  import type { SizeRepair } from '../lib/spot-size';
  let { rows, spots, areas, onapply, oncancel }: {
    rows: SizeRepair[]; spots: Spot[]; areas: { id: string; name: string }[];
    onapply: () => void; oncancel: () => void;
  } = $props();
  const names = $derived(new Map(spots.map(s => [s.id, s.name])));
  const areaNames = $derived(new Map(areas.map(a => [a.id, a.name])));
  let dialog: HTMLDivElement;
  let cancel: HTMLButtonElement;
  const currentText = (v: unknown) => v === undefined ? 'missing' : v === null ? 'null' : String(v);
  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    cancel.focus();
    return () => previous?.focus();
  });
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); oncancel(); }
    if (e.key === 'Tab') {
      const fields = [...dialog.querySelectorAll<HTMLButtonElement>('button')];
      const i = fields.indexOf(document.activeElement as HTMLButtonElement);
      e.preventDefault(); fields[(i + (e.shiftKey ? -1 : 1) + fields.length) % fields.length]?.focus();
    }
  }
</script>
<div class="backdrop" role="presentation">
  <div bind:this={dialog} class="glass panel dialog" role="dialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-labelledby="fix-size-title" data-testid="fix-sizes-dialog">
    <h2 id="fix-size-title">Fix sizes</h2>
    <p class="faint small">Only spots with no valid size. Proposals use the most common size in the area, then the project, then Settings. Equal counts use the smaller size.</p>
    <div class="preview"><table class="data"><thead><tr><th>Area</th><th>Spot</th><th>Current</th><th>Proposed µm</th></tr></thead><tbody>
      {#each rows as row (row.id)}<tr><td>{areaNames.get(row.area_id) ?? row.area_id}</td><td>{names.get(row.id) ?? row.id}</td><td>{currentText(row.current)}</td><td>{row.proposed}</td></tr>{/each}
    </tbody></table></div>
    <div class="row"><button bind:this={cancel} onclick={oncancel}>Cancel</button><button class="primary" onclick={onapply} data-testid="fix-sizes-apply">Apply</button></div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(38rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  .preview { max-height: 22rem; overflow: auto; margin-block: var(--space-3); }
  table { width: 100%; table-layout: fixed; }
  td { overflow-wrap: anywhere; }
  .row { display: flex; justify-content: flex-end; gap: var(--space-2); }
</style>
