<script lang="ts">
  import { releaseNotesUrl, type UpdateStatus } from '../lib/updates';
  import { openReleaseNotes } from '../lib/updater.svelte';
  let { status }: { status: UpdateStatus } = $props();
  const info = $derived(status.kind === 'available' || status.kind === 'ready' ? status.info : null);
  let error = $state('');
  async function open(event: MouseEvent) {
    event.preventDefault();
    if (!info) return;
    error = '';
    try { await openReleaseNotes(info.version); } catch (reason) { error = String(reason); }
  }
</script>

{#if info}
  {#if info.notes}<p class="notes small" data-testid="update-notes">{info.notes}</p>{/if}
  <a class="small" href={releaseNotesUrl(info.version)} target="_blank" rel="noreferrer" onclick={open}>Release notes</a>
  {#if error}<p class="faint small" role="alert">{error}</p>{/if}
{/if}

<style>
  .small { font-size: 0.8rem; }
  p { margin: 0; overflow-wrap: anywhere; }
  .notes { white-space: pre-wrap; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 4; line-clamp: 4; overflow: hidden; line-height: 1.45; max-height: 5.8em; }
</style>
