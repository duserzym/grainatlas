<script lang="ts">
  import { tick } from 'svelte';
  import { isHosted } from '@lab/bridge';
  import { app } from '../lib/store.svelte';
  import { updater, updatePreview } from '../lib/updater.svelte';

  const status = $derived(updater.status);
  let justChecked = $state(false);
  $effect(() => {
    const remaining = updater.manualNoneUntil - Date.now();
    justChecked = remaining > 0;
    const timer = setTimeout(() => justChecked = false, Math.max(0, remaining));
    return () => clearTimeout(timer);
  });
  const label = $derived(status.kind === 'checking' ? 'Checking'
    : status.kind === 'downloading' ? `Downloading${status.pct === null ? '' : ` ${status.pct}%`}`
    : status.kind === 'available' || status.kind === 'unsupported' && status.info ? 'Update available'
    : status.kind === 'ready' ? 'Update ready, restart'
    : status.kind === 'installing' ? 'Installing and restarting'
    : status.kind === 'error' ? 'Check failed'
    : status.kind === 'none' && justChecked ? 'Up to date' : '');
  async function openUpdates() {
    app.screen = 'settings';
    await tick();
    document.getElementById('updates')?.scrollIntoView({ block: 'center' });
  }
</script>

{#if !isHosted() || updatePreview}
  <button class="glass" data-ui data-testid="version-chip" onclick={openUpdates} title="Settings > Updates">
    <span class="version">v{updater.version}</span>
    {#if status.kind === 'checking' || status.kind === 'downloading'}<span class="spinner" aria-hidden="true"></span>
    {:else if status.kind === 'available' || status.kind === 'ready' || status.kind === 'unsupported' && status.info}<span class="dot" aria-hidden="true"></span>{/if}
    {#if label}<span>{label}</span>{/if}
  </button>
{/if}

<style>
  button.glass {
    position: fixed; right: 12px; bottom: 12px; z-index: 60;
    display: flex; align-items: center; gap: var(--space-2);
    max-width: 220px; border-radius: 999px; font-size: 0.75rem;
    background: var(--glass-strong); border-color: var(--glass-border);
    padding: 0.4rem 0.65rem; color: var(--text); line-height: 1.2;
  }
  .version { color: var(--text-muted); white-space: nowrap; }
  .dot, .spinner { flex: 0 0 auto; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); }
  .spinner { width: 10px; height: 10px; background: none; border: 1px solid var(--glass-border); border-top-color: var(--accent); animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
</style>
