<script lang="ts">
  import { onMount } from 'svelte';
  let { onchoose }: { onchoose: (choice: 'sync' | 'keep-draft' | 'stay') => void } = $props();
  let dialog: HTMLDialogElement;
  onMount(() => dialog.showModal());
</script>
<dialog bind:this={dialog} class="glass panel" aria-label="Leave Edit" tabindex="-1" onkeydown={e => e.stopPropagation()} oncancel={e => { e.preventDefault(); onchoose('stay'); }}>
  <h2>Leave Edit?</h2>
  <p>Your changes have not been synced to the database.</p>
  <div class="row">
    <button class="primary" onclick={() => onchoose('sync')}>Sync and leave</button>
    <button onclick={() => onchoose('keep-draft')}>Keep draft</button>
    <button onclick={() => onchoose('stay')}>Stay</button>
  </div>
</dialog>
