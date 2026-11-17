<script lang="ts">
  import { app, current } from '../lib/store.svelte';
  import { loadPublishedPixels } from '../lib/remote';
</script>

{#if app.pixelsLoad}
  <div class="glass panel" role="status" data-testid="pixels-loading">
    <span class="small">
      {#if app.pixelsLoad.error}Full-resolution images: {app.pixelsLoad.error}
      {:else}Loading full-resolution images: {app.pixelsLoad.done} of {app.pixelsLoad.total} tiles.{/if}
      Tone and print sheets become editable when this finishes.
    </span>
    {#if app.cancelPixelsLoad && !app.pixelsLoad.error}
      <button onclick={() => app.cancelPixelsLoad?.()}>Stop</button>
    {:else}
      <button disabled={!!app.cancelPixelsLoad} onclick={() => void loadPublishedPixels(current())}>Retry</button>
    {/if}
  </div>
{/if}

<style>
  .panel { display:flex; align-items:center; gap:var(--space-2); margin:0 var(--space-2) var(--space-2); padding:var(--space-2) var(--space-3); }
  span { flex:1; min-width:0; overflow-wrap:anywhere; }
  button { flex:none; }
</style>
