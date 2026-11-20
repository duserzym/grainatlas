<script lang="ts">
  import { isHosted } from '@lab/bridge';
  import { isDesktop } from '../lib/core/core';
  import { check, download, install, setPrefs, updater, updatePreview } from '../lib/updater.svelte';
  import { checkButtonView, RELEASES_PAGE, type Channel } from '../lib/updates';

  const status = $derived(updater.status);
  const info = $derived('info' in status ? status.info : null);
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
  <label class="field">Channel
    <select aria-label="Channel" value={updater.prefs.channel} onchange={(e) => setPrefs({ channel: e.currentTarget.value as Channel })}>
      <option value="stable">Stable</option><option value="beta">Beta (pre-releases)</option>
    </select>
  </label>
  <p class="faint small">Last check: {updater.prefs.lastCheck ? time(updater.prefs.lastCheck) : 'Not checked yet'}{updater.prefs.lastResult ? ` · ${updater.prefs.lastResult}` : ''}</p>
  {#if info?.notes}<div class="notes small">{info.notes}</div>{/if}
  <a class="small" href="https://github.com/duserzym/grainatlas/releases" target="_blank" rel="noreferrer">Release notes on GitHub</a>
{/if}

<style>
  .small { font-size: 0.8rem; }
  p { margin: 0; overflow-wrap: anywhere; }
  .buttons { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .notes { white-space: pre-wrap; overflow: auto; max-height: calc(12 * 1.45em); overflow-wrap: anywhere; }
</style>
