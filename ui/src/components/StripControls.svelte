<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import type { SheetResult, SheetStrip, StripEdit, StripKey } from '../lib/core/types';
  import ArrangeControls from './ArrangeControls.svelte';
  import { sameKey } from '../lib/strip-edit';
  let { target, droppedNotice, result, selected, select, edit, undo, redo, automatic, count, canUndo, canRedo, split, setSplit, axis, setAxis, pairs, page, arrange = false, selections = [], selectMany }: {
    arrange?: boolean; selections?: StripKey[]; selectMany?: (keys:StripKey[])=>void;
    target: HTMLElement; result: SheetResult; selected: SheetStrip | undefined; select: (key: StripKey) => void;
    droppedNotice: Snippet<[boolean]>;
    edit: (e: StripEdit, page?: number) => void; undo: () => void; redo: () => void; automatic: () => void; count: number; canUndo: boolean; canRedo: boolean;
    split: boolean; setSplit: (v: boolean) => void; axis: 'x' | 'y'; setAxis: (v: 'x' | 'y') => void;
    pairs: [StripKey, StripKey][]; page: number;
  } = $props();
  let panel: HTMLDivElement;
  let dialog: HTMLDialogElement;
  export function confirmAutomatic() { dialog.showModal(); }
  const plan = $derived(result.plan);
  const join = $derived(pairs.find(p => selected?.bands.some(b => p.some(k => sameKey(k, b.key))) && p.every(k => plan.strips.some(s => s.bands.some(b => sameKey(k, b.key))))));
  onMount(() => { target.append(panel); return () => panel.remove(); });
</script>

<div bind:this={panel} class="controls" role="region" aria-label="Strip controls" data-ui>
  {#if arrange && selectMany}
    <ArrangeControls {result} {selections} select={selectMany} {edit} {page}/>
    {@render droppedNotice(true)}
  {:else}
  <h3>Strips</h3>
  <div class="two">
    <button onclick={undo} disabled={!canUndo}>Undo</button><button onclick={redo} disabled={!canRedo}>Redo</button>
  </div>
  {@render droppedNotice(true)}
  {#if selected}
    <label class="field">Label <input aria-label="Strip label" value={selected.label} onchange={e => edit({ op: 'label', strip: selected.key, text: e.currentTarget.value })} /></label>
    <div class="two">
      <button onclick={() => selected && edit({ op: selected.hidden ? 'show' : 'hide', strip: selected.key })}>{selected.hidden ? 'Show' : 'Hide'}</button>
      <button onclick={() => setSplit(!split)} disabled={!!selected.hidden} aria-pressed={split}>Split</button>
      <button disabled={!join} onclick={() => join && edit({ op: 'join', band_a: join[0], band_b: join[1] })}>Join</button>
      <button disabled={selected.bands.length < 2} onclick={() => selected && edit({ op: 'separate', strip: selected.key })}>Separate</button>
    </div>
    {#if split}
      <label class="check"><input type="checkbox" checked={axis === 'y'} onchange={e => setAxis(e.currentTarget.checked ? 'y' : 'x')} /> Horizontal line</label>
      <p class="faint small">Click a band to place the line. Escape cancels.</p>
    {/if}
  {:else}<p class="faint small">Select a strip to move, crop or split it.</p>{/if}
  {#if plan.strips.some(s => s.hidden)}
    <label class="field">Hidden strips <select aria-label="Hidden strips" value="" onchange={e => { const s = plan.strips.find(s => JSON.stringify(s.key) === e.currentTarget.value); if (s) select(s.key); }}>
      <option value="">Choose a strip</option>{#each plan.strips.filter(s => s.hidden) as s}<option value={JSON.stringify(s.key)}>{s.label || s.area}</option>{/each}
    </select></label>
  {/if}
  <button onclick={() => edit({ op: 'repack' })}>Re-pack the others</button>
  <button disabled={!count} onclick={() => dialog.showModal()}>Back to automatic</button>
  <div class="two">
    <button onclick={() => { edit({ op: 'add_sheet', at: plan.sheet_bodies.length }, plan.sheet_bodies.length); }}>Add page</button>
    <button disabled={plan.sheet_bodies.length < 2 || plan.strips.some(s => s.sheet === page && !s.hidden)} onclick={() => edit({ op: 'remove_sheet', sheet: page })}>Remove empty page</button>
    <button disabled={page === 0} onclick={() => { edit({ op: 'move_sheet', from: page, to: page - 1 }, page - 1); }}>Page earlier</button>
    <button disabled={page >= plan.sheet_bodies.length - 1} onclick={() => { edit({ op: 'move_sheet', from: page, to: page + 1 }, page + 1); }}>Page later</button>
  </div>
  {#if result.problems?.length}
    <div data-testid="strip-problems" role="status"><h3>Fix before export</h3>
      {#each result.problems as problem}<p class="small">{problem.message} {#each problem.strips as key}<button onclick={() => select(key)}>Show strip</button>{/each}</p>{/each}
    </div>
  {/if}
  {/if}
</div>
<dialog bind:this={dialog} class="glass panel" aria-label="Back to automatic">
  <p>Drop all {count} edits and go back to the automatic layout?</p>
  <form method="dialog" class="two">
    <!-- svelte-ignore a11y_autofocus -->
    <button autofocus>Cancel</button><button onclick={automatic}>Drop edits</button>
  </form>
</dialog>
<style>
  .controls { display: grid; gap: var(--space-2); }
  .two { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--space-1); }
  dialog { max-width: min(28rem, calc(100vw - 2rem)); background: var(--glass-strong); color: var(--text); }
  dialog::backdrop { background: color-mix(in srgb, var(--bg) 55%, transparent); }
</style>
