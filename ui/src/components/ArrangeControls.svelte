<script lang="ts">
  import type { SheetResult, StripEdit, StripKey } from '../lib/core/types';
  import { chosen, formatPieceScale, groupMove, groupRect, groupScale, pieceLabels, pieceRect, placement, type Point } from '../lib/arrange';
  import { sameKey, firstFreePlace, keyId } from '../lib/strip-edit';
  import { primaryModifier } from '../lib/platform';
  let { result, selections, select, edit, page }:{result:SheetResult;selections:StripKey[];select:(keys:StripKey[])=>void;edit:(e:StripEdit,destination?:number)=>void;page:number}=$props();
  const plan=$derived(result.plan);
  const labels=$derived(pieceLabels(plan.strips));
  const selected=$derived(chosen(plan,selections).filter(s=>s.sheet===page));
  const one=$derived(selected.length===1?selected[0]:undefined);
  const group=$derived(groupRect(selected));
  const mixed=$derived(plan.strips.filter(s=>!s.hidden).some(s=>Math.abs((s.transform?.scale??1)-(plan.strips.find(s=>!s.hidden)?.transform?.scale??1))>1e-7));
  function size(value:number,axis:0|1){const old=group[axis+2]-group[axis];if(value>0&&old>0)edit({op:'transform',pieces:groupScale(selected,value/old)});}
  function move(value:number,axis:0|1){if(Number.isFinite(value)){const d:Point=[0,0];d[axis]=value-group[axis];edit({op:'transform',pieces:groupMove(selected,d)});}}
  function rotate(deg:number){if(one&&Number.isFinite(deg))edit({op:'transform',pieces:[{...placement(one),rotation_deg:deg}]});}
  function scale(input:HTMLInputElement){
    const value=Number(input.value);
    if(one&&value>0&&Number.isFinite(value)){
      edit({op:'transform',pieces:groupScale([one],plan.um_per_mm/value/(one.transform?.scale??1))});
      input.value=formatPieceScale(value);
    }
  }
  function sheet(dest:number){
    const r=groupRect(selected),body=plan.sheet_bodies[dest].body_rect_mm;
    const taken=plan.strips.filter(s=>!s.hidden&&s.sheet===dest&&!selected.some(a=>sameKey(a.key,s.key))).map(pieceRect);
    const at=firstFreePlace(body,[r[2]-r[0],r[3]-r[1]],taken);
    if(at)edit({op:'transform',pieces:groupMove(selected,[at[0]-r[0],at[1]-r[1]]).map(p=>({...p,sheet:dest}))},dest);
    else notice='No room on that sheet';
  }
  let notice=$state('');
</script>
<h3>Pieces on sheet {page+1}</h3>
<div class="pieces" aria-label="Piece list">
  {#each plan.strips.filter(s=>!s.hidden&&s.sheet===page) as s}<button class:selected={selections.some(k=>sameKey(k,s.key))} aria-pressed={selections.some(k=>sameKey(k,s.key))} onclick={e=>select(e.shiftKey?[...selections.filter(k=>!sameKey(k,s.key)),...(selections.some(k=>sameKey(k,s.key))?[]:[s.key])]:[s.key])}><span title={labels.get(keyId(s.key))}>{labels.get(keyId(s.key))}</span><span class="faint small">{s.bands.reduce((n,b)=>n+b.grains.length,0)} grains</span></button>{/each}
</div>
{#if selected.length}
  <h3>{one?labels.get(keyId(one.key)):`${selected.length} pieces selected`}</h3>
  {#if one}<label class="field">Scale (µm per mm)<input aria-label="Piece scale" type="number" min="0.001" step="any" value={formatPieceScale(plan.um_per_mm/(one.transform?.scale??1))} onchange={e=>scale(e.currentTarget)}/></label>{/if}
  <p class="faint small">Aspect ratio locked when resizing.</p>
  <div class="two">
    {#each ['Position x (mm)','Position y (mm)','Width (mm)','Height (mm)'] as label,i}<label class="field">{label}<input aria-label={label} type="number" step="0.1" value={Number((i<2?group[i]:group[i]-group[i-2]).toFixed(2))} onchange={e=>i<2?move(Number(e.currentTarget.value),i as 0|1):size(Number(e.currentTarget.value),(i-2) as 0|1)}/></label>{/each}
  </div>
  <div class="two"><button onclick={()=>size(plan.sheet_bodies[page].body_rect_mm[2]-plan.sheet_bodies[page].body_rect_mm[0],0)}>Fit width</button><button onclick={()=>edit({op:'transform',pieces:selected.map(s=>({...placement(s),scale:1}))})}>Reset size</button></div>
  {#if one}
    <div class="two"><button aria-label="Rotate left 90 degrees" onclick={()=>rotate((one!.transform?.rotation_deg??0)-90)}>Rotate −90°</button><button aria-label="Rotate right 90 degrees" onclick={()=>rotate((one!.transform?.rotation_deg??0)+90)}>Rotate +90°</button></div>
    <label class="field">Rotation (°)<input aria-label="Piece rotation" type="number" step="0.1" value={one.transform?.rotation_deg??0} onchange={e=>rotate(Number(e.currentTarget.value))}/></label>
  {/if}
  <label class="field">Move to sheet<select aria-label="Move to sheet" value={page} onchange={e=>sheet(Number(e.currentTarget.value))}>{#each plan.sheet_bodies as b,i}<option value={i}>Sheet {i+1}</option>{/each}</select></label>
  <p class="faint small">Labels kept at 6 pt minimum.</p>
{:else}<p class="faint small">Select a piece, or drag a box on empty paper.</p>{/if}
{#if mixed}<div class="warning panel small" role="status"><b>Different scales</b><p>Each piece has its own scale bar and spot-size circles. Spot sizes are shown with each piece.</p></div>{/if}
<p class="faint small">Drag to move. Drag a corner to resize. Shift-click adds. Arrow keys nudge 1 mm; Shift+arrow 10 mm. Alt turns snapping off. {primaryModifier()}+Z undoes; Shift+{primaryModifier()}+Z redoes.</p>
<p class="faint small">Pieces stay below the header. Overlapping drops are refused.</p>
{#if notice}<p class="small" role="status">{notice}</p>{/if}
<button onclick={()=>edit({op:'add_sheet',at:plan.sheet_bodies.length},plan.sheet_bodies.length)}>Add sheet</button>
<style>
  .pieces {display:grid;gap:var(--space-1);max-height:10rem;overflow:auto;}.pieces button {display:flex;justify-content:space-between;align-items:center;gap:var(--space-1);text-align:left;}.pieces button span:first-child {min-width:0;overflow-wrap:anywhere;}.pieces button span:last-child {flex:none;}.pieces button.selected {border-color:var(--accent);background:var(--accent-soft);}.two {display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:var(--space-1);}.warning {background:var(--accent-soft);padding:var(--space-2);}.warning p {margin-bottom:0;}
</style>
