<script lang="ts">
  import UpdateNotes from './UpdateNotes.svelte';
  import { isHosted } from '@lab/bridge';
  import { isDesktop } from '../lib/core/core';
  import { check, download, install, setPrefs, updater, updatePreview } from '../lib/updater.svelte';
  import { checkButtonView, RELEASES_PAGE } from '../lib/updates';

  const status = $derived(updater.status);
  const time = (at: number) => new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const view = $derived(checkButtonView(status, updater.version, updater.prefs.autoDownload));
</script>

<h2>Updates</h2>
<strong>grainatlas v{updater.version}</strong>
{#if isHosted()}
  <p class="faint small">UPb Lab updates grainatlas with the shell; see its Settings &gt; Updates.</p>
{:else}
  <p class="small" role="status">{view.text}</p>
  {#if view.secondary}<p class="faint small">{view.secondary}</p>{/if}
  <UpdateNotes {status} />
  {#if view.details !== null}<p class="faint small" style:user-select="text">Details: {view.details}</p>{/if}
  <div class="buttons">
    {#if isDesktop() || updatePreview}
      <button disabled={status.kind === 'checking' || status.kind === 'downloading' || status.kind === 'installing'} onclick={() => check(true)}>Check now</button>
    {/if}
    {#if status.kind === 'available' && !updater.prefs.autoDownload}
      <button onclick={download}>Download</button>
    {/if}
    {#if status.kind === 'ready'}<button class="primary" onclick={install}>Restart to update</button>{/if}
    {#if status.kind === 'unsupported'}
      <a class="btn" href={RELEASES_PAGE} target="_blank" rel="noreferrer">Open download page</a>
    {/if}
  </div>
  <label class="check"><input type="checkbox" checked={updater.prefs.autoDownload} onchange={(e) => setPrefs({ autoDownload: e.currentTarget.checked })} /> Download updates automatically</label>
  <p class="faint small">Last check: {updater.prefs.lastCheck ? time(updater.prefs.lastCheck) : 'Not checked yet'}{updater.prefs.lastResult ? ` · ${updater.prefs.lastResult}` : ''}</p>
{/if}

<style>
  .small { font-size: 0.8rem; }
  p { margin: 0; overflow-wrap: anywhere; }
  .buttons { display: flex; flex-wrap: wrap; gap: var(--space-2); }
</style>
