<script lang="ts">
  // Canvas screen: the infinite canvas with grain outlines, numbers and scale
  // bars, the grain list and the manual grain tools.
  import { app, type Tool } from '../lib/store.svelte';
  import Viewer, { type CanvasPoint } from '../lib/canvas/Viewer.svelte';
  import type { View } from '../lib/canvas/view';
  import type { Core } from '../lib/core/core';
  import type { Grain, SegmentParams } from '../lib/core/types';
  import { download } from '../lib/files';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  let viewer = $state<ReturnType<typeof Viewer>>();
  let view = $state<View | undefined>(undefined);
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  let cursorUm = $state<[number, number] | null>(null);
  let selGrains = $state<string[]>([]);
  let splitLine = $state<[number, number, number, number] | null>(null);
  let tab = $state<'grains' | 'spots'>('grains');

  const areaId = $derived(app.areaId && app.stitched[app.areaId] ? app.areaId : (Object.keys(app.stitched)[0] ?? null));
  const mosaic = $derived(areaId ? app.stitched[areaId].mosaic : null);
  const umPerPx = $derived(mosaic?.um_per_px ?? 1);
  const grains = $derived(areaId ? (app.grains[areaId] ?? []) : []);
  const segParams = $derived(areaId ? app.params[areaId]?.segment : null);
  /** screen px → um at the current zoom */
  const k = $derived(view ? umPerPx / view.zoom : 1);
  const rotDeg = $derived(view ? (view.rot * 180) / Math.PI : 0);

  $effect(() => {
    const t = areaId ? app.tone[areaId] : null;
    if (!t || !core) return;
    const snap = $state.snapshot(t);
    core.toneLut(snap).then((l) => (lut = l));
  });

  const scalebar = $derived.by(() => {
    if (!view) return null;
    const target = 140 * k;
    const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];
    const len = steps.filter((s) => s <= target).pop() ?? 1;
    return { len, px: len / k };
  });

  // ---- grains ---------------------------------------------------------------
  async function findGrains() {
    if (!core || !areaId || !segParams) return;
    const id = areaId;
    const params = $state.snapshot(segParams) as SegmentParams;
    const prev = $state.snapshot(app.grains[id] ?? []) as Grain[];
    const r = await app.run('Finding grains', () => core!.segment(id, params, prev));
    if (!r) return;
    app.checkpoint();
    app.grains = { ...app.grains, [id]: r };
    relinkSpots(id);
  }

  function setGrains(gs: Grain[]) {
    if (!areaId) return;
    app.checkpoint();
    app.grains = { ...app.grains, [areaId]: gs };
    relinkSpots(areaId);
  }

  /** Spots follow the grain under them (number changes, merges, splits). */
  function relinkSpots(id: string) {
    const gs = app.grains[id] ?? [];
    app.spots = app.spots.map((s) => {
      if (s.area_id !== id) return s;
      const g = grainAtPoint(gs, s.x_um, s.y_um);
      return { ...s, grain_id: g?.id ?? null, grain_number: g?.number ?? null };
    });
  }

  function grainAtPoint(gs: Grain[], x: number, y: number): Grain | null {
    for (const g of gs) {
      const [x0, y0, x1, y1] = g.bbox_um;
      if (x < x0 || y < y0 || x > x1 || y > y1) continue;
      if (inside(g.polygon_um, x, y)) return g;
    }
    return null;
  }

  function inside(poly: [number, number][], px: number, py: number) {
    let c = false;
    for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
      const [ax, ay] = poly[i], [bx, by] = poly[j];
      if (ay > py !== by > py && px < ((bx - ax) * (py - ay)) / (by - ay) + ax) c = !c;
    }
    return c;
  }

  async function refine() {
    if (!core || !areaId) return;
    const id = areaId;
    const targets = grains.filter((g) => selGrains.includes(g.id));
    const out: Grain[] = [];
    for (const g of targets) {
      const r = await app.run(`Refining grain ${g.number}`, () => core!.refineGrain(id, $state.snapshot(g) as Grain));
      if (r) out.push(r);
    }
    setGrains(grains.map((g) => out.find((o) => o.id === g.id) ?? g));
  }

  async function merge() {
    if (!core || !areaId || selGrains.length < 2) return;
    const id = areaId;
    const targets = grains.filter((g) => selGrains.includes(g.id)).map((g) => $state.snapshot(g) as Grain);
    const m = await app.run('Merging grains', () => core!.mergeGrains(id, targets));
    if (!m) return;
    setGrains([...grains.filter((g) => !selGrains.includes(g.id)), m].sort((a, b) => a.number - b.number));
    selGrains = [m.id];
  }

  function deleteGrains() {
    if (!selGrains.length) return;
    setGrains(grains.filter((g) => !selGrains.includes(g.id)));
    selGrains = [];
  }

  async function renumber(fromHere = false) {
    if (!core || !areaId || !segParams) return;
    let from: { grainId: string; number: number } | undefined;
    if (fromHere) {
      const g = grains.find((x) => x.id === selGrains[0]);
      if (!g) return;
      const n = Number(prompt(`Renumber from grain ${g.number}: new number`, String(g.number)));
      if (!n) return;
      from = { grainId: g.id, number: n };
    }
    const r = await core.renumber($state.snapshot(grains) as Grain[], segParams.row_tolerance, from);
    setGrains(r);
  }

  async function exportGrains() {
    if (!core || !areaId) return;
    const csv = await core.exportGrains(areaId, $state.snapshot(grains) as Grain[]);
    download(csv, `${areaId}_grains.csv`, 'text/csv');
  }

  function focusGrain(g: Grain) {
    selGrains = [g.id];
    const w = (g.bbox_um[2] - g.bbox_um[0]) / umPerPx, h = (g.bbox_um[3] - g.bbox_um[1]) / umPerPx;
    if (view) viewer?.centerOn(g.centroid_um[0] / umPerPx, g.centroid_um[1] / umPerPx, Math.min(view.w / (w * 3), view.h / (h * 3), 4));
  }

  // ---- pointer --------------------------------------------------------------
  function press(p: CanvasPoint): boolean {
    if (app.tool === 'grain-split') {
      splitLine = [p.x * umPerPx, p.y * umPerPx, p.x * umPerPx, p.y * umPerPx];
      return true;
    }
    return false;
  }
  function drag(p: CanvasPoint) {
    if (splitLine) splitLine = [splitLine[0], splitLine[1], p.x * umPerPx, p.y * umPerPx];
  }
  async function release() {
    if (!splitLine || !core || !areaId) return;
    const line = splitLine;
    splitLine = null;
    const mid: [number, number] = [(line[0] + line[2]) / 2, (line[1] + line[3]) / 2];
    const g = grainAtPoint(grains, ...mid) ?? grains.find((q) => selGrains.includes(q.id));
    if (!g) return;
    const id = areaId;
    const parts = await app.run(`Splitting grain ${g.number}`, () => core!.splitGrain(id, $state.snapshot(g) as Grain, line));
    if (!parts || parts.length < 2) return;
    const max = Math.max(0, ...grains.map((q) => q.number));
    parts[0] = { ...parts[0], id: g.id, number: g.number };
    for (let i = 1; i < parts.length; i++) parts[i] = { ...parts[i], number: max + i };
    setGrains([...grains.filter((q) => q.id !== g.id), ...parts].sort((a, b) => a.number - b.number));
  }
  async function click(p: CanvasPoint) {
    const x = p.x * umPerPx, y = p.y * umPerPx;
    if (app.tool === 'grain-add' && core && areaId && segParams) {
      const id = areaId;
      const g = await app.run('Growing grain', () => core!.grainAt(id, x, y, $state.snapshot(segParams) as SegmentParams));
      if (!g) {
        if (g === null) app.error = 'No grain at that point (background).';
        return;
      }
      const max = Math.max(0, ...grains.map((q) => q.number));
      const added = { ...g, number: max + 1 };
      setGrains([...grains.filter((q) => !inside(added.polygon_um, q.centroid_um[0], q.centroid_um[1])), added]);
      selGrains = [added.id];
      return;
    }
    const g = grainAtPoint(grains, x, y);
    const multi = (p.ev as MouseEvent).shiftKey || (p.ev as MouseEvent).metaKey;
    if (!g) {
      if (!multi) selGrains = [];
      return;
    }
    selGrains = multi ? (selGrains.includes(g.id) ? selGrains.filter((s) => s !== g.id) : [...selGrains, g.id]) : [g.id];
  }

  // ---- keyboard ---------------------------------------------------------------
  function key(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable) return;
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      if (e.shiftKey) app.redo();
      else app.undo();
      return;
    }
    if (mod) return;
    const tools: Record<string, Tool> = { v: 'select', g: 'grain-add', k: 'grain-split' };
    if (tools[e.key.toLowerCase()]) app.tool = tools[e.key.toLowerCase()];
    else if (e.key === '+' || e.key === '=') viewer?.zoomBy(1.5);
    else if (e.key === '-') viewer?.zoomBy(1 / 1.5);
    else if (e.key === '0') viewer?.fitAll();
    else if (e.key === '[') viewer?.rotateBy(-Math.PI / 12);
    else if (e.key === ']') viewer?.rotateBy(Math.PI / 12);
    else if ((e.key === 'Delete' || e.key === 'Backspace') && selGrains.length && tab === 'grains') deleteGrains();
    else if (e.key === 'Escape') {
      selGrains = [];
      app.tool = 'select';
    }
  }

  const cursor = $derived(app.tool === 'select' ? 'default' : 'crosshair');
  const fmt = (v: number, d = 0) => v.toFixed(d);
