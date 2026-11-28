<script lang="ts">
  import { app } from '../lib/store.svelte';
  import SpotLinkRepair from './SpotLinkRepair.svelte';
  import { showElement } from '../lib/held-items';
  const issues = $derived(app.sync.linkRepairs);
  const audits = $derived(app.sync.linkAudits);
  let auditPage = $state(0);
  const page = $derived(Math.max(0, Math.min(auditPage, Math.ceil(audits.length / 10) - 1)));
  let index = $state(0);
  const at = $derived(Math.max(0, Math.min(index, issues.length - 1)));
  const issue = $derived(issues[at]);
</script>

{#if issue}
  <h3>Spot links to review ({issues.length})</h3>
  <div class="row">
    <button disabled={at === 0} onclick={() => index = at - 1}>Previous spot</button>
    <span class="faint small">{at + 1} of {issues.length}</span>
    <button disabled={at >= issues.length - 1} onclick={() => index = at + 1}>Next spot</button>
  </div>
  {#key issue.spot.id}<SpotLinkRepair {issue} />{/key}
{/if}
{#if audits.length}
  <details data-testid="spot-link-audit">
    <summary>Reviewed spot links ({audits.length})</summary>
    <div class="row">
      <button disabled={page === 0} onclick={() => auditPage = page - 1}>Previous links</button>
      <span class="faint small">{page * 10 + 1}–{Math.min(audits.length, (page + 1) * 10)} of {audits.length}</span>
      <button disabled={(page + 1) * 10 >= audits.length} onclick={() => auditPage = page + 1}>Next links</button>
    </div>
    {#each audits.slice(page * 10, (page + 1) * 10) as spot}
      <p>{spot.name}: {spot.grain_link_review?.reason} <button onclick={() => showElement(app, 'spots', spot.id)}>Show spot</button></p>
    {/each}
  </details>
{/if}

<style>
  .row { display:flex; gap:var(--space-2); align-items:center; flex-wrap:wrap; }
  p { overflow-wrap:anywhere; }
</style>
