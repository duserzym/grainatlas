<script lang="ts">
  // The infinite canvas: a 2D canvas drawing the pyramid level closest to the
  // current zoom (or a live preview of the whole mosaic while a photometric
  // control moves), plus an SVG overlay in mosaic-pixel coordinates. Without a
  // mosaic (an area not stitched yet) only the overlay is drawn, over `extent`.
  // Pointer-first: two-finger scroll pans, pinch (ctrl+wheel) zooms, a mouse
  // wheel zooms, dragging empty space pans, WebKit trackpad rotation rotates,
  // a click without movement goes to the active tool.
  import { onMount, untrack, type Snippet } from 'svelte';
  import type { Core, RawImage } from '../core/core';
  import type { MosaicJson } from '../core/types';
  import { TileCache } from './tiles';
  import { type View, fit, matrix, panBy, rotateAt, toMosaic, zoomAt } from './view';

  export interface CanvasPoint {
    /** mosaic pixels */
    x: number;
    y: number;
    sx: number;
    sy: number;
    ev: PointerEvent | MouseEvent;
  }

  let {
    core,
    areaId,
    mosaic,
    extent,
    lut,
    version = 0,
    preview = null,
    onlevel,
    view = $bindable(),
    cursor = 'default',
    overlay,
    screenOverlay,
    onpress,
    ondrag,
    onrelease,
    onclick,
    ondblclick,
    onhover,
  }: {
    core: Core;
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
    view?: View;
    cursor?: string;
    overlay?: Snippet<[View]>;
    screenOverlay?: Snippet<[View]>;
    onpress?: (p: CanvasPoint) => boolean;
    ondrag?: (p: CanvasPoint) => void;
    onrelease?: (p: CanvasPoint) => void;
    onclick?: (p: CanvasPoint) => void;
    ondblclick?: (p: CanvasPoint) => void;
    onhover?: (p: CanvasPoint | null) => void;
  } = $props();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let cache: TileCache | null = null;
  let size = $state({ w: 800, h: 600 });
  let raf = 0;
  let spaceDown = false;
  let gesture: { scale: number; rot: number } | null = null;

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
    if (previewBmp && preview) {
      const f = 1 << preview.level;
      ctx.imageSmoothingEnabled = true;
      ctx.drawImage(previewBmp, 0, 0, previewBmp.width * f, previewBmp.height * f);
      ctx.imageSmoothingEnabled = vv.zoom * dpr < 1;
      if (preview.over === version) return;
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
    for (let r = Math.floor(y0 / span); r <= Math.min(info.rows - 1, Math.floor(y1 / span)); r++) {
      for (let c = Math.floor(x0 / span); c <= Math.min(info.cols - 1, Math.floor(x1 / span)); c++) {
        const b = cache.get(areaId, L, c, r);
        if (b) {
          ctx.drawImage(b, c * span, r * span, b.width * (1 << L) + eps, b.height * (1 << L) + eps);
        } else {
          // Fallback: the nearest coarser level already loaded.
          for (let k = L + 1; k <= maxL; k++) {
            const f = 1 << (k - L);
            const pc = Math.floor(c / f), pr = Math.floor(r / f);
            const pb = cache.get(areaId, k, pc, pr);
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
    const key = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !isTyping(e)) spaceDown = e.type === 'keydown';
    };
    window.addEventListener('keydown', key);
    window.addEventListener('keyup', key);
    // WebKit trackpad gestures (Safari, Tauri on macOS): pinch and rotate.
    const gs = (e: Event) => {
      e.preventDefault();
      gesture = { scale: 1, rot: 0 };
    };
    const gc = (e: Event) => {
      e.preventDefault();
      const g = e as unknown as { scale: number; rotation: number; clientX: number; clientY: number };
      if (!gesture) return;
      const r = host.getBoundingClientRect();
      const sx = (g.clientX ?? r.left + size.w / 2) - r.left, sy = (g.clientY ?? r.top + size.h / 2) - r.top;
      let nv = zoomAt(v, sx, sy, g.scale / gesture.scale);
      nv = rotateAt(nv, sx, sy, ((g.rotation - gesture.rot) * Math.PI) / 180);
      gesture = { scale: g.scale, rot: g.rotation };
      setView(nv);
    };
    const ge = (e: Event) => {
      e.preventDefault();
      gesture = null;
    };
    host.addEventListener('gesturestart', gs);
    host.addEventListener('gesturechange', gc);
    host.addEventListener('gestureend', ge);
    return () => {
      ro.disconnect();
      window.removeEventListener('keydown', key);
      window.removeEventListener('keyup', key);
      cancelAnimationFrame(raf);
      cache?.clear();
      previewBmp?.close();
    };
  });

  function isTyping(e: Event) {
    const t = e.target as HTMLElement | null;
    return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable);
  }

  function pt(e: PointerEvent | MouseEvent): CanvasPoint {
    const r = host.getBoundingClientRect();
    const sx = e.clientX - r.left, sy = e.clientY - r.top;
    const [x, y] = toMosaic(v, sx, sy);
    return { x, y, sx, sy, ev: e };
  }

  // ---- pointer handling ---------------------------------------------------
  let mode: 'none' | 'tool' | 'maybe' | 'pan' = 'none';
  let start = { x: 0, y: 0 };
  let last = { x: 0, y: 0 };
  const touches = new Map<number, { x: number; y: number }>();
  let pinch: { d: number; a: number } | null = null;

  function down(e: PointerEvent) {
    if ((e.target as Element).closest('[data-ui]')) return;
    host.setPointerCapture(e.pointerId);
    touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (touches.size === 2) {
      const [a, b] = [...touches.values()];
      pinch = { d: Math.hypot(b.x - a.x, b.y - a.y), a: Math.atan2(b.y - a.y, b.x - a.x) };
      mode = 'none';
      return;
    }
    start = last = { x: e.clientX, y: e.clientY };
    if (e.button === 1 || spaceDown) {
      mode = 'pan';
      return;
    }
    if (e.button !== 0) return;
    mode = onpress?.(pt(e)) ? 'tool' : 'maybe';
  }

  function move(e: PointerEvent) {
    if (touches.has(e.pointerId)) touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinch && touches.size === 2) {
      const [a, b] = [...touches.values()];
      const d = Math.hypot(b.x - a.x, b.y - a.y), ang = Math.atan2(b.y - a.y, b.x - a.x);
      const r = host.getBoundingClientRect();
      const mx = (a.x + b.x) / 2 - r.left, my = (a.y + b.y) / 2 - r.top;
      let nv = zoomAt(v, mx, my, d / pinch.d);
      nv = rotateAt(nv, mx, my, ang - pinch.a);
      pinch = { d, a: ang };
      setView(nv);
      return;
    }
    if (mode === 'tool') {
      ondrag?.(pt(e));
    } else if (mode === 'maybe') {
      if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) mode = 'pan';
    }
    if (mode === 'pan') {
      setView(panBy(v, e.clientX - last.x, e.clientY - last.y));
    }
    last = { x: e.clientX, y: e.clientY };
    if (mode === 'none') onhover?.(pt(e));
  }

  function up(e: PointerEvent) {
    touches.delete(e.pointerId);
    if (touches.size < 2) pinch = null;
    if (mode === 'tool') onrelease?.(pt(e));
    else if (mode === 'maybe') onclick?.(pt(e));
    mode = 'none';
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    const r = host.getBoundingClientRect();
    const sx = e.clientX - r.left, sy = e.clientY - r.top;
    if (e.ctrlKey || e.metaKey) {
      // Trackpad pinch (browsers report it as ctrl+wheel) or ctrl/cmd + mouse wheel.
      if (gesture) return;
      setView(zoomAt(v, sx, sy, Math.exp(-e.deltaY * 0.01)));
      return;
    }
    if (e.altKey) {
      setView(rotateAt(v, sx, sy, e.deltaY * 0.002));
      return;
    }
    const lineMode = e.deltaMode === 1;
    const mouseLike = lineMode || (e.deltaX === 0 && Math.abs(e.deltaY) >= 40 && Number.isInteger(e.deltaY) && Math.abs(e.deltaY) % 10 === 0 && !e.shiftKey);
    if (mouseLike) {
      // Mouse wheel: zoom about the cursor.
      const dy = lineMode ? e.deltaY * 33 : e.deltaY;
      setView(zoomAt(v, sx, sy, Math.exp(-dy * 0.0015)));
    } else {
      // Two-finger scroll: pan.
      setView(panBy(v, -e.deltaX, -e.deltaY));
    }
  }

  function dbl(e: MouseEvent) {
    if ((e.target as Element).closest('[data-ui]')) return;
    ondblclick?.(pt(e));
  }

  // Public helpers for toolbars.
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
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onpointerleave={() => onhover?.(null)}
  onwheel={wheel}
  ondblclick={dbl}
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
    user-select: none;
    -webkit-user-select: none;
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
</style>
