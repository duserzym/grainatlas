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
    // Opened from the window's close button, the focus may not be in the page yet: keys then
    // reach the window, not the dialog. While it is open, Escape cancels and no key reaches
    // the screens behind it (the canvas cleared its selection on that Escape).
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopImmediatePropagation();
        answer('cancel');
      } else if (!dialog.contains(e.target as Node)) {
        e.stopImmediatePropagation();
        if (e.key === 'Tab' || e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          cancel.focus();
        }
      }
    };
    window.addEventListener('keydown', key, true);
    return () => {
      window.removeEventListener('keydown', key, true);
      dialog.close();
    };
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
