<script lang="ts">
  import { app } from '../lib/store.svelte';
  import type { Progress } from '../lib/core/types';
  let { task = app.busy, onCancel = () => app.cancel(), testId = 'busy' }: { task?: { label: string; progress: Progress | null } | null; onCancel?: () => void; testId?: string } = $props();
  const pct = $derived(task?.progress ? Math.round(task.progress.fraction * 100) : null);
</script>

<div class="glass busy" role="status" aria-live="polite" data-testid={testId}>
  <div class="row">
    <span class="spinner" aria-hidden="true"></span>
    <strong>{task?.label}</strong>
    {#if task?.progress}<span class="muted">{task.progress.stage} {pct}%</span>{/if}
    <button onclick={onCancel}>Cancel</button>
  </div>
  <div class="bar"><div class="fill" style:transform="scaleX({(pct ?? 0) / 100})"></div></div>
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
    transform-origin: left;
    transition: transform var(--motion);
  }
  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--hairline);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner { animation: none; }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
