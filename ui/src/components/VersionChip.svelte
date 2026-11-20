<script lang="ts">
  import { tick } from 'svelte';
  import { isHosted } from '@lab/bridge';
  import { app } from '../lib/store.svelte';
  import { updater, updatePreview } from '../lib/updater.svelte';
  import { checkButtonView } from '../lib/updates';

  const view = $derived(checkButtonView(updater.status, updater.version, updater.prefs.autoDownload));
  async function openUpdates() {
    app.screen = 'settings';
    await tick();
    document.getElementById('updates')?.scrollIntoView({ block: 'center' });
  }
</script>

{#if !isHosted() || updatePreview}
  <button class="glass" data-ui data-testid="version-chip" onclick={openUpdates} title={`Settings > Updates · ${view.text}`}>
    <span class="version">v{updater.version}</span>
  </button>
{/if}

<style>
  button.glass {
    display: inline-flex; align-items: center; flex: none; align-self: center;
    border-radius: 999px; font-size: 0.75rem;
    background: var(--glass-strong); border-color: var(--glass-border);
    padding: 0.4rem 0.65rem; color: var(--text); line-height: 1.2;
  }
  .version { color: var(--text-muted); white-space: nowrap; }
</style>
