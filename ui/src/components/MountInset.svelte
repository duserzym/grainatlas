<script lang="ts">
  // Mount locator for the read-only viewer: every area of the mount at its stage position,
  // the shown area highlighted, Review's sample regions or selected spots, and the visible image.
  // Clicking an area shows it, like the Area menu. Size and collapsed state live in memory and
  // go to UPb Lab (bridge v5.1), which remembers them; the viewer writes nothing.
  import { onDestroy } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import type { Core } from '../lib/core/core';
  import type { AreaRecord, MosaicJson, Spot } from '../lib/core/types';
  import type { View } from '../lib/canvas/view';
  import { presentation } from '../lib/presentation';
  import { sendViewerInset } from '../lib/viewer-inset';
  import { sampleNames, sampleText, shortMountName } from '../lib/mount-inset-header';
  import {
    applyFit, areaExtent, defaultInset, ENLARGED_W, fitTransform, mapAspect, maxWidth, MIN_W, mosaicToMount, mountLayout,
    NORMAL_W, shownWidth, STEP, storedWidth, union, viewQuad, type Box, type GroupRegion, type Pt,
  } from '../lib/mount-inset';

  interface Props {
    areas: AreaRecord[];
    current: string;
    mosaics: Record<string, MosaicJson>;
    tiles: Pick<Core, 'pyramidTile'>;
    lut: Uint8Array;
    view: View | undefined;
    selected: Spot[];
    groups?: GroupRegion[];
    mountLabel?: string;
    onmount?: () => void;
    onsample?: (group: GroupRegion, area: string) => void;
    onarea: (id: string) => void;
  }
  let { areas, current, mosaics, tiles, lut, view, selected, groups = [], mountLabel = 'Mount', onmount, onsample, onarea }: Props = $props();
  const isViewer = $derived(presentation.current === 'viewer');

  // ---- geometry -------------------------------------------------------------------------
  const extents = $derived(Object.fromEntries(areas.map(a => [a.id, areaExtent(a, mosaics[a.id])])) as Record<string, Pt>);
  const layout = $derived(mountLayout(areas, extents));
  // Fit padded regions too, so groups on the edge of an area keep their full outline.
  const bounds = $derived(union([layout.bounds, ...groups.flatMap(g => g.regions.map(r => union(r.hull.map(([x, y]): Box => [x, y, x, y]))))]));
  const aspect = $derived(mapAspect(bounds));
  const currentArea = $derived(areas.find(a => a.id === current) ?? null);
  const currentLabel = $derived(layout.areas.find(a => a.id === current)?.label ?? '');
  const names = $derived(sampleNames(groups, areas));

  // ---- size and room --------------------------------------------------------------------
  const HEAD = 26, NOTE = 16, PAD = 8;
  let root = $state<HTMLDivElement>();
  let nameNode = $state<HTMLSpanElement>();
  let headerMetrics = $state<{ width: number; measure: (text: string) => number } | null>(null);
  let room = $state({ w: 0, h: 0, top: PAD, bottom: 0 });
  let inset = $state<{ collapsed: boolean; width: number } | null>(null);
  let animate = $state(0);

  $effect(() => {
    const received = presentation.inset;
    if (received) inset = { collapsed: received.collapsed, width: received.width };
  });
  $effect(() => {
    if (!inset && room.w > 0) inset = presentation.inset ?? defaultInset(room.w, room.h);
  });

  const chrome = $derived(HEAD + (layout.positioned ? 0 : NOTE) + 2);
  const limit = $derived(maxWidth(room.w, room.bottom - room.top, aspect, chrome));
  const width = $derived(shownWidth(inset?.width ?? NORMAL_W, limit));
  const headerLabel = $derived(sampleText(names, currentLabel, headerMetrics?.width ?? 0, headerMetrics?.measure));
  const mountTitle = $derived(!headerLabel.text && headerLabel.title ? `${mountLabel}: ${headerLabel.title}` : mountLabel);
  const mapH = $derived(Math.round(width * aspect));
  /** Too little room for a useful map: only the Mount button, disabled. */
  const fits = $derived(width >= 60 && mapH >= 24);
  const collapsed = $derived(!inset || inset.collapsed || !fits);
  /** At the enlarged size (or the most the room allows): the size button makes it normal again. */
  const enlarged = $derived(width >= Math.min(ENLARGED_W, Math.floor(limit)));

  // Measure complete captions in the space left after the mount name, at the rendered font.
  $effect(() => {
    const node = nameNode;
    if (!node) { headerMetrics = null; return; }
    const ctx = document.createElement('canvas').getContext('2d');
    const measureText = () => {
      if (ctx) ctx.font = getComputedStyle(node).font;
      headerMetrics = { width: node.clientWidth, measure: text => ctx?.measureText(text).width ?? Array.from(text).length * 6 };
    };
    const observer = new ResizeObserver(measureText);
    observer.observe(node); measureText();
    return () => observer.disconnect();
  });

  function measure() {
    const host = root?.parentElement;
    if (!host) return;
    const r = host.getBoundingClientRect();
    const controls = host.querySelector('.viewer-controls')?.getBoundingClientRect();
    const below = ['.legend', '.status'].map(s => host.querySelector(s)?.getBoundingClientRect()).filter((b): b is DOMRect => !!b && b.height > 0);
    room = {
      w: r.width, h: r.height,
      top: (controls ? controls.bottom - r.top : 0) + PAD,
      bottom: Math.min(r.height, ...below.map(b => b.top - r.top)) - PAD,
    };
  }
  let resize: ResizeObserver | null = null, changes: MutationObserver | null = null;
  function observe() {
    const host = root?.parentElement;
    if (!host || !resize || !changes) return;
    resize.disconnect();
    resize.observe(host);
    for (const el of host.querySelectorAll<HTMLElement>('.viewer-controls, .legend, .status')) resize.observe(el);
    measure();
  }
  $effect(() => {
    if (!root?.parentElement) return;
    resize = new ResizeObserver(measure);
    changes = new MutationObserver(observe);
    changes.observe(root.parentElement, { childList: true });
    observe();
    return () => { resize?.disconnect(); changes?.disconnect(); resize = changes = null; };
  });

  /** The user changed the inset: keep it and tell the shell (never for a received inset). */
  function userSet(next: { collapsed: boolean; width: number }) {
    inset = next;
    sendViewerInset({ collapsed: next.collapsed, width: storedWidth(next.width) });
  }
  function motionMs(e: MouseEvent | KeyboardEvent): number {
    if (!(e instanceof MouseEvent) || e.detail === 0 || !root) return 0; // keyboard: no motion
    const v = getComputedStyle(root).getPropertyValue('--motion').trim();
    const ms = v.endsWith('ms') ? parseFloat(v) : v.endsWith('s') ? parseFloat(v) * 1000 : 0;
    return Math.min(200, Number.isFinite(ms) ? ms : 0);
  }
  function toggle(e: MouseEvent) {
    animate = motionMs(e);
    userSet({ collapsed: !collapsed, width: inset?.width ?? NORMAL_W });
  }
  function enlarge() {
    animate = 0;
    userSet({ collapsed: false, width: enlarged ? NORMAL_W : Math.max(NORMAL_W + STEP, Math.min(ENLARGED_W, Math.floor(limit))) });
  }
  function open(_node: Element) {
    return { duration: animate, easing: cubicOut, css: (t: number) => `opacity:${t};transform:scale(${0.92 + 0.08 * t})` };
  }

  // Resize by the bottom-left corner: left and down grow it; the aspect ratio stays the mount's.
  let drag: { x: number; y: number; w: number; id: number } | null = null;
  function gripDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault(); e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, w: width, id: e.pointerId };
    animate = 0;
  }
  function gripMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    const grow = Math.max(drag.x - e.clientX, (e.clientY - drag.y) / aspect);
    inset = { collapsed: false, width: shownWidth(drag.w + grow, limit) };
  }
  function gripUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.id) return;
    drag = null;
    userSet({ collapsed: false, width });
  }
  function gripKey(e: KeyboardEvent) {
    const steps: Record<string, number> = { ArrowUp: STEP, ArrowRight: STEP, ArrowDown: -STEP, ArrowLeft: -STEP };
    let next: number | null = null;
    if (e.key in steps) next = width + steps[e.key];
    else if (e.key === 'Home') next = MIN_W;
    else if (e.key === 'End') next = limit;
    if (next === null) return;
    e.preventDefault(); e.stopPropagation();
    animate = 0;
    next = shownWidth(next, limit);
    if (next !== width) userSet({ collapsed: false, width: next });
  }

  // ---- drawing --------------------------------------------------------------------------
  const fit = $derived(fitTransform(bounds, width, mapH, 6));
  const px = (p: Pt) => applyFit(fit, bounds, p);
  const samples = $derived(groups.map(g => ({
    ...g, anchor: px(g.anchor),
    regions: g.regions.map(r => ({ ...r, points: r.points.map(px), hull: r.hull.map(px) })),
  })));
  const polygon = (ps: Pt[]) => ps.map(p => p.map(v => v.toFixed(1)).join(',')).join(' ');
  const shapes = $derived(layout.areas.map(a => {
    const [x0, y0] = px([a.box[0], a.box[1]]), [x1, y1] = px([a.box[2], a.box[3]]);
    return { ...a, x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0), on: a.id === current, active: isViewer || a.id in mosaics };
  }));
  const currentShape = $derived(shapes.find(s => s.on) ?? null);
  const short = (text: string, w: number) => {
    const n = Math.floor((w - 4) / 5.6);
    return n >= text.length ? text : n >= 3 ? `${text.slice(0, n - 1)}…` : '';
  };
  /** Sample names get first choice after the current-area label; all labels share collision boxes. */
  const labels = $derived.by(() => {
    const hit = (a: Box, b: Box) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    const placed: Box[] = [];
    if (currentShape && !groups.length) {
      const cx = currentShape.x + currentShape.w / 2, cy = currentShape.y >= 14 ? currentShape.y - 9 : currentShape.y + currentShape.h + 6;
      const half = currentLabel.length * 3.3 + 3;
      placed.push([cx - half, cy - 7, cx + half, cy + 7]);
      if (!groups.length) placed.push([currentShape.x, currentShape.y, currentShape.x + currentShape.w, currentShape.y + currentShape.h]);
    }
    const sampleLabels = new Map<number, string>();
    samples.forEach((g, i) => {
      const [cx, cy] = g.anchor;
      const region = g.regions.find(r => r.area === g.anchorArea);
      if (!region || cy < 7 || cy > mapH - 7) return;
      const xs = region.hull.map(p => p[0]), ys = region.hull.map(p => p[1]);
      if (Math.max(...ys) - Math.min(...ys) < 14) return;
      const room = 2 * Math.min(cx - Math.max(2, Math.min(...xs)), Math.min(width - 2, Math.max(...xs)) - cx);
      const text = short(g.name, room), half = text.length * 2.8 + 2;
      const box: Box = [cx - half, cy - 7, cx + half, cy + 7];
      if (!text || placed.some(p => hit(p, box))) return;
      placed.push(box); sampleLabels.set(i, text);
    });
    const out = new Map<string, string>();
    for (const sh of shapes) {
      // Sample regions cover the areas; their names replace the area labels.
      if (sh.on || groups.length || sh.w < 30 || sh.h < 12) continue;
      const text = short(sh.label, sh.w);
      if (!text) continue;
      const cx = sh.x + sh.w / 2, cy = sh.y + sh.h / 2, half = text.length * 2.8 + 2;
      const box: Box = [cx - half, cy - 6, cx + half, cy + 6];
      if (placed.some(p => hit(p, box))) continue;
      placed.push(box); out.set(sh.id, text);
    }
    return { areas: out, samples: sampleLabels };
  });
  const recById = $derived(new Map(areas.map(a => [a.id, a])));
  const dots = $derived(groups.length ? [] : selected.flatMap(s => {
    const rec = recById.get(s.area_id);
    const p = rec ? mosaicToMount(rec, layout, s.x_um, s.y_um) : null;
    return p ? [{ id: s.id, p: px(p) }] : [];
  }));
  const quad = $derived.by(() => {
    const m = mosaics[current];
    if (!view || !m || !currentArea) return null;
    const q = viewQuad(view, m.um_per_px, [m.w, m.h], (x, y) => mosaicToMount(currentArea, layout, x, y));
    return q ? q.map(p => px(p).map(v => v.toFixed(1)).join(',')).join(' ') : null;
  });

  // The coarsest pyramid tile of each shown area as its picture (one small tile per area).
  type Raw = { w: number; h: number; data: Uint8Array };
  const raws = new Map<string, Raw | 'failed'>();
  let rawVersion = $state(0);
  let alive = true;
  onDestroy(() => { alive = false; });
  $effect(() => {
    for (const [id, m] of Object.entries(mosaics)) {
      const L = m.levels.length - 1;
      const info = m.levels[L];
      const key = `${id}/${m.sha256}/${L}`;
      if (!info || info.cols !== 1 || info.rows !== 1 || raws.has(key)) continue;
      raws.set(key, 'failed');
      tiles.pyramidTile(id, L, 0, 0).then(r => {
        if (!alive) return;
        raws.set(key, r);
        rawVersion++;
      }, () => {});
    }
  });
  /** Data URLs of the pictures, made again only when a tile lands or the tone changes. */
  const hrefs = $derived.by(() => {
    rawVersion;
    const out: Record<string, { href: string; w: number; h: number; level: number }> = {};
    if (typeof document === 'undefined') return out;
    for (const [id, m] of Object.entries(mosaics)) {
      const L = m.levels.length - 1;
      const raw = raws.get(`${id}/${m.sha256}/${L}`);
      if (!raw || raw === 'failed') continue;
      const canvas = document.createElement('canvas');
      canvas.width = raw.w; canvas.height = raw.h;
      const ctx = canvas.getContext('2d');
      if (!ctx) continue;
      const img = ctx.createImageData(raw.w, raw.h);
      for (let i = 0, j = 0; i < raw.data.length; i++, j += 4) {
        const v = lut[raw.data[i]];
        img.data[j] = img.data[j + 1] = img.data[j + 2] = v; img.data[j + 3] = 255;
      }
      ctx.putImageData(img, 0, 0);
      out[id] = { href: canvas.toDataURL('image/png'), w: raw.w, h: raw.h, level: L };
    }
    return out;
  });
  const pictures = $derived.by(() => {
    const out: Record<string, { href: string; x: number; y: number; w: number; h: number }> = {};
    for (const a of areas) {
      const m = mosaics[a.id], pic = hrefs[a.id];
      if (!m || !pic) continue;
      const f = 1 << pic.level;
      const mw = m.w * m.um_per_px, mh = m.h * m.um_per_px;
      // The picture's corner on the mount: through the tile nearest the mosaic's centre.
      const c = mosaicToMount(a, layout, mw / 2, mh / 2);
      if (!c) continue;
      const [x0, y0] = px([c[0] - mw / 2, c[1] - mh / 2]);
      out[a.id] = { href: pic.href, x: x0, y: y0, w: pic.w * f * m.um_per_px * fit.k, h: pic.h * f * m.um_per_px * fit.k };
    }
    return out;
  });

  function activate(id: string, e: KeyboardEvent | MouseEvent) {
    if (e instanceof KeyboardEvent && e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault(); e.stopPropagation();
    if (isViewer || (id !== current && id in mosaics)) onarea(id);
  }
  function activateSample(group: GroupRegion, area: string, e: KeyboardEvent | MouseEvent) {
    if (!isViewer || (e instanceof KeyboardEvent && e.key !== 'Enter' && e.key !== ' ')) return;
    e.preventDefault(); e.stopPropagation();
    onsample?.(group, area);
  }
</script>

<div class="glass inset" class:open={!collapsed} data-ui data-testid="mount-inset" bind:this={root}
  style:top="{room.top}px" style:visibility={inset ? 'visible' : 'hidden'}>
  {#if collapsed}
    <button class="opener" onclick={toggle} aria-expanded="false" disabled={!fits} data-testid="mount-inset-open"
      title={fits ? mountLabel : `${mountLabel}: too little room for the mount map`}>Mount · {shortMountName(mountLabel)}</button>
  {:else}
    <div class="body" style:width="{width}px" transition:open data-testid="mount-inset-body">
      <div class="head">
        <button class="title" title={mountTitle} onclick={() => isViewer && onmount?.()} data-testid="mount-inset-label"><span class="mount-prefix">{'Mount: '}</span>{mountLabel}</button>
        <span class="faint name" title={headerLabel.title} bind:this={nameNode}>{headerLabel.text}</span>
        <button class="icon" onclick={enlarge} title={enlarged ? 'Smaller' : 'Larger'} aria-label={enlarged ? 'Smaller mount map' : 'Larger mount map'}
          disabled={limit < NORMAL_W + STEP} data-testid="mount-inset-enlarge">{enlarged ? '−' : '+'}</button>
        <button class="icon" onclick={toggle} aria-expanded="true" aria-label="Hide mount map" title="Hide mount map" data-testid="mount-inset-close">×</button>
      </div>
      <svg width={width} height={mapH} role="group" aria-label="Mount map" aria-describedby={groups.length ? 'mount-inset-groups' : undefined} data-testid="mount-inset-map">
        {#if groups.length}<desc id="mount-inset-groups">Sample groups: {groups.map(g => g.name).join(', ')}</desc>{/if}
        <defs><clipPath id="mount-inset-clip"><rect width={width} height={mapH} /></clipPath></defs>
        <rect class="surface" width={width} height={mapH} rx="4" />
        <g clip-path="url(#mount-inset-clip)">
          {#each shapes as s (s.id)}
            {@const pic = pictures[s.id]}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <g class="area" class:on={s.on} class:active={s.active} data-area={s.id} data-testid="mount-inset-area"
              role={s.active ? 'button' : 'img'} aria-label={s.label} aria-current={s.on ? 'true' : undefined}
              tabindex={s.active ? 0 : undefined}
              onclick={(e) => s.active && activate(s.id, e)} onkeydown={(e) => s.active && activate(s.id, e)}>
              {#if pic}
                <svg x={s.x} y={s.y} width={s.w} height={s.h} overflow="hidden">
                  <image href={pic.href} x={pic.x - s.x} y={pic.y - s.y} width={pic.w} height={pic.h} preserveAspectRatio="none" />
                </svg>
              {/if}
              <rect class="box" x={s.x} y={s.y} width={s.w} height={s.h} />
              {#if labels.areas.has(s.id)}
                <text class="short" x={s.x + s.w / 2} y={s.y + s.h / 2}>{labels.areas.get(s.id)}</text>
              {/if}
            </g>
          {/each}
          {#each samples as g}
            <g class="sample" style:--group-color={g.color} data-testid="mount-inset-group">
              {#each g.regions as r}
                <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                <polygon class="region" class:interactive={isViewer} points={polygon(r.hull)} data-area={r.area}
                  role={isViewer ? 'button' : 'img'} tabindex={isViewer ? 0 : undefined}
                  aria-label={`${g.name}: ${r.points.length} ${r.points.length === 1 ? 'spot' : 'spots'}. Show and list sessions`}
                  onclick={e => activateSample(g, r.area, e)} onkeydown={e => activateSample(g, r.area, e)} />
                {#each r.points as p}<circle class="group-dot" cx={p[0]} cy={p[1]} r="2" />{/each}
              {/each}
            </g>
          {/each}
          {#if samples.length && currentShape}
            <rect class="current-outline" x={currentShape.x} y={currentShape.y} width={currentShape.w} height={currentShape.h} />
          {/if}
          {#if quad}<polygon class="quad" points={quad} data-testid="mount-inset-view" />{/if}
          {#each dots as d (d.id)}<circle class="dot" cx={d.p[0]} cy={d.p[1]} r="3" data-testid="mount-inset-spot" />{/each}
          {#each samples as g, i}
            {#if labels.samples.has(i)}
              <text class="sample-label" x={g.anchor[0]} y={g.anchor[1]}>{labels.samples.get(i)}</text>
            {/if}
          {/each}
          {#if currentShape && !samples.length}
            {@const above = currentShape.y >= 14}
            <text class="here" data-testid="mount-inset-current"
              x={Math.min(width - 4, Math.max(4, currentShape.x + currentShape.w / 2))}
              y={above ? currentShape.y - 4 : Math.min(mapH - 4, currentShape.y + currentShape.h + 11)}
              text-anchor={currentShape.x + currentShape.w / 2 < 40 ? 'start' : currentShape.x + currentShape.w / 2 > width - 40 ? 'end' : 'middle'}>{currentLabel}</text>
          {/if}
        </g>
      </svg>
      {#if !layout.positioned}<div class="note faint" data-testid="mount-inset-note">positions not recorded</div>{/if}
      <div class="grip" role="slider" tabindex="0" aria-label="Mount map size" aria-orientation="horizontal"
        aria-valuemin={Math.min(MIN_W, Math.floor(limit))} aria-valuemax={Math.floor(limit)} aria-valuenow={width} aria-valuetext="{width} px wide"
        title="Drag to resize (arrow keys too)" data-testid="mount-inset-grip"
        onpointerdown={gripDown} onpointermove={gripMove} onpointerup={gripUp} onpointercancel={gripUp} onkeydown={gripKey}></div>
    </div>
  {/if}
</div>

<style>
  .inset { position: absolute; right: var(--space-2); z-index: 20; padding: var(--space-1); border-radius: var(--radius-sm); font-size: 0.72rem; }
  .opener { padding: 2px var(--space-2); font-size: 0.75rem; max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; vertical-align: top; }
  .body { position: relative; display: grid; gap: 2px; transform-origin: top right; }
  .head { display: flex; align-items: center; gap: var(--space-1); height: 24px; min-width: 0; }
  .head .title { font-weight: 600; font-size: inherit; padding: 0; border-color: transparent; background: transparent; text-align: left; flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mount-prefix { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .name { flex: 1 1 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .head .icon { flex: 0 0 18px; min-width: 18px; padding: 0; line-height: 20px; }
  svg { display: block; }
  .surface { fill: var(--image-bg); }
  .area .box { fill: transparent; stroke: var(--ov-label); stroke-opacity: 0.45; stroke-width: 1; }
  .area:not(.on) image { opacity: 0.5; }
  .area.active { cursor: pointer; }
  .area.active:hover .box { stroke-opacity: 0.9; }
  .area:focus { outline: none; }
  .area:focus-visible .box { stroke: var(--accent); stroke-opacity: 1; stroke-width: 2; stroke-dasharray: 3 2; }
  .area.on .box { stroke: var(--accent); stroke-opacity: 1; stroke-width: 2; }
  .short { fill: var(--ov-label); fill-opacity: 0.75; font-size: 9px; text-anchor: middle; dominant-baseline: central; pointer-events: none; }
  .here { fill: var(--ov-label); font-size: 10px; font-weight: 600; paint-order: stroke; stroke: var(--image-bg); stroke-width: 3px; pointer-events: none; }
  .sample { pointer-events: none; }
  .region { fill: var(--group-color); fill-opacity: 0.18; stroke: var(--group-color); stroke-width: 1.5; stroke-linejoin: round; }
  .region.interactive { pointer-events: auto; cursor: pointer; }
  .region.interactive:hover { fill-opacity: 0.35; stroke-width: 2; }
  .region.interactive:focus-visible { outline: none; fill-opacity: 0.35; stroke: var(--accent); stroke-width: 3; stroke-dasharray: 3 2; }
  .group-dot { fill: var(--group-color); stroke: var(--image-bg); stroke-width: 0.75; }
  .sample-label { fill: var(--ov-label); font-size: 9px; font-weight: 600; text-anchor: middle; dominant-baseline: central; paint-order: stroke; stroke: var(--image-bg); stroke-width: 3px; pointer-events: none; }
  .current-outline { fill: none; stroke: var(--accent); stroke-width: 2; pointer-events: none; }
  .quad { fill: none; stroke: var(--ov-number); stroke-width: 1.25; pointer-events: none; }
  .dot { fill: var(--ov-spot-selected); stroke: var(--image-bg); stroke-width: 1; pointer-events: none; }
  .note { font-size: 0.65rem; line-height: 14px; }
  .grip { position: absolute; left: -6px; bottom: -6px; width: 16px; height: 16px; cursor: nesw-resize; touch-action: none; border-radius: 4px; }
  .grip::before { content: ''; position: absolute; left: 4px; bottom: 4px; width: 7px; height: 7px; border-left: 2px solid var(--text-muted); border-bottom: 2px solid var(--text-muted); }
  .grip:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
