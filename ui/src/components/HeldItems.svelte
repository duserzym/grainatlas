<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { heldKind, nameList } from '../lib/sync';
  import { elementArea, showElement } from '../lib/held-items';

  const session = $derived(app.sync);
  const items = $derived(session.held);
  let index = $state(0);
  const at = $derived(Math.max(0, Math.min(index, items.length - 1)));
  const item = $derived(items[at]);
  const kind = $derived(item ? heldKind(item.kind) : null);
  const here = $derived(!!item && !!kind && !!elementArea(app, kind, item.id));
  const name = (label: string | undefined, id: string) => label || id.slice(0, 8);
</script>

{#if item}
  <div class="held" data-testid="held-items">
    <h3>{items.length} {items.length === 1 ? 'item' : 'items'} held back</h3>
    <p><b data-testid="held-label">{item.kind} {name(item.label, item.id)}</b>{#if item.op === 'delete'} · <span class="faint small">deleted here, kept by the database</span>{/if}</p>
    {#if item.reason}<p data-testid="held-reason">{item.reason}</p>{/if}
    {#if item.related?.length}<p class="faint small">Related: {nameList(item.related.map(r => `${r.kind} ${name(r.label, r.id)}`), 10)}</p>{/if}
    <div class="row">
      <button disabled={at === 0} onclick={() => index = at - 1} data-testid="held-previous">Previous</button>
      <span class="faint small" data-testid="held-position">{at + 1} of {items.length}</span>
      <button disabled={at >= items.length - 1} onclick={() => index = at + 1} data-testid="held-next">Next</button>
      <button disabled={!here} title={here ? 'Select it on the canvas' : 'Not in this copy'} onclick={() => kind && showElement(app, kind, item.id)} data-testid="held-show">Show</button>
    </div>
    <p class="faint small">Fix the held items here, then sync again. Items that depend on a held item are held too.</p>
    <details>
      <summary class="faint small">Details</summary>
      <p class="faint small mono">{item.kind} {item.id}{item.code ? ` · ${item.code}` : ''}{#each item.related ?? [] as r} · {r.kind} {r.id}{/each}</p>
    </details>
  </div>
{/if}

<style>
  .held { border:1px solid color-mix(in srgb, var(--warning) 35%, transparent); border-radius:var(--radius-sm); padding:var(--space-2); margin:var(--space-2) 0; }
  h3 { margin:0 0 var(--space-1); }
  p { margin:var(--space-1) 0; overflow-wrap:anywhere; }
  .row { display:flex; align-items:center; gap:var(--space-2); flex-wrap:wrap; }
  .small { font-size:0.8rem; }
</style>
