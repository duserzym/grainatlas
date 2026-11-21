<script lang="ts">
  import { areaImageEdits, printViewGrains, imageTransform, applyAffine, pointBounds } from '../lib/image-edits';
  import { onDestroy, onMount, type Snippet } from 'svelte';
  import { current } from '../lib/store.svelte';
  import type { PrintEditorState } from '../lib/print-cache';
  import type { Grain, SheetPlan, SheetRect, SheetResult, StripEdit, StripKey, ToneParams } from '../lib/core/types';
  import { chosen, nudge, placed } from '../lib/arrange';
  import { bandImageKey, nudgeOrigin, sameKey, splitPairs, StripHistory } from '../lib/strip-edit';
  import StripControls from './StripControls.svelte';
  import StripPage from './StripPage.svelte';

  const app = current();
  const displayGrains=$derived(Object.fromEntries(Object.entries(app.grains).map(([id,gs])=>{
    const a=app.project?.areas.find(a=>a.id===id),m=app.stitched[id]?.mosaic??a?.mosaic;
    return [id,m?printViewGrains(gs,areaImageEdits(app.project,id),[m.w*m.um_per_px,m.h*m.um_per_px]):gs];
  })));
  let { base, result, urls, controls, droppedNotice, history, editorState = $bindable(), storedDropped = 0, page = $bindable(0), enabled, onresult, onpending, onchange, onautomatic = () => {} }: {
    onautomatic?:()=>void; editorState: PrintEditorState;
    base: SheetPlan; result: SheetResult; urls: string[]; controls: HTMLElement; history: StripHistory; page?: number; enabled: boolean;
    droppedNotice: Snippet<[boolean]>;
    storedDropped?: number; onresult: (r: SheetResult) => void; onpending: (v: boolean) => void; onchange: () => void;
  } = $props();
  let revision = $state(0);
  let images = $state<Record<string, string>>({});
  let view = $state<ReturnType<typeof StripPage>>();
  let stripControls = $state<ReturnType<typeof StripControls>>();
  export function confirmAutomatic() { stripControls?.confirmAutomatic(); }
  let disposed = false;
  let running = false;
  let requested = 0;
  let nextPage: number | null = null;
  const cache = new Map<string, Promise<string>>();
  const selected = $derived(result.plan.strips.find(s => editorState.selection && sameKey(s.key, editorState.selection)));
  const count = $derived.by(() => { void revision; return history.count + storedDropped; });
  const canUndo = $derived.by(() => { void revision; return history.past.length > 0; });
  const canRedo = $derived.by(() => { void revision; return history.future.length > 0; });
  const pairs = $derived.by(() => { void revision; return splitPairs(base, history.edits); });
  const bounds = $derived.by(() => {
    const rects: Record<string, SheetRect> = {};
    for (const [area, summary] of Object.entries(app.stitched)) {
      const m = summary.mosaic;
      const size:[number,number]=[m.w*m.um_per_px,m.h*m.um_per_px];
      const image=imageTransform(areaImageEdits(app.project,area),size);
      const b=pointBounds([[0,0],[size[0],0],size,[0,size[1]]].map(p=>applyAffine(image,p as [number,number])));
      rects[area] = [0, 0, Math.ceil((b[2]-b[0])/m.um_per_px-1e-8)*m.um_per_px, Math.ceil((b[3]-b[1])/m.um_per_px-1e-8)*m.um_per_px];
    }
    return rects;
  });
  $effect(() => {
    if (!enabled) { editorState.splitting = false; view?.cancel(); }
  });
  $effect(() => {
    if (selected && selected.sheet !== page) { editorState.selection = null; editorState.splitting = false; }
  });
  $effect(() => {
    if (!running) page = Math.min(page, result.pages.length - 1);
  });
  $effect(() => {
    if (!enabled || editorState.arrange) return;
    const bands = result.plan.strips.filter(s => !s.hidden && s.sheet === page).flatMap(s => s.bands);
    const tones = $state.snapshot(app.tone) as Record<string, ToneParams>;
    for (const band of bands) {
      const key = bandImageKey(band);
      if (cache.has(key)) continue;
      const rect = $state.snapshot(band.mosaic_rect_um) as SheetRect;
      const promise = app.core.then(core => core.printBandImage(band.key.area, rect, tones)).then(bytes => {
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
        if (disposed) URL.revokeObjectURL(url); else images = { ...images, [key]: url };
        return url;
      });
      cache.set(key, promise);
      promise.catch(e => { cache.delete(key); if (!disposed) app.error = String(e); });
    }
  });
  onDestroy(() => { disposed = true; onpending(false); for (const url of Object.values(images)) URL.revokeObjectURL(url); });

  function selectMany(keys:StripKey[]) { editorState.selections=keys;editorState.selection=keys[0]??null; }
  $effect(() => {
    if (enabled && editorState.arrange && !result.plan.strips.some(s=>s.transform) && !history.edits.some(e=>e.op==='arrange')) { edit({op:'arrange'}); history.past=[]; }
  });
  function select(key: StripKey | null) {
    editorState.selection = key;
    const s = result.plan.strips.find(s => key && sameKey(s.key, key));
    if (s) page = s.sheet;
  }
  function edit(e: StripEdit, destination?: number) { if (app.readOnly) return; history.push($state.snapshot(e) as StripEdit); nextPage = destination ?? null; changed(); }
  function undo() { if (app.readOnly) return; nextPage = null; history.undo(); changed(); }
  function redo() { if (app.readOnly) return; nextPage = null; history.redo(); changed(); }
  function automatic() { if (app.readOnly) return; history.automatic(); onautomatic(); editorState.selections=[]; editorState.selection = null; editorState.splitting = false; changed(); }
  function changed() { onchange(); revision++; requested++; onpending(true); void render(); }
  /** Serialise worker calls, coalesce new input and never replace the page with a stale result. */
  async function render() {
    if (running) return;
    running = true;
    try {
      const core = await app.core;
      while (!disposed) {
        const version = requested;
        const rendered = history.edits;
        const edits = $state.snapshot(rendered) as StripEdit[];
        const r = await app.run('Rendering strips', () => core.printPlanRender($state.snapshot(base) as SheetPlan, edits, $state.snapshot(app.grains) as Record<string, Grain[]>, $state.snapshot(app.tone) as Record<string, ToneParams>));
        if (disposed) return;
        if (r?.dropped?.length) {
          history.reject(rendered, r.dropped.map(d => d.index));
          // Remove known refusals from Save and recovery, even when this result is stale.
          onchange();
          revision++;
        }
        if (version !== requested) continue;
        if (r) {
          if (r.dropped?.some(d => d.index === rendered.length - 1)) nextPage = null;
          onresult(r);
          if (nextPage !== null) { page = Math.max(0, Math.min(nextPage, r.pages.length - 1)); nextPage = null; }
          onpending(false);
        }
        break;
      }
    } finally { running = false; }
  }
  onMount(() => {
    function key(e: KeyboardEvent) {
      if (!enabled || document.querySelector('dialog[open]')) return;
      const el = e.target as HTMLElement;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); editorState.selections=[]; editorState.selection = null; editorState.splitting = false; view?.cancel(); return; }
      if (el.matches('input, textarea, select') || el.isContentEditable) return;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault(); e.stopImmediatePropagation(); if (e.shiftKey) redo(); else undo(); return;
      }
      const direction: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
      if (editorState.arrange && direction[e.key] && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();e.stopImmediatePropagation();
        let strips=chosen(result.plan,editorState.selections??[]);
        for(const op of history.edits.slice(renderedCount)) if(op.op==='transform') strips=strips.map(s=>{const p=op.pieces.find(p=>sameKey(p.strip,s.key));return p?placed(s,p):s;});
        if(strips.length) edit({op:'transform',pieces:nudge(strips,direction[e.key],e.shiftKey)});
        return;
      }
      if (selected && direction[e.key] && !e.metaKey && !e.ctrlKey) {
        e.preventDefault(); e.stopImmediatePropagation();
        // Include nudges queued before the renderer returns, so key repeat is exact.
        let origin: [number, number] = [...selected.origin_mm];
        for (const edit of history.edits.slice(renderedCount)) if (edit.op === 'move' && sameKey(edit.strip, selected.key) && edit.origin_mm) origin = [...edit.origin_mm];
        edit({ op: 'move', strip: selected.key, sheet: selected.sheet, origin_mm: nudgeOrigin(origin, direction[e.key], e.shiftKey) });
      }
    }
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  });
  let renderedCount = 0;
  onMount(() => { renderedCount = history.count; });
  $effect(() => { void result; renderedCount = history.count; });
</script>

{#if enabled}
  <StripControls bind:this={stripControls} arrange={editorState.arrange} selections={editorState.selections??[]} {selectMany} target={controls} {droppedNotice} {result} {selected} {select} {edit} {undo} {redo} {automatic} {count} {canUndo} {canRedo} split={editorState.splitting} setSplit={v => editorState.splitting = v} axis={editorState.axis} setAxis={v => editorState.axis = v} {pairs} {page} />
  <StripPage bind:this={view} arrange={editorState.arrange} pages={result.pages} selections={editorState.selections??[]} {selectMany} snap={editorState.snap??true} setSnap={v=>editorState.snap=v} {undo} {redo} {canUndo} {canRedo} automatic={confirmAutomatic} plan={result.plan} {urls} bind:page selected={editorState.selection} {select} {edit} {images} grains={displayGrains} {bounds} split={editorState.splitting} axis={editorState.axis} cancelSplit={() => editorState.splitting = false} />
{/if}
