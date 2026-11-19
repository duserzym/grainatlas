<script lang="ts">
  import ArrangePage from './ArrangePage.svelte';
  import PanZoom from '../lib/canvas/PanZoom.svelte';
  import { CLICK_SLOP, type ViewPoint } from '../lib/canvas/gestures';
  import type { Grain, SheetBand, SheetPlan, SheetRect, SheetStrip, StripEdit, StripKey } from '../lib/core/types';
  import { bandImageKey, bandRect, bodyOrigin, cropRect, grainChanges, keyId, sameKey, snapGap, snapOrigin, stripBlock, targetGrains } from '../lib/strip-edit';
  let { plan, urls, page = $bindable(0), selected, select, edit, images, grains, bounds, split, axis, cancelSplit, arrange = false, pages = [], selections = [], selectMany, snap = true, setSnap = () => {}, undo = () => {}, redo = () => {}, canUndo = false, canRedo = false, automatic = () => {} }: {
    arrange?:boolean;pages?:string[];selections?:StripKey[];selectMany?:(keys:StripKey[])=>void;snap?:boolean;setSnap?:(v:boolean)=>void;
    undo?:()=>void;redo?:()=>void;canUndo?:boolean;canRedo?:boolean;automatic?:()=>void;
    plan: SheetPlan; urls: string[]; page?: number; selected: StripKey | null; select: (key: StripKey | null) => void; edit: (e: StripEdit, destination?: number) => void;
    images: Record<string, string>; grains: Record<string, Grain[]>; bounds: Record<string, SheetRect>; split: boolean; axis: 'x' | 'y'; cancelSplit: () => void;
  } = $props();
  const PX = 6;
  let panzoom = $state<ReturnType<typeof PanZoom>>();
  type Drag = { strip: SheetStrip; band?: SheetBand; edge?: number; start: [number, number]; screen: [number, number]; origin: [number, number]; rect?: SheetRect; splitAt?: number | null; moved: boolean };
  let drag = $state<Drag | null>(null);
  const strips = $derived(plan.strips.filter(s => !s.hidden && s.sheet === page));
  let arrangeView = $state<ReturnType<typeof ArrangePage>>();
  export function cancel() { drag = null; arrangeView?.cancel(); }
  function press(p: ViewPoint) {
    const el = (p.ev.target as Element).closest('[data-strip]');
    const s = strips.find(s => keyId(s.key) === el?.getAttribute('data-strip'));
    if (!s) return false;
    select(s.key);
    const band = s.bands.find(b => keyId(b.key) === (p.ev.target as Element).closest('[data-band]')?.getAttribute('data-band'));
    const edge = (p.ev.target as Element).getAttribute('data-edge');
    drag = { strip: s, band, edge: edge === null ? undefined : Number(edge), start: [p.x / PX, p.y / PX], screen: [p.ev.clientX, p.ev.clientY], origin: [...s.origin_mm], rect: band ? [...band.mosaic_rect_um] : undefined, moved: false };
    if (split && band) update(p);
    return true;
  }
  function update(p: ViewPoint) {
    if (!drag) return;
    const d = drag, k = plan.um_per_mm;
    const dx = p.x / PX - d.start[0], dy = p.y / PX - d.start[1];
    d.moved ||= Math.hypot(p.ev.clientX - d.screen[0], p.ev.clientY - d.screen[1]) > CLICK_SLOP;
    if (split && d.band) {
      const b = d.band, a = axis === 'x' ? 0 : 1;
      const r = bandRect(d.strip, b, k);
      const value = b.mosaic_rect_um[a] + ((a === 0 ? p.x : p.y) / PX - r[a]) * k;
      d.splitAt = snapGap(value, a, a === 0 ? [b.mosaic_rect_um[1], b.mosaic_rect_um[3]] : [b.mosaic_rect_um[0], b.mosaic_rect_um[2]], targetGrains(plan, d.strip.area, grains), b.mosaic_rect_um[a] + 0.001, b.mosaic_rect_um[a + 2] - 0.001);
    } else if (d.band && d.edge !== undefined) {
      const value = d.band.mosaic_rect_um[d.edge] + (d.edge % 2 === 0 ? dx : dy) * k;
      d.rect = cropRect(d.band.mosaic_rect_um, d.edge, value, bounds[d.strip.area] ?? d.band.mosaic_rect_um, targetGrains(plan, d.strip.area, grains));
    } else {
      const origin: [number, number] = [d.strip.origin_mm[0] + dx, d.strip.origin_mm[1] + dy];
      const inside = bodyOrigin(plan, d.strip, origin);
      d.origin = bodyOrigin(plan, d.strip, p.ev.altKey ? inside : snapOrigin(plan, d.strip, page, inside));
    }
  }
  function release(p: ViewPoint) {
    if (!drag) return;
    if (p.ev.type === 'pointercancel') { drag = null; return; }
    update(p);
    const d = drag;
    drag = null;
    if (split && d.band) {
      if (d.splitAt != null) edit({ op: 'split', band: d.band.key, axis, at_um: d.splitAt });
      cancelSplit();
    } else if (d.band && d.edge !== undefined && d.rect && d.moved) {
      edit({ op: 'crop', band: d.band.key, mosaic_rect_um: d.rect });
    } else if (d.moved) {
      const thumb = document.elementFromPoint(p.ev.clientX, p.ev.clientY)?.closest('[data-sheet]');
      if (thumb) {
        const sheet = Number(thumb.getAttribute('data-sheet'));
        edit({ op: 'move', strip: d.strip.key, sheet, origin_mm: null }, sheet);
      } else edit({ op: 'move', strip: d.strip.key, sheet: page, origin_mm: d.origin });
    }
  }
  function visibleBand(s: SheetStrip, b: SheetBand): SheetRect {
    const r = bandRect(s, b, plan.um_per_mm);
    if (drag?.band && sameKey(drag.band.key, b.key) && drag.edge !== undefined && drag.rect) {
      return [r[0] + (drag.rect[0] - b.mosaic_rect_um[0]) / plan.um_per_mm, r[1] + (drag.rect[1] - b.mosaic_rect_um[1]) / plan.um_per_mm,
        r[0] + (drag.rect[2] - b.mosaic_rect_um[0]) / plan.um_per_mm, r[1] + (drag.rect[3] - b.mosaic_rect_um[1]) / plan.um_per_mm];
    }
    return r;
  }
  function rectStyle(r: SheetRect) { return `left:${r[0] * PX}px;top:${r[1] * PX}px;width:${(r[2] - r[0]) * PX}px;height:${(r[3] - r[1]) * PX}px`; }
