<script lang="ts">
  import { app } from '../lib/store.svelte';
  import Viewer from '../lib/canvas/Viewer.svelte';
  import type { View } from '../lib/canvas/view';
  import type { Core } from '../lib/core/core';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  let viewer = $state<ReturnType<typeof Viewer>>();
  let view = $state<View | undefined>(undefined);
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  let cursorUm = $state<[number, number] | null>(null);

  const areaId = $derived(app.areaId && app.stitched[app.areaId] ? app.areaId : Object.keys(app.stitched)[0] ?? null);
  const mosaic = $derived(areaId ? app.stitched[areaId].mosaic : null);
  const umPerPx = $derived(mosaic?.um_per_px ?? 1);

  $effect(() => {
    const t = areaId ? app.tone[areaId] : null;
    if (!t || !core) return;
    const snap = $state.snapshot(t);
    core.toneLut(snap).then((l) => (lut = l));
  });

  // A scale bar in screen space: the largest round length under ~140 px.
  const scalebar = $derived.by(() => {
    if (!view) return null;
    const umPerScreenPx = umPerPx / view.zoom;
    const target = 140 * umPerScreenPx;
    const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];
    const len = steps.filter((s) => s <= target).pop() ?? 1;
    return { len, px: len / umPerScreenPx };
  });
</script>

{#if core && areaId && mosaic}
  <div class="canvas">
    {#key areaId}
      <Viewer bind:this={viewer} bind:view {core} {areaId} {mosaic} {lut} version={app.pixelsVersion}
        onhover={(p) => (cursorUm = p ? [p.x * umPerPx, p.y * umPerPx] : null)} />
    {/key}

    <div class="glass topleft" data-ui>
      <select bind:value={app.areaId} aria-label="Area">
        {#each Object.keys(app.stitched) as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
      </select>
      <button class="icon" title="Zoom in (+)" onclick={() => viewer?.zoomBy(1.5)}>+</button>
      <button class="icon" title="Zoom out (−)" onclick={() => viewer?.zoomBy(1 / 1.5)}>−</button>
      <button class="icon" title="Fit (0)" onclick={() => viewer?.fitAll()}>Fit</button>
      <button class="icon" title="Pixels 1:1" onclick={() => viewer?.oneToOne()}>1:1</button>
      <button class="icon" title="Rotate left 15° ([)" onclick={() => viewer?.rotateBy(-Math.PI / 12)}>⟲</button>
      <button class="icon" title="Rotate right 15° (])" onclick={() => viewer?.rotateBy(Math.PI / 12)}>⟳</button>
    </div>

    <div class="glass status" data-ui>
      {#if cursorUm}<span class="num">x {cursorUm[0].toFixed(1)} µm, y {cursorUm[1].toFixed(1)} µm</span>{/if}
      {#if view}<span class="num muted">{(view.zoom * 100).toFixed(view.zoom < 0.1 ? 1 : 0)}% · {((view.rot * 180) / Math.PI).toFixed(0)}°</span>{/if}
      {#if scalebar}
        <span class="bar"><span style:width="{scalebar.px}px"></span>{scalebar.len >= 1000 ? `${scalebar.len / 1000} mm` : `${scalebar.len} µm`}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .canvas {
    position: absolute;
    inset: 0;
  }
  .topleft {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    display: flex;
    gap: var(--space-1);
    padding: var(--space-2);
    align-items: center;
  }
  .status {
    position: absolute;
    bottom: var(--space-2);
    left: var(--space-2);
    display: flex;
    gap: var(--space-4);
    padding: var(--space-1) var(--space-3);
    align-items: center;
    font-size: 0.85rem;
  }
  .bar {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .bar span {
    display: inline-block;
    height: 6px;
    border: 2px solid var(--text);
    border-top: 0;
  }
</style>
