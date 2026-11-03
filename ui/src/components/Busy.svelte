<script lang="ts">
  import { app } from '../lib/store.svelte';
  const pct = $derived(app.busy?.progress ? Math.round(app.busy.progress.fraction * 100) : null);
</script>

<div class="glass busy" role="status" aria-live="polite" data-testid="busy">
  <div class="row">
    <span class="spinner" aria-hidden="true"></span>
    <strong>{app.busy?.label}</strong>
    {#if app.busy?.progress}<span class="muted">{app.busy.progress.stage} {pct}%</span>{/if}
    <button onclick={() => app.cancel()}>Cancel</button>
  </div>
  <div class="bar"><div class="fill" style:width="{pct ?? 0}%"></div></div>
</div>

<style>
  .busy {
    position: fixed;
    top: 4.5rem;
    left: 50%;
    transform: translateX(-50%);
    padding: var(--space-3) var(--space-4);
    min-width: 26rem;
    z-index: 40;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .row button {
    margin-left: auto;
  }
  .bar {
    margin-top: var(--space-2);
    height: 4px;
    background: var(--hairline);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width var(--motion);
  }
  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--hairline);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
