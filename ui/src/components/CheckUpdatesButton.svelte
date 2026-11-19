<script lang="ts">
  import { tick } from 'svelte';
  import { isHosted } from '@lab/bridge';
  import { app } from '../lib/store.svelte';
  import { check, download, install, updater, updatePreview } from '../lib/updater.svelte';
  import { checkButtonView, RELEASES_PAGE } from '../lib/updates';

  const view = $derived(checkButtonView(updater.status, updater.version, updater.prefs.autoDownload));
  const info = $derived('info' in updater.status ? updater.status.info : null);
  let open = $state(false);
  let anchor = $state<HTMLSpanElement>();
  let button = $state<HTMLButtonElement>();
  let popover = $state<HTMLDivElement>();
  let left = $state(0);
  let top = $state(0);

  function position() {
    if (!popover || !anchor) return;
    const at = anchor.getBoundingClientRect();
    const { width, height } = popover.getBoundingClientRect();
    const x = Math.max(8, Math.min(at.left, innerWidth - width - 8));
    const y = at.bottom + 8 + height <= innerHeight - 8 ? at.bottom + 8 : Math.max(8, at.top - height - 8);
    left = x - at.left;
    top = y - at.top;
  }
  async function afterResize() {
    // The header moves its theme switch into overflow at the narrow breakpoint.
    // Measure after that DOM update, since the anchor can move without resizing.
    await tick();
    position();
  }
  $effect(() => {
    if (!popover || !anchor) return;
    position();
    const observer = new ResizeObserver(position);
    observer.observe(popover);
    observer.observe(anchor);
    // Moving controls into overflow changes the row's width, not the icon's.
    if (anchor.parentElement) observer.observe(anchor.parentElement);
    window.addEventListener('resize', afterResize);
    window.addEventListener('scroll', position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', afterResize);
      window.removeEventListener('scroll', position, true);
    };
  });
  function toggle() {
    open = !open;
    // check() already ignores checks/downloads/installations in progress.
    if (open) void check(true);
  }
  function outside(event: PointerEvent) {
    if (open && event.target instanceof Node && !anchor?.contains(event.target)) open = false;
  }
  function escape(event: KeyboardEvent) {
    if (open && event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      open = false;
      button?.focus();
    }
  }
  async function openSettings(event: MouseEvent) {
    event.preventDefault();
    open = false;
    app.screen = 'settings';
    await tick();
    const panel = document.getElementById('updates');
    panel?.scrollIntoView({ block: 'center' });
    panel?.querySelector<HTMLElement>('button, a')?.focus({ preventScroll: true });
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={escape} />

{#if !isHosted() || updatePreview}
  <span class="anchor" bind:this={anchor}>
    <button class="icon" bind:this={button} data-testid="check-updates"
      aria-label="Check for updates" title={`Check for updates · v${updater.version} · ${view.text}`}
      aria-busy={view.busy}
      aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? 'check-updates-popover' : undefined}
      data-badge={view.badge ? 'true' : undefined} data-busy={view.busy ? 'true' : undefined} onclick={toggle}>
      <svg class:rotating={updater.status.kind === 'checking'} viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <path d="M12.76 5.25a5.5 5.5 0 1 0 0 5.5M12.76 1.75v3.5h-3.5"
          fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      {#if view.badge}<span class="dot" aria-hidden="true"></span>{/if}
    </button>
    {#if open}
      <div class="glass panel" id="check-updates-popover" data-testid="check-updates-popover"
        role="dialog" aria-label="Updates" tabindex="-1" bind:this={popover} style:left={`${left}px`} style:top={`${top}px`}>
        <p role="status">{view.text}</p>
        {#if updater.status.kind === 'unsupported' && info}<p class="small">{updater.status.reason}</p>{/if}
        {#if info}<p class="faint small">Current version: v{info.currentVersion}</p>{/if}
        {#if view.action === 'download'}<button onclick={download}>Download</button>
        {:else if view.action === 'install'}<button class="primary" onclick={install}>Restart to update</button>
        {:else if view.action === 'releases' || updater.status.kind === 'unsupported'}<a class="btn" href={RELEASES_PAGE} target="_blank" rel="noreferrer">Open download page</a>{/if}
        <a class="small" href="#/settings" onclick={openSettings}>Update settings…</a>
      </div>
    {/if}
  </span>
{/if}

<style>
  .anchor { position: relative; display: inline-flex; flex: none; align-self: center; }
  button.icon { position: relative; display: grid; place-items: center; width: 28px; height: 28px; min-width: 28px; padding: 0; }
  .dot { position: absolute; top: 2px; right: 2px; width: 6px; height: 6px; border-radius: 50%; background: var(--accent); }
  .panel {
    position: absolute; z-index: 10; display: grid; justify-items: start; gap: var(--space-2);
    width: min(320px, calc(100vw - 16px)); max-height: calc(100vh - 16px); overflow: auto;
    padding: var(--space-3); background: var(--glass-strong); white-space: normal;
  }
  p { margin: 0; overflow-wrap: anywhere; }
  .small { font-size: 0.8rem; }
  .rotating { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .rotating { animation: none; } }
</style>