</script>

{#if arrange && selectMany}
  <ArrangePage bind:this={arrangeView} {plan} {pages} {urls} bind:page {selections} select={selectMany} {edit} {snap} {setSnap} {undo} {redo} {canUndo} {canRedo} {automatic}/>
{:else}
<div class="editor-view">
  <nav class="thumbs" aria-label="Print pages" data-ui>
    {#each urls as url, i}<button data-testid="page-thumb" data-sheet={i} aria-label="Page {i + 1}" aria-current={page === i ? 'page' : undefined} onclick={() => page = i}><img src={url} alt="" /><span>{i + 1}</span></button>{/each}
  </nav>
  <div class="paper-view">
    <PanZoom bind:this={panzoom} w={plan.paper_mm[0] * PX} h={plan.paper_mm[1] * PX} fitKey={String(page)} label="Strip editor" gestures={{ onpress: press, ondrag: update, onrelease: release, oncancel: cancel, canNudge: () => !selected, ondblclick: p => !!(p.ev.target as Element).closest('[data-strip]') }}>
      <img class="page" src={urls[page]} alt="Page {page + 1}" width={plan.paper_mm[0] * PX} height={plan.paper_mm[1] * PX} data-testid="page" />
      {#if drag?.moved && !split}
        {@const old = stripBlock(drag.strip)}
        <div class="erase" style={rectStyle(old)}></div>
        {#each drag.strip.bands as band}
          {@const r = bandRect({ ...drag.strip, origin_mm: drag.origin }, band, plan.um_per_mm)}
          {@const crop = drag.band && sameKey(drag.band.key, band.key) && drag.rect ? drag.rect : band.mosaic_rect_um}
          {@const cr: SheetRect = [r[0] + (crop[0] - band.mosaic_rect_um[0]) / plan.um_per_mm, r[1] + (crop[1] - band.mosaic_rect_um[1]) / plan.um_per_mm, r[0] + (crop[2] - band.mosaic_rect_um[0]) / plan.um_per_mm, r[1] + (crop[3] - band.mosaic_rect_um[1]) / plan.um_per_mm]}
          <div class="band-preview" style={rectStyle(cr)}>
            <img src={images[bandImageKey(band)]} alt="" style={`position:absolute;left:${(r[0] - cr[0]) * PX}px;top:${(r[1] - cr[1]) * PX}px;width:${(r[2] - r[0]) * PX}px;height:${(r[3] - r[1]) * PX}px`} />
          </div>
        {/each}
      {/if}
      <svg class="overlay" width={plan.paper_mm[0] * PX} height={plan.paper_mm[1] * PX} viewBox={`0 0 ${plan.paper_mm.join(' ')}`} aria-label="Strip boxes">
        <rect class="body" x={plan.sheet_bodies[page]?.body_rect_mm[0]} y={plan.sheet_bodies[page]?.body_rect_mm[1]} width={(plan.sheet_bodies[page]?.body_rect_mm[2] ?? 0) - (plan.sheet_bodies[page]?.body_rect_mm[0] ?? 0)} height={(plan.sheet_bodies[page]?.body_rect_mm[3] ?? 0) - (plan.sheet_bodies[page]?.body_rect_mm[1] ?? 0)} />
        {#each strips as s (keyId(s.key))}
          {@const moving = drag?.moved && sameKey(drag.strip.key, s.key) && drag.edge === undefined && !split}
          {@const r = stripBlock(s, moving ? drag!.origin : s.origin_mm)}
          {#if drag?.moved && sameKey(drag.strip.key, s.key) && !split}<text class="moving-label" x={drag!.origin[0]} y={drag!.origin[1] + 3.1}>{s.sample} · {s.label}</text>{/if}
          <g data-strip={keyId(s.key)}>
            <rect data-testid="strip-block" class="block" class:selected={selected && sameKey(selected, s.key)} x={r[0]} y={r[1]} width={r[2] - r[0]} height={r[3] - r[1]} />
            {#if selected && sameKey(selected, s.key)}
              {#each s.bands as b (keyId(b.key))}
                {@const br = visibleBand(s, b)}
                <g data-band={keyId(b.key)}>
                  <rect class="band" data-testid="strip-band" x={br[0]} y={br[1]} width={br[2] - br[0]} height={br[3] - br[1]} />
                  {#if !split && !moving}
                    {#each [0, 1, 2, 3] as edge}
                      <rect class="handle" data-testid={`crop-edge-${edge}`} data-edge={edge} x={(edge % 2 === 0 ? br[edge] : (br[0] + br[2]) / 2) - 1.5} y={(edge % 2 === 1 ? br[edge] : (br[1] + br[3]) / 2) - 1.5} width="3" height="3" />
                    {/each}
                  {/if}
                </g>
              {/each}
            {/if}
          </g>
        {/each}
        {#if drag?.band && drag.rect && drag.edge !== undefined}
          {@const br = bandRect(drag.strip, drag.band, plan.um_per_mm)}
          {#each grainChanges(drag.band.mosaic_rect_um, drag.rect, targetGrains(plan, drag.strip.area, grains)).leaving as grain}
            <rect data-testid="grain-leaving" class="leaving" x={br[0] + (grain.bbox_um[0] - drag.band.mosaic_rect_um[0]) / plan.um_per_mm} y={br[1] + (grain.bbox_um[1] - drag.band.mosaic_rect_um[1]) / plan.um_per_mm} width={(grain.bbox_um[2] - grain.bbox_um[0]) / plan.um_per_mm} height={(grain.bbox_um[3] - grain.bbox_um[1]) / plan.um_per_mm} />
          {/each}
        {/if}
        {#if split && drag?.band && drag.splitAt != null}
          {@const br = bandRect(drag.strip, drag.band, plan.um_per_mm)}
          {@const at = (drag.splitAt - drag.band.mosaic_rect_um[axis === 'x' ? 0 : 1]) / plan.um_per_mm}
          <line class="split" x1={axis === 'x' ? br[0] + at : br[0]} x2={axis === 'x' ? br[0] + at : br[2]} y1={axis === 'y' ? br[1] + at : br[1]} y2={axis === 'y' ? br[1] + at : br[3]} />
        {/if}
      </svg>
    </PanZoom>
    <div class="tools glass" data-ui><button onclick={() => panzoom?.zoomBy(1 / 1.5)} aria-label="Zoom out">−</button><button onclick={() => panzoom?.zoomBy(1.5)} aria-label="Zoom in">+</button><button onclick={() => panzoom?.fitAll()}>Fit</button></div>
  </div>
</div>
{/if}
<style>
  .editor-view { position: absolute; inset: 0; display: grid; grid-template-columns: 5rem minmax(0, 1fr); gap: var(--space-2); }
  .thumbs { overflow: auto; display: flex; flex-direction: column; gap: var(--space-2); min-height: 0; margin-top: var(--notice-bottom, 0px); padding: var(--space-1); }
  .thumbs button { flex: none; padding: var(--space-1); }
  .thumbs img { width: 100%; display: block; background: white; }
  .thumbs [aria-current] { outline: 2px solid var(--accent); }
  .paper-view { position: relative; min-width: 0; min-height: 0; overflow: hidden; }
  .page { display: block; background: white; }
  .overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }
  .body { fill: none; stroke: var(--text-faint); stroke-width: 0.2; stroke-dasharray: 2 2; }
  .block { fill: transparent; stroke: var(--accent); stroke-width: 0.25; pointer-events: all; cursor: move; }
  .block.selected { stroke-width: 0.8; }
  .band { fill: transparent; pointer-events: all; cursor: move; }
  .handle { fill: white; stroke: var(--accent); stroke-width: 0.4; pointer-events: all; cursor: crosshair; }
  .leaving { fill: none; stroke: var(--danger); stroke-width: 0.8; }
  .moving-label { fill: #000; font-size: 2.5px; }
  .split { stroke: var(--accent); stroke-width: 0.6; }
  .erase { position: absolute; background: white; pointer-events: none; }
  .band-preview { position: absolute; overflow: hidden; background: var(--hairline); pointer-events: none; }
  .tools { position: absolute; top: var(--space-2); right: var(--space-2); display: flex; padding: var(--space-1); gap: var(--space-1); }
</style>
