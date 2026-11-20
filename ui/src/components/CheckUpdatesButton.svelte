<script lang="ts">
  import { tick } from 'svelte';
  import { isHosted } from '@lab/bridge';
  import { app } from '../lib/store.svelte';
  import { check, download, install, openWhatsNew, updater, updatePreview } from '../lib/updater.svelte';
  import UpdateNotes from './UpdateNotes.svelte';
  import { checkButtonView, RELEASES_PAGE } from '../lib/updates';

  const view = $derived(checkButtonView(updater.status, updater.version, updater.prefs.autoDownload));
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
    updater.whatsNewOpen = false;
    open = !open;
    // check() already ignores checks/downloads/installations in progress.
    if (open) void check(true);
  }
  function outside(event: PointerEvent) {
    if (open && event.target instanceof Node && !anchor?.contains(event.target)) open = false;
  }
  function dismissWhatsNew() {
    updater.whatsNewOpen = false;
    button?.focus();
  }
  $effect(() => {
    if (updater.whatsNewOpen) {
      open = false;
      void tick().then(() => popover?.querySelector<HTMLButtonElement>('button')?.focus());
    }
  });
  function escape(event: KeyboardEvent) {
    if ((open || updater.whatsNewOpen) && event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      open = false;
      dismissWhatsNew();
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

<svelte:window onpointerdown={outside} onkeydowncapture={escape} />

{#if !isHosted() || updatePreview}
  <span class="anchor" bind:this={anchor}>
    <button class="icon" bind:this={button} data-testid="check-updates"
      aria-label="Check for updates" title={`Check for updates · v${updater.version} · ${view.text}`}
      aria-busy={view.busy}
      aria-haspopup="dialog" aria-expanded={open || updater.whatsNewOpen} aria-controls={updater.whatsNewOpen ? 'whats-new' : open ? 'check-updates-popover' : undefined}
      data-badge={view.badge ? 'true' : undefined} data-busy={view.busy ? 'true' : undefined} onclick={toggle}>
      <svg class:rotating={view.busy} viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <path d="M13.5 6a5.75 5.75 0 0 0-10-1.5M2.5 2v3h3M2.5 10a5.75 5.75 0 0 0 10 1.5M13.5 14v-3h-3"
          fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      {#if view.badge}<span class="dot" aria-hidden="true"></span>{/if}
    </button>
    {#if updater.whatsNewOpen}
      <div class="glass panel" id="whats-new" data-testid="whats-new"
        role="dialog" aria-labelledby="whats-new-title" tabindex="-1" bind:this={popover} style:left={`${left}px`} style:top={`${top}px`}>
        <div class="card-heading">
          <strong id="whats-new-title">What's new in v{updater.version}</strong>
          <button class="icon" aria-label="Dismiss what's new" onclick={dismissWhatsNew}>✕</button>
        </div>
        <p class="small" style:white-space="pre-wrap">{updater.whatsNewNotes}</p>
      </div>
    {:else if open}
      <div class="glass panel" id="check-updates-popover" data-testid="check-updates-popover"
        role="dialog" aria-label="Updates" tabindex="-1" bind:this={popover} style:left={`${left}px`} style:top={`${top}px`}>
        <p role="status">{view.text}</p>
        {#if view.secondary}<p class="faint small">{view.secondary}</p>{/if}
        <UpdateNotes status={updater.status} />
        {#if view.details !== null}<p class="faint small" style:user-select="text">Details: {view.details}</p>{/if}
        {#if view.action === 'download'}<button onclick={download}>Download</button>
        {:else if view.action === 'install'}<button class="primary" onclick={install}>Restart to update</button>
        {:else if view.action === 'releases' || updater.status.kind === 'unsupported'}<a class="btn" href={RELEASES_PAGE} target="_blank" rel="noreferrer">Open download page</a>{/if}
        <a class="small" href="#whats-new" onclick={(event) => { event.preventDefault(); openWhatsNew(); }}>What's new</a>
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
  .card-heading { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); width: 100%; }
  p { margin: 0; overflow-wrap: anywhere; }
  .small { font-size: 0.8rem; }
  .rotating { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .rotating { animation: none; } }
</style>
