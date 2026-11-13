<script lang="ts">
  // A glass panel that can be dragged anywhere over the canvas by its header.
  // Its place is the caller's (remembered); it is kept inside the area it floats over, also
  // when the window gets smaller (a place saved on a large monitor, opened on a laptop).
  import { onMount, type Snippet } from 'svelte';

  let { title, children, x = $bindable(24), y = $bindable(120), onclose, onmoved, minimized = false, onminimize, passive = false }: { title: string; children: Snippet; x?: number; y?: number; onclose?: () => void; onmoved?: () => void; minimized?: boolean; onminimize?: () => void; passive?: boolean } = $props();
  let el: HTMLDivElement;

  /** Keep the whole panel inside the parent; its body scrolls when constrained. */
  function clamp() {
    const p = el?.offsetParent as HTMLElement | null;
    if (!p) return;
    const maxX = Math.max(0, p.clientWidth - el.offsetWidth);
    const maxY = Math.max(0, p.clientHeight - el.offsetHeight - 8);
    const nx = Math.min(Math.max(0, x), maxX);
    const ny = Math.min(Math.max(0, y), maxY);
    if (nx !== x) x = nx;
    if (ny !== y) y = ny;
  }
  onMount(() => {
    clamp();
    const ro = new ResizeObserver(clamp);
    if (el.offsetParent) ro.observe(el.offsetParent);
    ro.observe(el);
    return () => ro.disconnect();
  });

  let drag: { dx: number; dy: number } | null = null;
  function down(e: PointerEvent) {
    if ((e.target as HTMLElement).closest('button')) return;
    drag = { dx: e.clientX - x, dy: e.clientY - y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    x = e.clientX - drag.dx;
    y = e.clientY - drag.dy;
    clamp();
  }
  function up() {
    if (!drag) return;
    drag = null;
    onmoved?.();
  }
</script>

<div class="glass floating" class:minimized bind:this={el} inert={passive} style:pointer-events={passive ? 'none' : undefined} style:left="{x}px" style:top="{y}px" style:max-height="calc(100% - {y + 8}px)" data-ui role="dialog" aria-label={title}>
  <header role="toolbar" tabindex="-1" onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onlostpointercapture={up}>
    <span class="grip" aria-hidden="true">⠿</span>
    <strong {title}>{title}</strong>
    {#if onminimize && !passive}
      <button class="icon minimize" aria-expanded={!minimized} onclick={onminimize}
        aria-label={minimized ? 'Expand spot and grain info' : 'Minimize spot and grain info'}
        title={minimized ? 'Expand info panel' : 'Minimize info panel'}>{minimized ? '⌄' : '⌃'}</button>
    {/if}
    {#if onclose && !passive}<button class="icon close" onclick={onclose} aria-label="Close">✕</button>{/if}
  </header>
  {#if !minimized}<div class="body">{@render children()}</div>{/if}
</div>

<style>
  .floating {
    position: absolute;
    width: min(19rem, 100%);
    z-index: 20;
    background: var(--glass-strong);
    display: flex;
    flex-direction: column;
    min-height: min(8rem, calc(100% - 8px));
  }
  header {
    display: flex;
    flex: none;
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
  .minimized {width:min(16rem,100%);min-height:0;}
  .minimized header {padding-block:var(--space-1);border-bottom:0;}
  header strong {min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
  .grip {
    color: var(--text-faint);
  }
  .minimize {margin-left:auto;}
  .minimize, .close {
    flex:none;
    padding: 0 0.4rem;
    min-width: 0;
  }
  .body {
    padding: var(--space-3);
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-2);
    grid-auto-rows: max-content;
    align-content: start;
    flex: 1 1 auto;
    font-size: 0.88rem;
    overflow: auto;
    min-height: 0;
  }
</style>
