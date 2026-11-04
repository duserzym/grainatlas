<script lang="ts">
  // A glass panel that can be dragged anywhere over the canvas by its header.
  import type { Snippet } from 'svelte';

  let { title, children, x = $bindable(24), y = $bindable(120), onclose }: { title: string; children: Snippet; x?: number; y?: number; onclose?: () => void } = $props();

  let drag: { dx: number; dy: number } | null = null;
  function down(e: PointerEvent) {
    if ((e.target as HTMLElement).closest('button')) return;
    drag = { dx: e.clientX - x, dy: e.clientY - y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    x = Math.max(0, e.clientX - drag.dx);
    y = Math.max(0, e.clientY - drag.dy);
  }
</script>

<div class="glass floating" style:left="{x}px" style:top="{y}px" style:max-height="calc(100% - {y + 8}px)" data-ui role="dialog" aria-label={title}>
  <header role="toolbar" tabindex="-1" onpointerdown={down} onpointermove={move} onpointerup={() => (drag = null)}>
    <span class="grip" aria-hidden="true">⠿</span>
    <strong>{title}</strong>
    {#if onclose}<button class="icon close" onclick={onclose} aria-label="Close">✕</button>{/if}
  </header>
  <div class="body">{@render children()}</div>
</div>

<style>
  .floating {
    position: absolute;
    width: 19rem;
    z-index: 20;
    background: var(--glass-strong);
    display: flex;
    flex-direction: column;
    min-height: 8rem;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    cursor: grab;
    border-bottom: 1px solid var(--hairline);
    touch-action: none;
    user-select: none;
  }
  header:active {
    cursor: grabbing;
  }
  .grip {
    color: var(--text-faint);
  }
  .close {
    margin-left: auto;
    padding: 0 0.4rem;
    min-width: 0;
  }
  .body {
    padding: var(--space-3);
    display: grid;
    gap: var(--space-2);
    font-size: 0.88rem;
    overflow: auto;
    min-height: 0;
  }
</style>
