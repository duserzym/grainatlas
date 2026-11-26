<script lang="ts">
  import PanZoom from '../lib/canvas/PanZoom.svelte';
  import type { ViewPoint } from '../lib/canvas/gestures';
  import type { PiecePlacement, SheetPlan, SheetStrip, StripEdit, StripKey } from '../lib/core/types';
  import { sameKey, contains, intersects, keyId, firstFreePlace } from '../lib/strip-edit';
  import { align, chosen, clampGroup, distribute, groupMove, groupRect, groupScale, pieceLabels, pieceMatrix, pieceRect, placed, snapGroup, type Guide, type Point } from '../lib/arrange';
  let { plan, pages, urls, page=$bindable(0), selections, select, edit, snap, setSnap, undo, redo, canUndo, canRedo, automatic }: {
    plan:SheetPlan; pages:string[]; urls:string[]; page?:number; selections:StripKey[]; select:(keys:StripKey[])=>void;
    edit:(e:StripEdit,destination?:number)=>void; snap:boolean; setSnap:(v:boolean)=>void;
    undo:()=>void;redo:()=>void;canUndo:boolean;canRedo:boolean;automatic:()=>void;
  }=$props();
  const PX=6;
  let view:ReturnType<typeof PanZoom>;
  let scope=$state('all');
  let hint=$state('');
  let guides=$state<Guide[]>([]);
  let preview=$state<PiecePlacement[]>([]);
  type Drag={ start:Point; current:Point; screen:Point; strips:SheetStrip[]; corner?:number; box:boolean; moved:boolean; add:boolean };
  let drag=$state<Drag|null>(null);
  const strips=$derived(plan.strips.filter(s=>!s.hidden&&s.sheet===page));
  const selected=$derived(chosen(plan,selections).filter(s=>s.sheet===page));
  const labels=$derived(pieceLabels(plan.strips));
  const selectionLabel=$derived(selected.length===1?labels.get(keyId(selected[0].key))??'':`${selected.length} pieces selected`);
  const shown=$derived.by(()=>{const ps=new Map(preview.map(p=>[keyId(p.strip),p]));return strips.map(s=>{const p=ps.get(keyId(s.key));return p?placed(s,p):s;});});
  const group=$derived(groupRect(shown.filter(s=>selections.some(k=>sameKey(k,s.key)))));
  // Keep the renderer's exact imagery, labels, reference circles and geometry.
  const svg=$derived.by(()=>{
    const doc=new DOMParser().parseFromString(pages[page]??'<svg/>','image/svg+xml');
    const pieces:Record<string,string>={};
    for(const g of doc.querySelectorAll('g[data-piece]')) { pieces[g.getAttribute('data-piece')!]=g.innerHTML; g.remove(); }
    return {header:doc.documentElement.innerHTML,pieces};
  });
  export function cancel(){drag=null;preview=[];guides=[];}
  function press(p:ViewPoint){
    hint=''; const at:Point=[p.x/PX,p.y/PX];
    const el=(p.ev.target as Element).closest('[data-strip]');
    const s=strips.find(s=>keyId(s.key)===el?.getAttribute('data-strip'));
    const handle=(p.ev.target as Element).getAttribute('data-corner');
    let keys=selections;
    if(s){
      if(p.ev.shiftKey) keys=keys.some(k=>sameKey(k,s.key))?keys.filter(k=>!sameKey(k,s.key)):[...keys,s.key];
      else if(!keys.some(k=>sameKey(k,s.key))) keys=[s.key];
      select(keys);
    }else if(handle===null&&!p.ev.shiftKey){keys=[];select([]);}
    drag={start:at,current:at,screen:[p.ev.clientX,p.ev.clientY],strips:chosen(plan,keys).filter(s=>s.sheet===page),corner:handle===null?undefined:Number(handle),box:!s&&handle===null,moved:false,add:p.ev.shiftKey};
    return true;
  }
  function update(p:ViewPoint){
    if(!drag)return;
    drag.current=[p.x/PX,p.y/PX];drag.moved ||= Math.hypot(p.ev.clientX-drag.screen[0],p.ev.clientY-drag.screen[1])>3;
    if(drag.box||!drag.strips.length)return;
    const dx=drag.current[0]-drag.start[0],dy=drag.current[1]-drag.start[1];
    if(drag.corner!==undefined){
      const r=groupRect(drag.strips),c=drag.corner;
      const anchor:Point=[c===0||c===3?r[2]:r[0],c===0||c===1?r[3]:r[1]];
      const v:Point=[drag.start[0]-anchor[0],drag.start[1]-anchor[1]];
      const factor=Math.max(0.02,((v[0]+dx)*v[0]+(v[1]+dy)*v[1])/(v[0]**2+v[1]**2));
      preview=groupScale(drag.strips,factor,anchor);guides=[];
    }else{
      preview=groupMove(drag.strips,[dx,dy]);
      if(snap&&!p.ev.altKey){const snapped=snapGroup(plan,drag.strips.map((s,i)=>placed(s,preview[i])));guides=snapped.guides;preview=preview.map(p=>({...p,origin_mm:[p.origin_mm[0]+snapped.delta[0],p.origin_mm[1]+snapped.delta[1]]}));}else guides=[];
    }
    const moving=drag.strips.map((s,i)=>placed(s,preview[i]));
    const correction=clampGroup(moving,plan.sheet_bodies[page].body_rect_mm);
    if(correction[1]>0) hint='Pieces stay below the header';
    preview=preview.map(p=>({...p,origin_mm:[p.origin_mm[0]+correction[0],p.origin_mm[1]+correction[1]]}));
  }
  function release(p:ViewPoint){
    if(!drag)return;
    if(p.ev.type==='pointercancel'){cancel();return;}
    update(p);const d=drag;
    if(d.box&&d.moved){
      const box:[number,number,number,number]=[Math.min(d.start[0],d.current[0]),Math.min(d.start[1],d.current[1]),Math.max(d.start[0],d.current[0]),Math.max(d.start[1],d.current[1])];
      const keys=strips.filter(s=>intersects(box,pieceRect(s))||contains(box,pieceRect(s))).map(s=>s.key);
      select(d.add?[...selections,...keys.filter(k=>!selections.some(a=>sameKey(a,k)))]:keys);
    }else if(d.moved&&preview.length){
      const thumb=document.elementFromPoint(p.ev.clientX,p.ev.clientY)?.closest('[data-sheet]');
      if(thumb){const dest=Number(thumb.getAttribute('data-sheet'));const moving=d.strips.map((s,i)=>placed(s,preview[i])),r=groupRect(moving);
        const taken=plan.strips.filter(s=>!s.hidden&&s.sheet===dest&&!d.strips.some(a=>sameKey(a.key,s.key))).map(pieceRect);
        const at=firstFreePlace(plan.sheet_bodies[dest].body_rect_mm,[r[2]-r[0],r[3]-r[1]],taken);
        if(at)edit({op:'transform',pieces:groupMove(moving,[at[0]-r[0],at[1]-r[1]]).map(p=>({...p,sheet:dest}))},dest);
        else hint='No room on that sheet';
      }else edit({op:'transform',pieces:preview});
    }
    cancel();
  }
  function autoArrange(){const keys=plan.strips.filter(s=>!s.hidden&&(scope==='all'||(scope==='selected')===selections.some(k=>sameKey(k,s.key)))).map(s=>s.key);if(keys.length)edit({op:'auto_arrange',strips:keys});}
