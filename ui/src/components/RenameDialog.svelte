<script lang="ts">
  import { app } from '../lib/store.svelte';
  let dialog: HTMLDivElement;
  let cancel: HTMLButtonElement;
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); app.renamePrompt?.answer('cancel'); }
    if (e.key === 'Tab') {
      const buttons = [...dialog.querySelectorAll('button')];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      e.preventDefault();
      buttons[(index + (e.shiftKey ? -1 : 1) + buttons.length) % buttons.length]?.focus();
    }
  }
  $effect(() => { if (app.renamePrompt) cancel?.focus(); });
</script>
<div class="backdrop" role="presentation">
  <div bind:this={dialog} class="glass panel dialog" role="dialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-labelledby="rename-title" data-testid="rename-dialog">
    <h2 id="rename-title">Rename from pattern</h2>
    <p>Rename with sample {app.renamePrompt?.newSample} (was {app.renamePrompt?.oldSample})?</p>
    <div class="row">
      <button class="primary" onclick={() => app.renamePrompt?.answer('use')}>Use {app.renamePrompt?.newSample}</button>
      <button onclick={() => app.renamePrompt?.answer('keep')}>Keep {app.renamePrompt?.oldSample}</button>
      <button bind:this={cancel} onclick={() => app.renamePrompt?.answer('cancel')}>Cancel</button>
    </div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(32rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  h2, p, button { overflow-wrap: anywhere; }
  .row { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); }
  button { min-width: 0; max-width: 100%; }
</style>
