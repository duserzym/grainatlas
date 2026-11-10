<script lang="ts">
  import { app } from '../lib/store.svelte';
  let review: HTMLButtonElement;
  let continueButton: HTMLButtonElement;
  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') app.checkPrompt?.answer(false);
    if (e.key === 'Tab') {
      e.preventDefault();
      (document.activeElement === review ? continueButton : review)?.focus();
    }
  }
  $effect(() => review?.focus());
</script>
<div class="backdrop" role="presentation">
  <div class="glass panel dialog" role="alertdialog" tabindex="-1" onkeydown={key} aria-modal="true" aria-label="Project warnings" data-testid="check-dialog">
    <p>{app.checkPrompt?.summary}</p>
    {#if app.findings.some((f) => f.code === 'never_segmented')}
      <p class="faint small">Review, then open Grains and use Find grains in all areas.</p>
    {/if}
    <div class="row">
      <button bind:this={review} onclick={() => app.checkPrompt?.answer(false)}>Review</button>
      <button bind:this={continueButton} class="primary" onclick={() => app.checkPrompt?.answer(true)} data-testid="checks-continue">Continue</button>
    </div>
  </div>
</div>
<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; display: grid; place-items: center; background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .dialog { width: min(30rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: auto; background: var(--glass-strong); }
  p { overflow-wrap: anywhere; }
  .row { display: flex; justify-content: flex-end; gap: var(--space-2); }
</style>
