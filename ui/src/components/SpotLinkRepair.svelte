<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { oversightSession } from '../lib/oversight';
  import { showElement } from '../lib/held-items';
  import type { LinkIssue } from '../lib/link-repair';
  let { issue }: { issue: LinkIssue } = $props();
  let candidate = $state('');
  const selected = $derived(issue.candidates.find(c => c.grain.id === candidate));
  const disabled = $derived(app.readOnly || oversightSession.readOnly || app.sync.busy || !!app.sync.review || !!app.sync.confirmation || !!app.sync.staleOpen);
  const restoreReason = $derived(issue.original ? app.sync.keepHeldReason({ kind: 'spot', id: issue.spot.id }) : '');
</script>

<div class="panel" data-testid="spot-link-repair">
  <p><b>{issue.spot.name}</b> · {issue.spot.area_id}</p>
  <p>{issue.reason}</p>
  {#if !issue.candidates.length}<p class="faint small">No eligible outline to assign. Show spot to inspect its area, then restore the original or detach deliberately. You can also draw or correct an outline and return to Review issues.</p>{/if}
  <div class="row">
    <button onclick={() => showElement(app, 'spots', issue.spot.id)}>Show spot</button>
    {#if issue.candidates.length}
      <label>Grain
        <select class="field" value={selected ? candidate : ''} onchange={e => candidate = e.currentTarget.value} disabled={disabled} aria-label={`Grain for ${issue.spot.name}`}>
          <option value="">Choose an outline</option>
          {#each issue.candidates as c}
            <option value={c.grain.id}>{c.grain.label || `${c.grain.area_id}/${c.grain.number}`}{c.boundary ? ' (boundary)' : ''}</option>
          {/each}
        </select>
      </label>
      <button disabled={!selected} onclick={() => selected && showElement(app, 'grains', selected.grain.id)}>Show grain</button>
      <button disabled={disabled || !selected} onclick={() => app.sync.repairSpot(issue.spot.id, candidate)}>Assign same spot</button>
    {/if}
    {#if issue.original}
      <button disabled={!app.sync.canKeepHeld({ kind: 'spot', id: issue.spot.id })} onclick={() => app.sync.repairSpot(issue.spot.id, 'keep-original')}>Restore original spot and grain</button>
    {/if}
    <button disabled={disabled} onclick={() => app.sync.repairSpot(issue.spot.id, 'detach')}>Detach grain, keep spot</button>
  </div>
  {#if issue.candidates.length && !selected}<p class="faint small">Choose an outline to enable Show grain and Assign same spot.</p>{/if}
  {#if disabled}<p class="faint small">{oversightSession.readOnly ? 'Viewing as someone else is read-only. Return to an editable session to change this link.' : app.readOnly ? 'Open an editable copy to change this link.' : app.sync.busy ? 'Wait for Sync to finish, then choose a recovery action.' : 'Use the header Sync action to finish the pending revision or removal review first.'}</p>{/if}
  {#if restoreReason}<p class="faint small">Restore original: {restoreReason}</p>{/if}
  <p class="faint small">Assignment keeps this spot's identity, position and measurements. Detach removes only the grain link. Sync saves the choice; Undo restores it.</p>
</div>

<style>
  p { margin:var(--space-1) 0; overflow-wrap:anywhere; }
  .row { display:flex; gap:var(--space-2); flex-wrap:wrap; align-items:center; }
  select { max-width:18rem; }
</style>
