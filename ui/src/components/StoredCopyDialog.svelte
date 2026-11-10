<script lang="ts">
  const n = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
  import { app } from '../lib/store.svelte';
  import { answerStoredCopy, storedCopyCounts, storedCopyDate } from '../lib/stored-copy';

  const pending = $derived(app.storedCopy!);
  const date = $derived(storedCopyDate(pending.saved_at));
  const counts = $derived(storedCopyCounts(app.fullProject()!, pending.project));
  let restoreBtn: HTMLButtonElement;
  let keepBtn: HTMLButtonElement;
  $effect(() => keepBtn?.focus());

  function key(e: KeyboardEvent) {
    // Keep editing and navigation shortcuts underneath the modal from handling these keys.
    e.stopImmediatePropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      answerStoredCopy(app, false);
    } else if (e.key === 'Tab') {
      e.preventDefault();
      (document.activeElement === restoreBtn ? keepBtn : restoreBtn)?.focus();
    }
  }
</script>

<svelte:window onkeydowncapture={key} />
<div class="backdrop" role="presentation">
  <div class="glass panel dialog" role="alertdialog" aria-modal="true" aria-labelledby="stored-copy-title" aria-describedby="stored-copy-text" data-testid="stored-copy-dialog">
    <h2 id="stored-copy-title">Unsaved changes found</h2>
    <p id="stored-copy-text" class="muted">This app has changes to {pending.project.package.name} from {date} that are newer than the project file: {n(counts.areas, 'area', 'areas')} with grains, {n(counts.spots, 'spot', 'spots')} ({counts.extra} not in the file).</p>
    <div class="row">
      <button type="button" bind:this={keepBtn} onclick={() => answerStoredCopy(app, false)} data-testid="stored-copy-keep">Keep the file</button>
      <button type="button" class="primary" bind:this={restoreBtn} onclick={() => answerStoredCopy(app, true)} data-testid="stored-copy-restore">Restore changes</button>
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
    max-height: calc(100vh - 2rem);
    overflow: auto;
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
