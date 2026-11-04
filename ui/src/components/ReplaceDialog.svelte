<script lang="ts">
  // UPb Studio handed over files while a project is open: ask once before
  // replacing it (Open, Cancel). The open project is saved first, so nothing
  // of the user's work is lost either way.
  import { app } from '../lib/store.svelte';
  import { answerOffer } from '../lib/actions';

  const offer = $derived(app.offer!);
  const current = app.pkg?.name ?? (app.remote ? 'the project from the server' : 'the open project');
  let openBtn: HTMLButtonElement;
  $effect(() => openBtn?.focus());

  function open() {
    app.persist();
    void answerOffer(true);
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') void answerOffer(false);
  }
</script>

<svelte:window onkeydown={key} />
<div class="backdrop" role="presentation">
  <div class="glass panel dialog" role="alertdialog" aria-modal="true" aria-labelledby="replace-title" aria-describedby="replace-text" data-testid="replace-dialog">
    <h2 id="replace-title">Open {offer.name}?</h2>
    <p id="replace-text" class="muted">
      It replaces <strong>{current}</strong> here.
      {#if app.remote}Open the project from its link again to come back to it.{:else}Its grains, spots and parameters stay saved in this browser: open the same package again to continue (stitched mosaics are rebuilt).{/if}
    </p>
    <div class="row">
      <button type="button" onclick={() => answerOffer(false)} data-testid="replace-cancel">Cancel</button>
      <button type="button" class="primary" bind:this={openBtn} onclick={open} data-testid="replace-open">Open</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg) 55%, transparent);
  }
  .dialog {
    width: min(30rem, calc(100vw - 2rem));
    display: grid;
    gap: var(--space-3);
    background: var(--glass-strong);
  }
  .dialog h2,
  .dialog p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
