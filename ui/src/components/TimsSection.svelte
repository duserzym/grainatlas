<script lang="ts">
  import StatusChip from '../lib/ui/StatusChip.svelte';
  import { untrack } from 'svelte';
  import { app } from '../lib/store.svelte';
  import type { FractionOut } from '../lib/tims';
  import type { OutboxEntry } from '../lib/tims-outbox';
  let { grain, onedit }: { grain: string; onedit: (fraction: FractionOut, failed?: OutboxEntry) => void } = $props();
  const store = $derived(app.tims.store);
  const badge = $derived(app.tims.badges.get(grain));
  const entries = $derived.by(() => { app.tims.version; return store?.data.outbox.filter(e => e.state === 'failed' && (badge?.fractions.some(f => f.id === e.target || f.batch_id === e.target) || (e.body.sources as { atlas_grain_id: string }[] | undefined)?.some(s => s.atlas_grain_id === grain))) ?? []; });
  const batchIds = $derived([...new Set(badge?.fractions.flatMap(f => f.batch_id ? [f.batch_id] : []) ?? [])].join(','));
  $effect(() => {
    const ids = batchIds, current = store;
    if (ids && current) untrack(() => { for (const id of ids.split(',')) void current.loadBatch(id); });
  });
  const label = (id: string) => Object.values(app.grains).flat().find(g => g.id === id)?.label || Object.values(app.grains).flat().find(g => g.id === id)?.number || id;
  async function remove(fraction: FractionOut) {
    if (!store || app.readOnly) return;
    const source = fraction.sources.filter(s => s.atlas_grain_id === grain);
    const others = fraction.sources.some(s => s.atlas_grain_id !== grain);
    if (!confirm(`${others ? 'Remove this grain from the fraction' : 'Remove this pluck'}? It is kept in the database history.`)) return;
    try {
      if (others) for (const s of source) await store.retireSource(fraction.id, s.id);
      else await store.retireFraction(fraction.id);
    } catch (e) { app.error = e instanceof Error ? e.message : String(e); }
  }
</script>

{#if badge || entries.length}
  <section data-testid="tims-section">
    <strong>TIMS</strong>
    {#each badge?.fractions ?? [] as f (f.id)}
      {@const batch = store?.batches.find(b => b.id === f.batch_id)}
      {@const set = store?.data.sets.find(s => s.id === batch?.capsule_set_id)}
      <p>{set?.name ?? f.capsule_set_name ?? 'No capsule set'}{f.capsule_number ? ` · ${set?.capsule_labels[f.capsule_number - 1] ?? f.capsule_label ?? f.capsule_number}` : ''}{batch ? ` · ${batch.started_on}` : ''}</p>
      <p>{f.name} · {f.mineral} · <StatusChip text={f.status} /></p>
      {#each f.sources.filter(s => s.atlas_grain_id === grain && s.is_fragment) as s (s.id)}<p>Fragment: {s.fragment_note}</p>{/each}
      {#if f.notes}<p>{f.notes}</p>{/if}
      {#if f.sources.some(s => s.atlas_grain_id !== grain)}<p>Other grains: {f.sources.filter(s => s.atlas_grain_id !== grain).map(s => label(s.atlas_grain_id)).join(', ')}</p>{/if}
      <p class="faint small">{badge?.state === 'waiting' ? 'Waits for the mount to sync' : badge?.state === 'pending' ? 'Saved on this computer; sends when online' : badge?.state === 'failed' ? 'Pluck needs attention' : 'Sent to the database'}</p>
      {#if !app.readOnly}<div class="row"><button onclick={() => onedit(f)}>Edit</button><button class="danger" onclick={() => { void remove(f); }}>Remove pluck</button></div>{/if}
    {/each}
    {#each entries as entry (entry.id)}
      <p class="danger" role="alert">{entry.error}</p>
      {#if !app.readOnly}<div class="row"><button onclick={() => { const fraction = badge?.fractions.find(f => f.id === entry.target || f.batch_id === entry.target); if (fraction) onedit(fraction, entry); }}>Edit</button><button onclick={() => { if (store) void store.discard(entry.id); }}>Discard</button></div>{/if}
    {/each}
  </section>
{/if}

<style>
  section {border-top:1px solid var(--hairline);padding-top:var(--space-2);display:grid;gap:var(--space-2);overflow-wrap:anywhere;}
  p {margin:0;}
  [role=alert] {color:var(--danger);}
</style>
