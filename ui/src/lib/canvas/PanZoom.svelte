<script lang="ts">
  // Pan and zoom for any content of a known size (the layout page preview),
  // with the same gestures and feel as the mosaic views (./gestures.ts). No
  // rotation; the content is fitted on mount and whenever `fitKey` changes.
  import { onMount, type Snippet } from 'svelte';
  import { attachGestures, type GestureOptions } from './gestures';
  import { type View, fit, matrix, zoomAt } from './view';
  import { registerZoomView } from '../zoom';

  let { w, h, fitKey = '', label = 'Preview', children, gestures = {} }: { w: number; h: number; fitKey?: string; label?: string; children: Snippet; gestures?: Partial<Pick<GestureOptions, 'onpress' | 'ondrag' | 'onrelease' | 'oncancel' | 'ondblclick' | 'canNudge'>> } = $props();

  let host: HTMLDivElement;
  let size = $state({ w: 800, h: 600 });
  let view = $state<View>(fit(800, 600, 1, 1));
  const m = $derived(matrix(view));

  function fitAll() {
    view = fit(size.w, size.h, w, h);
  }

  $effect(() => {
    void fitKey;
    void w;
    void h;
    fitAll();
  });

  onMount(() => {
    const unregisterZoom = registerZoomView(host, { zoomBy, fit: fitAll });
    const ro = new ResizeObserver(() => {
      const r = host.getBoundingClientRect();
      const first = size.w === 800 && size.h === 600;
      size = { w: Math.max(1, r.width), h: Math.max(1, r.height) };
      if (first) fitAll();
      else view = { ...view, w: size.w, h: size.h };
    });
    ro.observe(host);
    const detach = attachGestures({ host, getView: () => view, setView: (v) => (view = v), fit: fitAll, rotate: false,
      onpress: p => gestures.onpress?.(p) ?? false, ondrag: p => gestures.ondrag?.(p), onrelease: p => gestures.onrelease?.(p),
      oncancel: () => gestures.oncancel?.(), ondblclick: p => gestures.ondblclick?.(p), canNudge: () => gestures.canNudge?.() ?? true });
    return () => {
      unregisterZoom();
      ro.disconnect();
      detach();
    };
  });

  export function zoomBy(f: number) {
    view = zoomAt(view, size.w / 2, size.h / 2, f);
  }
  export { fitAll };
</script>

<div class="panzoom" bind:this={host} role="application" aria-label={label} data-view>
  <div class="content" style:width="{w}px" style:height="{h}px" style:transform="matrix({m.join(',')})">
    {@render children()}
  </div>
</div>

<style>
  .panzoom {
    position: absolute;
    inset: 0;
    overflow: hidden;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    cursor: grab;
  }
  .panzoom:active {
    cursor: grabbing;
  }
  .content {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
  }
  .content :global(img) {
    -webkit-user-drag: none;
    pointer-events: none;
  }
</style>
