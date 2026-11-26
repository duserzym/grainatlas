<script lang="ts">
  import { app, type ThemePref } from '../lib/store.svelte';
  import SegmentedControl from '../lib/ui/SegmentedControl.svelte';
  let { labels = false }: { labels?: boolean } = $props();
  const options = $derived([
    { value: 'light', label: 'Light', ariaLabel: 'Light theme', title: 'Light theme' },
    { value: 'dark', label: 'Dark', ariaLabel: 'Dark theme', title: 'Dark theme' },
    { value: 'system', label: 'System', ariaLabel: 'Follow the system setting', title: 'Follow the system setting' },
  ].map(option => ({ ...option, testid: `theme-${option.value}`,
    title: app.hostTheme ? `UPb Lab sets the theme (${app.hostTheme})` : option.title })));
</script>

<SegmentedControl {options} value={app.hostTheme ? '' : app.themeChosen || !app.hosted ? app.themePref : 'system'}
  label="Theme" disabled={!!app.hostTheme} testid="theme-switch" size="small"
  onchange={value => app.setTheme(value as ThemePref)}>
  {#snippet content(option)}
    <span class="choice">
      {#if option.value === 'light'}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><circle cx="8" cy="8" r="3" fill="currentColor" /><g stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" /></g></svg>
      {:else if option.value === 'dark'}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M13.5 10.2A5.8 5.8 0 0 1 5.8 2.5a5.8 5.8 0 1 0 7.7 7.7z" fill="currentColor" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="2" y="3" width="12" height="8.5" rx="1.3" fill="none" stroke="currentColor" stroke-width="1.4" /><path d="M5.5 14h5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /><path d="M8 4.5v5.5a2.75 2.75 0 0 1 0-5.5z" fill="currentColor" /></svg>
      {/if}
      {#if labels}<span>{option.label}</span>{/if}
    </span>
  {/snippet}
</SegmentedControl>
<style>
  .choice { display: inline-flex; align-items: center; gap: .35rem; vertical-align: middle; }
</style>
