<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { heldKind, nameList } from '../lib/sync';
  import { elementArea, showElement } from '../lib/held-items';
  import { openFromServer } from '../lib/remote';
  import { sameId } from '../lib/deeplink';
  import SpotLinkRepair from './SpotLinkRepair.svelte';

  const session = $derived(app.sync);
  const items = $derived(session.held);
  const projectCollisions = $derived(items.some(item => item.code === 'label_collision' || item.code === 'foreign_project'));
  const published = $derived(session.status?.atlas_version ? session.status.current_project_uuid : null);
  const server = $derived(app.remote?.server ?? session.draft?.server ?? app.server);
  let index = $state(0);
  const at = $derived(Math.max(0, Math.min(index, items.length - 1)));
  const item = $derived(items[at]);
  const kind = $derived(item ? heldKind(item.kind) : null);
  const keepReason = $derived(item ? session.keepHeldReason(item) : '');
  const here = $derived(!!item && !!kind && !!elementArea(app, kind, item.id));
  const repairs = $derived(item ? session.linkRepairs.filter(i =>
    i.spot.id === item.id || i.spot.grain_id === item.id || i.original?.grain_id === item.id
    || item.related?.some(r => r.id === i.spot.id)) : []);
  const name = (label: string | undefined, id: string) => label || id.slice(0, 8);
</script>

{#if item}
  <div class="held" data-testid="held-items">
    <h3>{items.length} {items.length === 1 ? 'item' : 'items'} held back</h3>
    <p><b data-testid="held-label">{item.kind} {name(item.label, item.id)}</b>{#if item.op === 'delete'} · <span class="faint small">deleted here, kept by the database</span>{/if}</p>
    <p data-testid="held-reason">{item.reason || item.code || 'The database rejected this operation. Review the item and retry Sync.'}</p>
    {#if item.related?.length}<p class="faint small">Related: {nameList(item.related.map(r => `${r.kind} ${name(r.label, r.id)}`), 10)}</p>{/if}
    <div class="row">
      <button disabled={at === 0} onclick={() => index = at - 1} data-testid="held-previous">Previous</button>
      <span class="faint small" data-testid="held-position">{at + 1} of {items.length}</span>
      <button disabled={at >= items.length - 1} onclick={() => index = at + 1} data-testid="held-next">Next</button>
      <button disabled={!here} title={here ? 'Select it on the canvas' : 'Not in this copy'} onclick={() => kind && showElement(app, kind, item.id)} data-testid="held-show">Show</button>
      <button disabled={!!keepReason} title={keepReason} onclick={() => session.keepHeld(item)} data-testid="held-keep">Keep database version</button>
    </div>
    <p class="faint small">Keep database version cancels this operation and restores required links. Other local edits stay here. Sync verifies the result; Undo restores the local operation.</p>
    {#if keepReason}<p class="faint small">{keepReason}</p>{/if}
    {#each item.related ?? [] as related}
      {@const relatedKind = heldKind(related.kind)}
      {#if relatedKind && elementArea(app, relatedKind, related.id)}
        <button onclick={() => relatedKind && showElement(app, relatedKind, related.id)}>Show {related.kind} {name(related.label, related.id)}</button>
      {/if}
    {/each}
    {#each repairs.slice(0, 10) as issue (issue.spot.id)}<SpotLinkRepair {issue} />{/each}
    {#if repairs.length > 10}<p class="faint small">Review the remaining spots in Spot links to review below.</p>{/if}
    {#if projectCollisions}
      <p>Some of these changes conflict with items registered by another project on this mount. Open that project to review the registered grains and spots. Keep this draft to compare new outlines or placements.</p>
      {#if published && !sameId(published, app.project?.uuid) && server && session.status}
        {@const mount = session.status.mount_id}
        <button disabled={session.busy} onclick={() => openFromServer(server, published, null, null, mount)}>Open current atlas</button>
      {/if}
      <p class="faint small">If the project with the registered items has no published atlas, open its saved project file.</p>
    {:else}
      <p class="faint small">Fix the held items here, then sync again. Items that depend on a held item are held too.</p>
      {#if !here}<p class="faint small">This item is absent here. Keep its database version to restore it, or use Show on a related item to review its area. If it belongs to another project, open that project or its saved file.</p>{/if}
    {/if}
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
