<script lang="ts">
  // Light / dark / system, in the top bar and in Settings. The UPb Studio shell's
  // theme wins while it sets one on this page (hosted).
  import { app, type ThemePref } from '../lib/store.svelte';

  let { labels = false }: { labels?: boolean } = $props();
  const options: { id: ThemePref; label: string; title: string }[] = [
    { id: 'light', label: 'Light', title: 'Light theme' },
    { id: 'dark', label: 'Dark', title: 'Dark theme' },
    { id: 'system', label: 'System', title: 'Follow the system setting' },
  ];
  const pressed = (id: ThemePref) => (app.themeChosen || !app.hosted ? app.themePref === id : id === 'system');
</script>

<div class="seg" role="group" aria-label="Theme" data-testid="theme-switch">
  {#each options as o (o.id)}
    <button
      class:labels
      class="icon"
      aria-pressed={!app.hostTheme && pressed(o.id)}
      disabled={!!app.hostTheme}
      title={app.hostTheme ? `UPb Studio sets the theme (${app.hostTheme})` : o.title}
      aria-label={o.title}
      onclick={() => app.setTheme(o.id)}
      data-testid="theme-{o.id}"
    >
      {#if o.id === 'light'}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><circle cx="8" cy="8" r="3" fill="currentColor" /><g stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" /></g></svg>
      {:else if o.id === 'dark'}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M13.5 10.2A5.8 5.8 0 0 1 5.8 2.5a5.8 5.8 0 1 0 7.7 7.7z" fill="currentColor" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="2" y="3" width="12" height="8.5" rx="1.3" fill="none" stroke="currentColor" stroke-width="1.4" /><path d="M5.5 14h5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /><path d="M8 4.5v5.5a2.75 2.75 0 0 1 0-5.5z" fill="currentColor" /></svg>
      {/if}
      {#if labels}<span>{o.label}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-sm);
  }
  .seg button {
    border-color: transparent;
    background: transparent;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    min-width: 0;
    padding: 0.25rem 0.45rem;
  }
  .seg button.labels {
    padding: 0.3rem 0.7rem;
  }
</style>