</script>

<div class="arrange-view">
  <div class="toolbar glass" data-testid="arrange-toolbar" data-ui>
    <div class="row">
      <button onclick={autoArrange}>Auto-arrange</button><select aria-label="Auto-arrange scope" bind:value={scope}><option value="all">All pieces</option><option value="selected">Selected</option><option value="unselected">Unselected</option></select>
      <button aria-pressed={snap} onclick={()=>setSnap(!snap)}>Snap {snap?'on':'off'}</button>
      <button onclick={undo} disabled={!canUndo}>Undo</button><button onclick={redo} disabled={!canRedo}>Redo</button><button onclick={automatic}>Back to automatic</button>
    </div>
    <div class="row" aria-label="Align pieces"><span class="faint small">Align</span>
      {#each ['Left','Centre','Right','Top','Middle','Bottom'] as name,i}<button disabled={selected.length<2} onclick={()=>edit({op:'transform',pieces:align(selected,i<3?0:1,(i%3)/2)})}>{name}</button>{/each}
      <button disabled={selected.length<3} onclick={()=>edit({op:'transform',pieces:distribute(selected)})}>Distribute</button><span class="faint small">Position on paper · mm</span>
    </div>
  </div>
  <div class="workspace">
    <nav class="thumbs" aria-label="Print pages" data-ui>{#each urls as url,i}<button data-testid="page-thumb" data-sheet={i} aria-label="Page {i+1}" aria-current={page===i?'page':undefined} onclick={()=>{page=i;select([]);}}><img src={url} alt=""/><span>{i+1}</span></button>{/each}</nav>
    <div class="paper-view">
      <PanZoom bind:this={view} w={plan.paper_mm[0]*PX} h={plan.paper_mm[1]*PX} fitKey={String(page)} label="Arrange pieces" gestures={{onpress:press,ondrag:update,onrelease:release,oncancel:cancel,canNudge:()=>false}}>
        <svg data-testid="page" width={plan.paper_mm[0]*PX} height={plan.paper_mm[1]*PX} viewBox={`0 0 ${plan.paper_mm.join(' ')}`} xmlns="http://www.w3.org/2000/svg" font-family="DejaVu Sans, sans-serif" aria-label="Print sheet">
          {@html svg.header}
          {#each shown as s,i (keyId(s.key))}
            {@const original=strips[i]}
            <g data-piece={keyId(s.key)} transform={`matrix(${pieceMatrix(s).join(' ')})`}><g transform={`translate(${-original.origin_mm[0]} ${-original.origin_mm[1]})`}>{@html svg.pieces[keyId(s.key)]??''}</g></g>
            {@const r=pieceRect(s)}
            <rect data-strip={keyId(s.key)} data-testid="strip-block" class="block" class:selected={selections.some(k=>sameKey(k,s.key))} x={r[0]} y={r[1]} width={r[2]-r[0]} height={r[3]-r[1]}/>
          {/each}
          {#if selected.length}
            {@const labelWidth=Math.min(plan.paper_mm[0]-8,selectionLabel.length*1.5+4)}
            {@const labelX=Math.max(4,Math.min(group[0],plan.paper_mm[0]-labelWidth-4))}
            <g class="selection-readout" data-testid="piece-selection-readout" transform={`translate(${labelX} ${Math.max(1,group[1]-7)})`}>
              <rect width={labelWidth} height="5.5" rx="0.7" />
              <text x="2" y="3.8" font-size="2.6">{selectionLabel}</text>
            </g>
            <rect class="selection" x={group[0]} y={group[1]} width={group[2]-group[0]} height={group[3]-group[1]} />
            {#each [[group[0],group[1]],[group[2],group[1]],[group[2],group[3]],[group[0],group[3]]] as p,i}<rect data-corner={i} data-testid={`resize-corner-${i}`} class="handle" x={p[0]-1.3} y={p[1]-1.3} width="2.6" height="2.6"/>{/each}
          {/if}
          {#if drag?.box&&drag.moved}<rect class="box" x={Math.min(drag.start[0],drag.current[0])} y={Math.min(drag.start[1],drag.current[1])} width={Math.abs(drag.current[0]-drag.start[0])} height={Math.abs(drag.current[1]-drag.start[1])}/>{/if}
          {#each guides as g}<line data-testid="snap-guide" class="guide" x1={g.axis===0?g.at:0} x2={g.axis===0?g.at:plan.paper_mm[0]} y1={g.axis===1?g.at:0} y2={g.axis===1?g.at:plan.paper_mm[1]}/>{/each}
        </svg>
      </PanZoom>
      <div class="tools glass" data-ui><button aria-label="Zoom out" title="Zoom out" onclick={()=>view.zoomBy(1/1.5)}>−</button><button aria-label="Zoom in" title="Zoom in" onclick={()=>view.zoomBy(1.5)}>+</button><button onclick={()=>view.fitAll()} title="Fit" aria-label="Fit">Fit</button><span class="small">{page+1} / {urls.length}</span></div>
      {#if hint}<p class="hint glass small" role="status">{hint}</p>{/if}
    </div>
  </div>
</div>
<style>
  .arrange-view { position:absolute;inset:0;display:grid;grid-template-rows:auto minmax(0,1fr);gap:var(--space-2); }
  .toolbar { padding:var(--space-2);display:grid;gap:var(--space-1); }
  .row { display:flex;flex-wrap:wrap;align-items:center;gap:var(--space-1); }
  .row button,.row select { padding:var(--space-1) var(--space-2);font-size:0.85rem; }
  .workspace { display:grid;grid-template-columns:4rem minmax(0,1fr);min-height:0; }
  .thumbs { overflow:auto;display:flex;flex-direction:column;gap:var(--space-1);padding:var(--space-1); }
  .thumbs button { flex:none;padding:var(--space-1); }.thumbs img { width:100%;display:block; }.thumbs [aria-current] {outline:2px solid var(--accent);}
  .paper-view {position:relative;min-height:0;overflow:hidden;}.tools {position:absolute;bottom:var(--space-2);left:50%;transform:translateX(-50%);display:flex;gap:var(--space-1);align-items:center;padding:var(--space-1);}
  .block {fill:transparent;stroke:transparent;cursor:move;}.block.selected {stroke:var(--accent);stroke-width:0.35;}.selection {fill:none;stroke:var(--accent);stroke-width:0.3;stroke-dasharray:1 1;pointer-events:none;}.handle {fill:white;stroke:var(--accent);stroke-width:0.35;cursor:nwse-resize;}.box {fill:var(--accent-soft);stroke:var(--accent);stroke-width:0.3;pointer-events:none;}.guide {stroke:var(--accent);stroke-width:0.2;pointer-events:none;}.hint {position:absolute;top:var(--space-2);left:var(--space-2);padding:var(--space-2);pointer-events:none;}
  .selection-readout {pointer-events:none;}.selection-readout rect {fill:var(--bg);stroke:var(--accent);stroke-width:0.2;}.selection-readout text {fill:var(--accent);}
</style>