</script>

<svelte:window onkeydown={key} />

{#if core && areaId && mosaic}
  <div class="canvas">
    {#key areaId}
      <Viewer bind:this={viewer} bind:view {core} {areaId} {mosaic} {lut} version={app.pixelsVersion} {cursor}
        onpress={press} ondrag={drag} onrelease={release} onclick={click}
        onhover={(p) => (cursorUm = p ? [p.x * umPerPx, p.y * umPerPx] : null)}>
        {#snippet overlay()}
          <g transform="scale({1 / umPerPx})">
            {#if app.layers.outlines}
              {#each grains as g (g.id)}
                <polygon class="grain" class:sel={selGrains.includes(g.id)} points={g.polygon_um.map((p) => p.join(',')).join(' ')} />
              {/each}
            {/if}
            {#if app.layers.scalebars}
              {#each grains as g (g.id)}
                {@const y = g.bbox_um[3] + 6 * k}
                <g class="gbar">
                  <line x1={g.bbox_um[0]} x2={g.bbox_um[0] + g.scale_bar_um} y1={y} y2={y} />
                  <text transform="translate({g.bbox_um[0]} {y + 12 * k}) rotate({-rotDeg}) scale({k})">{g.scale_bar_um} µm</text>
                </g>
              {/each}
            {/if}
            {#if app.layers.numbers}
              {#each grains as g (g.id)}
                <text class="gnum" class:sel={selGrains.includes(g.id)} transform="translate({g.bbox_um[0]} {g.bbox_um[1]}) rotate({-rotDeg}) scale({k})" dy="-3">{g.number}</text>
              {/each}
            {/if}
            {#if splitLine}
              <line class="splitline" x1={splitLine[0]} y1={splitLine[1]} x2={splitLine[2]} y2={splitLine[3]} />
            {/if}
          </g>
        {/snippet}
      </Viewer>
    {/key}

    <div class="glass topleft" data-ui>
      <select bind:value={app.areaId} aria-label="Area">
        {#each Object.keys(app.stitched) as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
      </select>
      <span class="sep"></span>
      <button class="icon" aria-pressed={app.tool === 'select'} title="Select / move (V)" onclick={() => (app.tool = 'select')}>Select</button>
      <button class="icon" aria-pressed={app.tool === 'grain-add'} title="Add grain: click inside it (G)" onclick={() => (app.tool = 'grain-add')}>+ Grain</button>
      <button class="icon" aria-pressed={app.tool === 'grain-split'} title="Split grain: draw a line across it (K)" onclick={() => (app.tool = 'grain-split')}>Split</button>
      <span class="sep"></span>
      <button class="icon" title="Undo (⌘Z)" disabled={!app.canUndo} onclick={() => app.undo()}>↶</button>
      <button class="icon" title="Redo (⇧⌘Z)" disabled={!app.canRedo} onclick={() => app.redo()}>↷</button>
      <span class="sep"></span>
      <button class="icon" title="Zoom in (+)" onclick={() => viewer?.zoomBy(1.5)}>+</button>
      <button class="icon" title="Zoom out (−)" onclick={() => viewer?.zoomBy(1 / 1.5)}>−</button>
      <button class="icon" title="Fit (0)" onclick={() => viewer?.fitAll()}>Fit</button>
      <button class="icon" title="Pixels 1:1" onclick={() => viewer?.oneToOne()}>1:1</button>
      <button class="icon" title="Rotate left 15° ([)" onclick={() => viewer?.rotateBy(-Math.PI / 12)}>⟲</button>
      <button class="icon" title="Rotate right 15° (])" onclick={() => viewer?.rotateBy(Math.PI / 12)}>⟳</button>
    </div>

    <aside class="glass side" data-ui>
      <div class="tabs">
        <button aria-pressed={tab === 'grains'} onclick={() => (tab = 'grains')}>Grains <span class="faint">{grains.length}</span></button>
        <button aria-pressed={tab === 'spots'} onclick={() => (tab = 'spots')}>Spots</button>
      </div>
      {#if tab === 'grains' && segParams}
        <details class="params">
          <summary>Segmentation</summary>
          <label class="field">Grains are
            <select bind:value={segParams.polarity}><option value="bright">brighter than the epoxy</option><option value="dark">darker (transmitted light)</option></select>
          </label>
          <label class="field">Threshold
            <select bind:value={segParams.method}><option value="background">background + margin</option><option value="otsu">Otsu</option></select>
          </label>
          <label class="field">Fixed threshold (0–1, empty = automatic)
            <input type="number" step="0.01" min="0" max="1" value={segParams.threshold ?? ''} oninput={(e) => (segParams.threshold = e.currentTarget.value === '' ? null : +e.currentTarget.value)} />
          </label>
          <label class="field">Minimum size (µm) <input type="number" min="1" bind:value={segParams.min_size_um} /></label>
          <label class="check"><input type="checkbox" bind:checked={segParams.split} /> split touching grains</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.grabcut} /> GrabCut refinement (slower)</label>
        </details>
        <div class="row">
          <button class="primary" onclick={findGrains} data-testid="find-grains">{grains.length ? 'Find again' : 'Find grains'}</button>
          <button onclick={exportGrains} disabled={!grains.length}>Grain CSV</button>
        </div>
        <div class="row">
          <button onclick={refine} disabled={!selGrains.length} title="GrabCut on the selected grains">Refine</button>
          <button onclick={merge} disabled={selGrains.length < 2} title="Shift-click grains to select several">Merge</button>
          <button class="danger" onclick={deleteGrains} disabled={!selGrains.length}>Delete</button>
        </div>
        <div class="row">
          <button onclick={() => renumber(false)} disabled={!grains.length}>Renumber all</button>
          <button onclick={() => renumber(true)} disabled={selGrains.length !== 1}>Renumber from here</button>
        </div>
        <div class="list">
          <table class="data">
            <thead><tr><th>#</th><th>⌀ µm</th><th>L × W µm</th><th>bar</th></tr></thead>
            <tbody>
              {#each grains as g (g.id)}
                <tr class:selected={selGrains.includes(g.id)} onclick={() => focusGrain(g)}>
                  <td class="num">{g.number}{g.manual ? '*' : ''}</td><td class="num">{fmt(g.eq_diameter_um)}</td><td class="num">{fmt(g.major_um)} × {fmt(g.minor_um)}</td><td class="num">{g.scale_bar_um}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else if tab === 'spots'}
        <p class="muted">Spots arrive in phase 5.</p>
      {/if}
      <details class="layers" open>
        <summary>Layers</summary>
        <label class="check"><input type="checkbox" bind:checked={app.layers.outlines} /> outlines</label>
        <label class="check"><input type="checkbox" bind:checked={app.layers.numbers} /> numbers</label>
        <label class="check"><input type="checkbox" bind:checked={app.layers.scalebars} /> scale bars</label>
      </details>
    </aside>

    <div class="glass status" data-ui>
      {#if cursorUm}<span class="num">x {cursorUm[0].toFixed(1)} µm, y {cursorUm[1].toFixed(1)} µm</span>{/if}
      {#if view}<span class="num muted">{(view.zoom * 100).toFixed(view.zoom < 0.1 ? 1 : 0)}% · {rotDeg.toFixed(0)}°</span>{/if}
      {#if scalebar}
        <span class="bar"><span style:width="{scalebar.px}px"></span>{scalebar.len >= 1000 ? `${scalebar.len / 1000} mm` : `${scalebar.len} µm`}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .canvas {
    position: absolute;
    inset: 0;
  }
  .topleft {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    right: 22rem;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-2);
    align-items: center;
    width: fit-content;
  }
  .sep {
    width: 1px;
    height: 1.4rem;
    background: var(--hairline);
    margin: 0 var(--space-1);
  }
  .side {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    bottom: var(--space-2);
    width: 20.5rem;
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: hidden;
  }
  .tabs,
  .row {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .tabs button {
    flex: 1;
  }
  .params,
  .layers {
    display: grid;
    gap: var(--space-1);
    font-size: 0.88rem;
  }
  .params[open],
  .layers[open] {
    padding-bottom: var(--space-2);
  }
  .list {
    flex: 1;
    overflow: auto;
    min-height: 6rem;
  }
  .list tr {
    cursor: pointer;
  }
  .status {
    position: absolute;
    bottom: var(--space-2);
    left: var(--space-2);
    display: flex;
    gap: var(--space-4);
    padding: var(--space-1) var(--space-3);
    align-items: center;
    font-size: 0.85rem;
  }
  .bar {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .bar span {
    display: inline-block;
    height: 6px;
    border: 2px solid var(--text);
    border-top: 0;
  }
  :global(.gm-root) .grain {
    fill: none;
    stroke: var(--ov-outline);
    stroke-width: 1.4;
    opacity: 0.9;
  }
  :global(.gm-root) .grain.sel {
    stroke: var(--ov-number);
    stroke-width: 2.4;
    fill: color-mix(in srgb, var(--ov-number) 12%, transparent);
  }
  :global(.gm-root) .gnum {
    fill: var(--ov-number);
    font: 600 13px var(--font-sans);
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.75);
    stroke-width: 3px;
  }
  :global(.gm-root) .gbar line {
    stroke: var(--ov-label);
    stroke-width: 2;
  }
  :global(.gm-root) .gbar text {
    fill: var(--ov-label);
    font: 11px var(--font-sans);
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.75);
    stroke-width: 3px;
  }
  :global(.gm-root) .splitline {
    stroke: var(--danger);
    stroke-width: 2;
    stroke-dasharray: 6 4;
  }
</style>
