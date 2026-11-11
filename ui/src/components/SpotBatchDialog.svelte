<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { Grain, Spot } from '../lib/core/types';
  import { planBatch, type BatchOrder } from '../lib/batch';
  let { spots, project, grains, tableIds, pattern: initialPattern, numbering, onapply, oncancel }: {
    spots: Spot[]; project: Spot[]; grains: Grain[]; tableIds: string[]; pattern: string; numbering: boolean;
    onapply: (spots: Spot[]) => void; oncancel: () => void;
  } = $props();
  // These are the defaults at opening, independent of subsequent field edits.
  let sample = $state(untrack(() => spots.every((s) => s.sample === spots[0].sample) ? spots[0].sample : ''));
  let pattern = $state(untrack(() => initialPattern));
  let renumber = $state(untrack(() => numbering));
  let start = $state(untrack(() => Math.min(...spots.map((s) => s.n))));
  let order = $state<BatchOrder>('table');
  let dialog: HTMLDivElement;
  let input: HTMLInputElement;
  const plan = $derived(planBatch(project, spots, { sample, pattern, renumber, start, order }, tableIds, grains));
  const invalid = $derived(renumber && (!Number.isInteger(start) || start < 1));
  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    input.focus();
    return () => previous?.focus();
  });
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); oncancel(); }
    if (e.key === 'Tab') {
      const fields = [...dialog.querySelectorAll<HTMLElement>('input:not(:disabled), select:not(:disabled), button:not(:disabled)')];
      const i = fields.indexOf(document.activeElement as HTMLElement);
      e.preventDefault();
      fields[(i + (e.shiftKey ? -1 : 1) + fields.length) % fields.length]?.focus();
    }
  }
</script>
<div class="backdrop" role="presentation">
  <div bind:this={dialog} class="glass panel dialog" role="dialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-labelledby="batch-title" data-testid="batch-dialog">
    <h2 id="batch-title">Rename {spots.length} spots</h2>
    <label class="field">Sample <input bind:this={input} type="text" bind:value={sample} placeholder="keep each spot's sample" /></label>
    <label class="field">Pattern <input type="text" bind:value={pattern} /></label>
    <div class="row">
      <label class="check"><input type="checkbox" bind:checked={renumber} /> Renumber</label>
      <label class="field">from <input type="number" min="1" step="1" bind:value={start} disabled={!renumber} data-testid="batch-start" /></label>
      <label class="field">Order <select bind:value={order} data-testid="batch-order"><option value="table">table order</option><option value="placement">placement order</option><option value="rows">along the rows, left to right</option></select></label>
    </div>
    {#if plan.clashes}<p class="clash" data-testid="batch-clash">{plan.clashes} {plan.clashes === 1 ? 'name' : 'names'} would be used twice.</p>{/if}
    <div class="preview"><table class="data"><thead><tr><th>Old</th><th>→ New</th></tr></thead><tbody>
      {#each plan.rows as row (row.old.id)}<tr class:clash={row.clash} data-testid="batch-preview"><td>{row.old.name}</td><td>→ {row.next.name}</td></tr>{/each}
    </tbody></table></div>
    <div class="row actions"><button onclick={oncancel}>Cancel</button><button class="primary" disabled={!!plan.clashes || invalid} onclick={() => onapply(plan.rows.map((r) => r.next))} data-testid="batch-apply">Apply</button></div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(38rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .row > * { min-width: 0; }
  input[type='number'] { width: 7rem; }
  .preview { max-height: 16rem; overflow: auto; margin-block: var(--space-3); }
  td { overflow-wrap: anywhere; }
  table { width: 100%; table-layout: fixed; }
  .clash { color: var(--warning); }
  .actions { justify-content: flex-end; }
</style>
