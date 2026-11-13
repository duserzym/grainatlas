<script lang="ts">
  import { app } from '../lib/store.svelte';
  const session = $derived(app.sync);
</script>

{#if session.candidates.length && session.state === 'unregistered'}
  <div class="link-offer" data-testid="sync-link-offer">
    {#each session.candidates as candidate (`${candidate.mount_id}:${candidate.project_uuid}`)}
      <div><span>Link to mount {candidate.mount_label}?</span> <button disabled={session.busy || !candidate.can_edit || app.readOnly} onclick={() => session.link(candidate)}>Link</button></div>
    {/each}
    <button onclick={() => session.dismissLink()}>Not this one</button>
  </div>
{/if}

<style>
  .link-offer { display:flex; flex-wrap:wrap; align-items:center; gap:var(--space-2); margin-top:var(--space-2); }
</style>
