<script lang="ts">
  // The infinite canvas: a 2D canvas drawing the pyramid level closest to the
  // current zoom (or a live preview of the whole mosaic while a photometric
  // control moves), plus an SVG overlay in mosaic-pixel coordinates. Without a
  // mosaic (an area not stitched yet) only the overlay is drawn, over `extent`.
  // Gestures come from ./gestures.ts, shared with every image view: mouse wheel
  // zooms about the cursor, two-finger scroll pans, pinch zooms (ctrl+wheel in
  // Chromium, gesture events in WebKit), drag pans, double-click fits, arrows
  // nudge; a click without movement goes to the active tool.
  import { onMount, untrack, type Snippet } from 'svelte';
  import type { Core, RawImage } from '../core/core';
  import type { MosaicJson } from '../core/types';
  import { TileCache } from './tiles';
  import { type View, fit, matrix, rotateAt, toMosaic, zoomAt } from './view';
  import { attachGestures, type ViewPoint } from './gestures';
  import { ringLuminance } from './contrast';

  /** A point on the canvas: `x`, `y` in mosaic pixels, `sx`, `sy` on screen. */
  export type CanvasPoint = ViewPoint;

  let {
    core,
    areaId,
    mosaic,
    extent,
    lut,
    version = 0,
    preview = null,
    onlevel,
    ondraw,
    view = $bindable(),
    cursor = 'default',
    overlay,
    screenOverlay,
    onpress,
    ondrag,
    onrelease,
    oncancel,
    onclick,
    ondblclick,
    onhover,
    canNudge,
    rotate = true,
  }: {
    /** Where pyramid tiles come from: the core, or a published bundle on a server. */
    core: Pick<Core, 'pyramidTile'>;
    areaId: string;
    mosaic: MosaicJson | null;
    /** Size in mosaic pixels when there is no mosaic. */
    extent?: { w: number; h: number };
    lut: Uint8Array;
    version?: number;
    /** Live preview: the whole mosaic at pyramid `level`; drawn instead of the tiles while `over === version`. */
    preview?: { level: number; img: RawImage; over: number } | null;
    /** The pyramid level drawn (on change). */
    onlevel?: (level: number) => void;
    /** The displayed pixels changed, including asynchronously loaded tiles. */
    ondraw?: () => void;
    view?: View;
    cursor?: string;
    overlay?: Snippet<[View]>;
    screenOverlay?: Snippet<[View]>;
    onpress?: (p: CanvasPoint) => boolean;
    ondrag?: (p: CanvasPoint) => void;
    onrelease?: (p: CanvasPoint) => void;
    oncancel?: () => void;
    onclick?: (p: CanvasPoint) => void;
    /** Return true when handled; otherwise a double-click fits the view. */
    ondblclick?: (p: CanvasPoint) => boolean | void;
    onhover?: (p: CanvasPoint | null) => void;
    /** Arrow keys pan unless this returns false. */
    canNudge?: () => boolean;
    /** Rotation gestures (default on). */
    rotate?: boolean;
  } = $props();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let cache: TileCache | null = null;
  let size = $state({ w: 800, h: 600 });
  let raf = 0;

  const ext = $derived(mosaic ? { w: mosaic.w, h: mosaic.h } : (extent ?? { w: 1, h: 1 }));
  const v = $derived<View>(view ?? fit(size.w, size.h, ext.w, ext.h));
  let previewBmp: ImageBitmap | null = null;
  let previewFor: unknown = null;
  let lastLevel = -1;

  // The preview as a bitmap through the display LUT (rebuilt when either changes).
  $effect(() => {
    const p = preview, l = lut;
    if (!p) {
      previewBmp?.close();
      previewBmp = null;
      previewFor = null;
      schedule();
      return;
    }
    const tag = {};
    previewFor = tag;
    const rgba = new Uint8ClampedArray(p.img.w * p.img.h * 4);
    const d = p.img.data;
    for (let i = 0, j = 0; i < d.length; i++, j += 4) {
      const g = l[d[i]];
      rgba[j] = rgba[j + 1] = rgba[j + 2] = g;
      rgba[j + 3] = 255;
    }
    createImageBitmap(new ImageData(rgba, p.img.w, p.img.h)).then((b) => {
      if (previewFor !== tag) {
        b.close();
        return;
      }
      previewBmp?.close();
      previewBmp = b;
      schedule();
    });
  });
  const m = $derived(matrix(v));

  function setView(nv: View) {
    view = nv;
  }

  function schedule() {
    if (!raf) raf = requestAnimationFrame(draw);
  }

  function draw() {
    raf = 0;
    if (!canvas || !cache) return;
    try {
      drawImage(cache);
    } finally {
      ondraw?.();
    }
  }

  function drawImage(cache: TileCache) {
    const dpr = window.devicePixelRatio || 1;
    const W = Math.round(size.w * dpr), H = Math.round(size.h * dpr);
    if (canvas.width !== W || canvas.height !== H) {
      canvas.width = W;
      canvas.height = H;
    }
    const ctx = canvas.getContext('2d')!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = getComputedStyle(host).getPropertyValue('--image-bg').trim() || '#05070b';
    ctx.fillRect(0, 0, W, H);
    if (!mosaic) return;
    const levels = mosaic.levels;
    if (!levels.length) return;
    const vv = v;
    const maxL = levels.length - 1;
    const L = Math.max(0, Math.min(maxL, Math.floor(Math.log2(1 / (vv.zoom * dpr)))));
    if (L !== lastLevel) {
      lastLevel = L;
      onlevel?.(L);
    }
    ctx.setTransform(...matrix(vv, dpr));
    ctx.imageSmoothingEnabled = vv.zoom * dpr < 1;
    ctx.imageSmoothingQuality = 'high';
    // Live preview: alone while it is newer than the tiles, else as a backdrop while they load.
    const drawPreview = () => {
      if (!previewBmp || !preview) return;
      const f = 1 << preview.level;
      ctx.imageSmoothingEnabled = true;
      ctx.drawImage(previewBmp, 0, 0, previewBmp.width * f, previewBmp.height * f);
      ctx.imageSmoothingEnabled = vv.zoom * dpr < 1;
    };
    if (previewBmp && preview && preview.over === version) {
      drawPreview();
      return;
    }
    // Visible mosaic bounds.
    const corners = [toMosaic(vv, 0, 0), toMosaic(vv, size.w, 0), toMosaic(vv, 0, size.h), toMosaic(vv, size.w, size.h)];
    const x0 = Math.max(0, Math.min(...corners.map((c) => c[0])));
    const y0 = Math.max(0, Math.min(...corners.map((c) => c[1])));
    const x1 = Math.min(mosaic.w, Math.max(...corners.map((c) => c[0])));
    const y1 = Math.min(mosaic.h, Math.max(...corners.map((c) => c[1])));
    if (x1 <= x0 || y1 <= y0) return;
    const T = mosaic.tile_size;
    const span = T * (1 << L);
    const info = levels[L];
    const eps = 0.5 / (vv.zoom * dpr); // hide hairline gaps between tiles
    const r0 = Math.floor(y0 / span), r1 = Math.min(info.rows - 1, Math.floor(y1 / span));
    const c0 = Math.floor(x0 / span), c1 = Math.min(info.cols - 1, Math.floor(x1 / span));
    const got = new Map<string, ImageBitmap | null>();
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) got.set(`${c}/${r}`, cache.get(areaId, L, c, r));
    // The backdrop only while some visible tile is still loading.
    if ([...got.values()].some((b) => !b)) drawPreview();
    for (let r = r0; r <= r1; r++) {
      for (let c = c0; c <= c1; c++) {
        const b = got.get(`${c}/${r}`);
        if (b) {
          ctx.drawImage(b, c * span, r * span, b.width * (1 << L) + eps, b.height * (1 << L) + eps);
        } else {
          // Fallback: the nearest coarser level already loaded.
          for (let k = L + 1; k <= maxL; k++) {
            const f = 1 << (k - L);
            const pc = Math.floor(c / f), pr = Math.floor(r / f);
            const pb = cache.peek(areaId, k, pc, pr);
            if (!pb) continue;
            const sub = T / f;
            const sx = (c % f) * sub, sy = (r % f) * sub;
            const sw = Math.min(sub, pb.width - sx), sh = Math.min(sub, pb.height - sy);
            if (sw > 0 && sh > 0) ctx.drawImage(pb, sx, sy, sw, sh, c * span, r * span, sw * (1 << k) + eps, sh * (1 << k) + eps);
            break;
          }
        }
      }
    }
  }

  $effect(() => {
    core;
    cache?.clear();
    cache = new TileCache(core);
    cache.onLoad = schedule;
    cache.setLut(untrack(() => lut));
    schedule();
  });
  $effect(() => {
    cache?.setLut(lut);
    schedule();
  });
  let seen = '';
  $effect(() => {
    const key = `${version}/${areaId}`;
    if (key !== seen) {
      if (seen) cache?.clear();
      seen = key;
    }
    schedule();
  });
  $effect(() => {
    v;
    schedule();
  });

  onMount(() => {
    const ro = new ResizeObserver(() => {
      const r = host.getBoundingClientRect();
      const old = size;
      size = { w: Math.max(1, r.width), h: Math.max(1, r.height) };
      if (view) view = { ...view, w: size.w, h: size.h };
      else view = fit(size.w, size.h, ext.w, ext.h);
      if (old.w !== size.w || old.h !== size.h) schedule();
    });
    ro.observe(host);
    const detach = attachGestures({
      host,
      getView: () => v,
      setView,
      fit: () => fitAll(),
      rotate,
      onpress: (p) => onpress?.(p) ?? false,
      ondrag: (p) => ondrag?.(p),
      onrelease: (p) => onrelease?.(p),
      oncancel: () => oncancel?.(),
      onclick: (p) => onclick?.(p),
      ondblclick: (p) => ondblclick?.(p),
      onhover: (p) => onhover?.(p),
      canNudge: () => canNudge?.() ?? true,
    });
    return () => {
      ro.disconnect();
      detach();
      cancelAnimationFrame(raf);
      cache?.clear();
      previewBmp?.close();
    };
  });

  // Public helpers for toolbars.
  /** Screen CSS pixels relative to the Viewer, sampling the displayed ring. */
  export function luminanceAt(sx: number, sy: number, r: number): number | null {
    return canvas ? ringLuminance(canvas, sx, sy, r, window.devicePixelRatio || 1) : null;
  }
  export function zoomBy(f: number) {
    setView(zoomAt(v, size.w / 2, size.h / 2, f));
  }
  export function rotateBy(rad: number) {
    setView(rotateAt(v, size.w / 2, size.h / 2, rad));
  }
  export function fitAll() {
    setView(fit(size.w, size.h, ext.w, ext.h, 0));
  }
  export function centerOn(x: number, y: number, zoom?: number) {
    setView({ ...v, cx: x, cy: y, zoom: zoom ?? v.zoom });
  }
  export function oneToOne() {
    setView(zoomAt(v, size.w / 2, size.h / 2, 1 / (v.zoom * (window.devicePixelRatio || 1))));
  }
</script>

<div
  class="viewer"
  bind:this={host}
  style:cursor
  role="application"
  aria-label="Mosaic canvas"
  data-view
>
  <canvas bind:this={canvas} style:width="{size.w}px" style:height="{size.h}px"></canvas>
  <svg class="overlay" width={size.w} height={size.h}>
    <g transform="matrix({m.join(' ')})">
      {@render overlay?.(v)}
    </g>
    {@render screenOverlay?.(v)}
  </svg>
</div>

<style>
  .viewer {
    position: absolute;
    inset: 0;
    overflow: hidden;
    touch-action: none;
    background: var(--image-bg);
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
  .overlay {
    position: absolute;
    inset: 0;
    overflow: visible;
  }
  .overlay :global(*) {
    vector-effect: non-scaling-stroke;
  }
  .viewer, .viewer :global(*) {
    user-select: none;
    -webkit-user-select: none;
    -webkit-user-drag: none;
  }
</style>
