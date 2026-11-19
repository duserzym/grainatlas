<script lang="ts">
  import type { AreaImageEdits } from '../lib/core/types';
  import { emptyImageEdits, imageRecipe } from '../lib/image-edits';
  import Histogram from './Histogram.svelte';
  import { primaryModifier } from '../lib/platform';
  export type ImageTool = 'rect' | 'polygon' | 'mask' | 'erase' | null;
  let { edits, tool=$bindable(null), radius=$bindable(12), hist, leftOff, registered, canUndo, canRedo, change, undo, redo, close, finishPolygon, polygonCount, readOnly=false }: {
    edits: AreaImageEdits; tool?: ImageTool; radius?: number; hist: number[]|Uint32Array|null;
    leftOff: number; registered: number; canUndo: boolean; canRedo: boolean;
    change: (e: AreaImageEdits)=>void; undo:()=>void; redo:()=>void; close:()=>void;
    finishPolygon:()=>void; polygonCount:number; readOnly?: boolean;
  }=$props();
  const black=$derived((edits.levels?.black??0)/255);
  const white=$derived((edits.levels?.white??255)/255);
  const quarterTurns=$derived(Math.sign(edits.rotate_deg)*Math.floor((Math.abs(edits.rotate_deg)+45-1e-9)/90));
  const fine=$derived(edits.rotate_deg-quarterTurns*90);
  function angle(n:number) { change({...edits,rotate_deg:quarterTurns*90+Math.max(-45,Math.min(45,n||0))}); }
  function levels(black:number,white:number) {
    black=Math.round(Math.max(0,Math.min(254,Number.isFinite(black)?black:0)));
    white=Math.round(Math.max(black+1,Math.min(255,Number.isFinite(white)?white:255)));
    change({...edits,levels:{black,white}});
  }
  function auto() { if (!hist) return; const total=Array.from(hist).reduce((a,b)=>a+b,0); let sum=0,black=0,white=255; for(let i=0;i<256;i++){sum+=hist[i];if(sum<=total*0.01)black=i;if(sum<total*0.99)white=i+1;} levels(black,white); }
