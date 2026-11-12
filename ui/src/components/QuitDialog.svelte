<script lang="ts">
  import { onMount } from 'svelte';

  let { titles, answer }: {
    titles: string[];
    answer: (choice: 'save' | 'discard' | 'cancel') => void;
  } = $props();
  let dialog: HTMLDialogElement;
  let cancel: HTMLButtonElement;

  onMount(() => {
    dialog.showModal();
    cancel.focus();
    return () => dialog.close();
  });
</script>

<dialog
  bind:this={dialog}
  class="glass panel"
  role="alertdialog"
  aria-labelledby="quit-title"
  aria-describedby="quit-description"
  oncancel={(event) => { event.preventDefault(); answer('cancel'); }}
>
  <h2 id="quit-title">Quit GrainAtlas?</h2>
  <p id="quit-description">
    {titles.length === 1 ? 'This project has unsaved changes:' : 'These projects have unsaved changes:'}
  </p>
  <ul>
    {#each titles as title}
      <li>{title}</li>
    {/each}
  </ul>
  <div class="actions">
    <button bind:this={cancel} onclick={() => answer('cancel')}>Cancel</button>
    <button onclick={() => answer('discard')}>Quit without saving</button>
    <button class="primary" onclick={() => answer('save')}>Save and quit</button>
  </div>
</dialog>

<style>
  dialog.glass {
    width: min(31rem, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    color: var(--text);
    background: var(--glass-strong);
  }
  dialog::backdrop { background: color-mix(in srgb, var(--bg) 55%, transparent); }
  h2 { margin-top: 0; }
  ul { max-height: 12rem; overflow: auto; overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); }
</style>
