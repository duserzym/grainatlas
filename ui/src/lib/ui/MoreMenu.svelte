<!-- Local menu companion to the ported controls; uses the shared icon button and tokens. -->
<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import IconButton from './IconButton.svelte';
  import { menuIndex } from '../ui-controls';
  let { label, title, children }: { label: string; title?: string; children: Snippet } = $props();
  let open = $state(false);
  let root: HTMLDivElement;
  let menu = $state<HTMLDivElement>();
  const uid = $props.id();
  function items() { return [...menu!.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]:not(:disabled)')]; }
  function close(restore = true) { open = false; if (restore) root.querySelector<HTMLButtonElement>('button')?.focus(); }
  async function show(last = false) { open = true; await tick(); if (!open || !menu) return; const buttons = items(); (last ? buttons.at(-1) : buttons[0])?.focus(); }
  function keyboard(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === 'Tab') close();
    else {
      const buttons = items(), index = buttons.indexOf(e.target as HTMLButtonElement);
      const next = menuIndex(e.key, index, buttons.length);
      if (next !== null) { e.preventDefault(); e.stopPropagation(); buttons[next]?.focus(); }
    }
  }
  function outside(e: PointerEvent) { if (open && !root.contains(e.target as Node)) close(false); }
</script>
<svelte:window onpointerdown={outside} />
<div class="more" bind:this={root}>
  <IconButton icon="more" {label} {title} aria-haspopup="menu" aria-expanded={open} aria-controls={open ? uid : undefined}
    onclick={() => open ? close() : show()}
    onkeydown={e => { if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); void show(e.key === 'ArrowUp'); } }} />
  {#if open}
    <div id={uid} class="glass panel menu" role="menu" tabindex="-1" aria-label={label} bind:this={menu} onkeydown={keyboard}
      onclick={e => { if ((e.target as Element).closest('[role="menuitemradio"]')) close(); }}>
      {@render children()}
    </div>
  {/if}
</div>
<style>
  .more { position: relative; flex: none; }
  .menu { position: absolute; top: calc(100% + var(--space-1)); right: 0; display: grid; min-width: 10rem; padding: var(--space-1); z-index: 30; }
  .menu :global(button) { text-align: left; }
  .menu :global(button[aria-checked=true]) { background: var(--accent-soft); color: var(--accent); }
</style>