</script>
<h2>Edit image</h2>
<div class="image-controls">
<fieldset disabled={readOnly}>
  <div class="section"><h3>Crop</h3><div class="two"><button aria-pressed={tool==='rect'} onclick={()=>tool=tool==='rect'?null:'rect'}>Rectangle</button><button aria-pressed={tool==='polygon'} onclick={()=>tool=tool==='polygon'?null:'polygon'}>Polygon</button></div>
  <p class="faint small">Drag a rectangle or click polygon corners. Drag handles to adjust.</p>
  {#if tool==='polygon'}<button disabled={polygonCount<3} onclick={finishPolygon}>Finish polygon</button>{/if}</div>
  <div class="section"><h3>Rotate</h3><div class="two"><button onclick={()=>change({...edits,rotate_deg:edits.rotate_deg-90})}>90° left</button><button onclick={()=>change({...edits,rotate_deg:edits.rotate_deg+90})}>90° right</button></div>
  <label class="field angle">Fine angle<input aria-label="Rotation angle" type="number" min="-45" max="45" step="0.1" value={Number(fine.toFixed(1))} onchange={e=>angle(e.currentTarget.valueAsNumber)} /></label>
  <input aria-label="Rotation angle slider" type="range" min="-45" max="45" step="0.1" value={fine} onchange={e=>angle(e.currentTarget.valueAsNumber)} />
  <div class="two"><button aria-pressed={edits.flip_h} onclick={()=>change({...edits,flip_h:!edits.flip_h})}>Flip horizontal</button><button aria-pressed={edits.flip_v} onclick={()=>change({...edits,flip_v:!edits.flip_v})}>Flip vertical</button></div></div>
  <div class="section"><div class="heading"><h3>Levels</h3><button disabled={!hist} onclick={auto}>Auto</button></div>
  <Histogram bins={hist ? Uint32Array.from(hist) : null} bind:black={()=>black,n=>levels(n*255,white*255)} bind:white={()=>white,n=>levels(black*255,n*255)} gamma={1} />
  <div class="two"><label class="field">Black point<input aria-label="Black point" type="number" min="0" max="254" value={edits.levels?.black??0} onchange={e=>levels(e.currentTarget.valueAsNumber,edits.levels?.white??255)} /></label><label class="field">White point<input aria-label="White point" type="number" min="1" max="255" value={edits.levels?.white??255} onchange={e=>levels(edits.levels?.black??0,e.currentTarget.valueAsNumber)} /></label></div></div>
  <div class="section"><h3>Mask</h3><div class="two"><button aria-pressed={tool==='mask'} onclick={()=>tool=tool==='mask'?null:'mask'}>Paint out</button><button aria-pressed={tool==='erase'} onclick={()=>tool=tool==='erase'?null:'erase'}>Erase mask</button></div><label class="field angle">Brush radius (µm)<input aria-label="Mask brush radius" type="number" min="1" bind:value={radius} /></label><p class="faint small">Paint out tape and epoxy edges.</p></div>
  <div class="section"><h3>Edit list</h3><ol>{#each imageRecipe(edits) as item}<li>{item}</li>{/each}</ol></div>
</fieldset>
</div>
<div class="image-footer" data-testid="image-edit-footer">
  <p class="faint small" data-testid="image-crop-count">{leftOff} grains left off by the crop</p>
  {#if registered}<p class="small" role="status">{registered===1?'1 of these grains has a registered spot, which stays registered.':`${registered} of these grains have registered spots, which stay registered.`}</p>{/if}
  <p class="faint small">Edits change only how the image is shown and printed. Spots, grains and stage coordinates stay where they are. Original pixels are kept.</p>
  <div class="actions">
    <button disabled={readOnly || !canUndo} onclick={undo} title={`${primaryModifier()}+Z`}>Undo</button>
    <button disabled={readOnly || !canRedo} onclick={redo} title={`${primaryModifier()}+Shift+Z`}>Redo</button>
    <button disabled={readOnly} onclick={()=>change(emptyImageEdits())}>Reset</button>
    <button onclick={close} title="Leave Edit image mode">Done</button>
  </div>
</div>
<style>
  .heading { display:flex; align-items:center; justify-content:space-between; gap:var(--space-1); }
  .actions { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:var(--space-1); }
  .actions button {min-width:0;padding-inline:var(--space-1)}
  .image-footer {position:relative;flex:none;display:grid;gap:var(--space-2);border-top:1px solid var(--hairline);padding-top:var(--space-2)}
  .image-footer::before {content:'';position:absolute;bottom:100%;left:0;right:12px;height:18px;background:linear-gradient(transparent,var(--bg));pointer-events:none}
  .image-controls {flex:1;min-height:0;overflow-y:scroll;scrollbar-gutter:stable;scrollbar-width:thin;scrollbar-color:var(--text-faint) var(--hairline);padding-right:var(--space-1);padding-bottom:18px;scroll-padding-bottom:18px}
  .image-controls::-webkit-scrollbar {width:8px}
  .image-controls::-webkit-scrollbar-track {background:var(--hairline);border-radius:var(--radius-sm)}
  .image-controls::-webkit-scrollbar-thumb {background:var(--text-faint);border-radius:var(--radius-sm)}
  fieldset { align-content:start;border:0; padding:0; margin:0; display:grid; gap:var(--space-2); min-width:0; }
  .section { display:grid; gap:var(--space-1); padding-top:var(--space-1); }
  h2,h3,p,ol { margin:0; } h2 {flex:none;font-size:1rem} h3 {font-size:0.8rem}
  .two {display:grid;grid-template-columns:1fr 1fr;gap:var(--space-1)}
  .angle {display:grid;grid-template-columns:1fr 5rem;align-items:center}
  input {min-width:0;width:100%} ol {padding-left:1.5rem;font-size:0.8rem}
  .section :global(.hist) {height:32px}
  @media(max-height:820px) { fieldset {gap:3px}.section{gap:3px}.section :global(button){padding:4px 6px;min-height:26px;font-size:12px}.section :global(input[type=number]){height:26px}.section :global(.hist){height:26px} }
</style>
