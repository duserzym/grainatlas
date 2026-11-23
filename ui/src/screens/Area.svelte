<script lang="ts">
  import { areaImageEdits, levelsLut } from '../lib/image-edits';
  import { bundlePixels, neutralTone } from '../lib/bundle-tone';
  import { grainNumber } from '../lib/grain-lineage';
  // Area screen: the area in the shared image view (tile thumbnails at their
  // stage positions before stitching, the mosaic after), metadata, stitch and
  // correction controls with a live histogram, per-tile overrides and the
  // residual map. Every correction and tone control re-renders the mosaic live
  // (App.svelte watches the parameters; lib/actions.ts liveChanged).
  import { app, current } from '../lib/store.svelte';
  import { findGrainsInArea } from '../lib/grain-finding';
  import { autoTone, changeScale, exportPng, resetTone, scaleText, stitch, stitchAll, toggleBeforeAfter } from '../lib/actions';
  import { rawToCanvas } from '../lib/files';
  import Histogram from '../components/Histogram.svelte';
  import EditorMountInset from '../lib/EditorMountInset.svelte';
  import Viewer, { type CanvasPoint } from '../lib/canvas/Viewer.svelte';
  import type { View } from '../lib/canvas/view';
  import type { Core } from '../lib/core/core';
  import type { AreaMeta, TileOverride } from '../lib/core/types';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  let viewer = $state<ReturnType<typeof Viewer>>();
  const imageEdits=$derived(areaImageEdits(app.project,app.areaId));
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  const imageLut=$derived(levelsLut(lut,imageEdits));

  let thumbs = $state<Record<string, string>>({});
  let tileSel = $state<number | null>(null);
  let hist = $state<Uint32Array | null>(null);
  let showResiduals = $state(true);
  let showTiles = $state(false);

  const published = $derived(!!app.remote);
  let waitingTone = $state(neutralTone());
  const bundle = $derived(bundlePixels(app));
  const remoteTiles = $derived(!bundle ? app.remoteTiles : null);
  const tiles = $derived(remoteTiles ?? core);
  const areas = $derived<AreaMeta[]>(app.pkg?.areas ?? (published ? (app.project?.areas ?? []).map(a => ({ id: a.id, name: a.name, pixel_size_nm: a.pixel_size_nm, stage_sign: a.stage_sign, tiles: [], grid: null, field_um: null, pitch_um: null, meta: {} })) : []));
  const meta = $derived(areas.find(a => a.id === app.areaId) ?? null);
  const areaIdx = $derived(app.pkg?.areas.findIndex((a) => a.id === app.areaId) ?? -1);
  const params = $derived(app.areaId ? app.params[app.areaId] : null);
  const tone = $derived(app.areaId ? (app.tone[app.areaId] ?? waitingTone) : null);
  const summary = $derived(app.areaId ? app.stitched[app.areaId] : null);
  const placement = $derived(summary?.placement ?? null);
  const compare = $derived(app.areaId ? (app.compare[app.areaId] ?? null) : null);
  const areaGrains = $derived(app.areaId ? (app.grains[app.areaId] ?? []) : []);
  const umPerPx = $derived((meta?.pixel_size_nm ?? 1000) / 1000);
  const viewScale = $derived(summary?.mosaic.um_per_px ?? umPerPx);
  const viewKey = $derived(`${meta?.id}/${summary ? 'mosaic' : 'tiles'}`);
  let viewport = $state<{ key: string; view: View }>();
  const view = $derived(viewport?.key === viewKey ? viewport?.view : undefined);

  function setView(key: string, next: View | undefined) {
    if (key === viewKey && next) viewport = { key, view: next };
  }
  function changeArea(id: string) {
    app.areaId = id;
    tileSel = null;
  }

  async function findGrains(id: string) {
    const state = current();
    if (await findGrainsInArea(state, id)) state.screen = 'canvas';
  }

  // Tile frames in mosaic pixels: the solved placement if stitched, else the prior.
  const boxes = $derived.by(() => {
    if (!meta) return [];
    return meta.tiles.map((t, i) => {
      const p = placement?.tiles[i];
      if (p) return { i, x: p.x, y: p.y, w: p.w, h: p.h };
      const w = t.w || (meta.field_um?.[0] ?? 100) / umPerPx;
      const h = t.h || (meta.field_um?.[1] ?? 100) / umPerPx;
      return { i, x: t.pos_um[0] / umPerPx, y: t.pos_um[1] / umPerPx, w, h };
    });
  });
  const frame = $derived.by(() => {
    if (summary) return { w: summary.mosaic.w, h: summary.mosaic.h };
    if (!boxes.length) return { w: 1, h: 1 };
    return { w: Math.max(...boxes.map((b) => b.x + b.w)), h: Math.max(...boxes.map((b) => b.y + b.h)) };
  });

  $effect(() => {
    const t = tone;
    if (!t || !core) return;
    core.toneLut($state.snapshot(t)).then((l) => (lut = l));
  });

  function tileClick(p: CanvasPoint) {
    const b = [...boxes].reverse().find((q) => p.x >= q.x && p.x <= q.x + q.w && p.y >= q.y && p.y <= q.y + q.h);
    tileSel = b ? b.i : null;
  }

  $effect(() => {
    const m = meta, idx = areaIdx;
    if (!m || idx < 0) return;
    (async () => {
      const core = await app.core;
      for (const t of m.tiles) {
        const k = `${m.id}/${t.index}`;
        if (thumbs[k]) continue;
        try {
          const img = await core.tileThumb(idx, t.index, 240);
          thumbs = { ...thumbs, [k]: rawToCanvas(img).toDataURL('image/jpeg', 0.8) };
        } catch {
          /* unreadable tile: leave blank */
        }
      }
    })();
  });

  $effect(() => {
    const id = app.areaId;
    app.pixelsVersion;
    if (!id || (published && !bundle) || !app.stitched[id]) {
      hist = null;
      return;
    }
    app.core.then((c) => c.histogram(id)).then((h) => (hist = h)).catch(() => (hist = null));
  });

  function override(i: number): TileOverride | undefined {
    return params?.photometric.overrides.find((o) => o.index === i);
  }
  function setOverride(i: number, patch: Partial<TileOverride>) {
    if (!params || !summary) return;
    const list = params.photometric.overrides.filter((o) => o.index !== i);
    const cur = override(i) ?? { index: i, gain: summary.photometric.gain[i], offset: summary.photometric.offset[i], no_background: false };
    list.push({ ...cur, ...patch });
    params.photometric.overrides = list;
  }
  function matchNeighbours(i: number) {
    if (!params) return;
    params.photometric.overrides = params.photometric.overrides.filter((o) => o.index !== i);
  }

  function residualColor(r: number, used: boolean) {
    if (!used) return 'var(--text-faint)';
    return r < 0.5 ? 'var(--ok)' : r < 1 ? 'var(--warning)' : 'var(--danger)';
  }
  const fmt = (v: number, d = 2) => (Number.isFinite(v) ? v.toFixed(d) : '–');
</script>

{#if meta && params && tone}
  <div class="area">
    <aside class="glass list">
      <h3>Areas</h3>
      <ul>
        {#each areas as a (a.id)}
          <li>
            <button class="arearow" aria-pressed={a.id === app.areaId} onclick={() => changeArea(a.id)} data-testid="area-{a.id}">
              <span class="nm">{a.name}{#if app.grains[a.id] === undefined} <span class="faint" title="no grains yet" aria-label="no grains yet">○</span>{/if}</span>
              <span class="faint mono">{a.id}</span>
              {#if !published}<span class="faint">{a.grid ? `${a.grid[0]}×${a.grid[1]}` : ''} · {a.tiles.length} tiles</span>{/if}
              {#if app.stitched[a.id]}<span class="pill ok">{published ? 'published' : 'stitched'}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if app.pkg && !published}
      <button class="primary wide" onclick={() => stitchAll()} disabled={!!app.busy}>Stitch all areas</button>
      {#if app.pkg.navigation}<p class="faint small">Navigation image and stage calibration found.</p>{/if}
      <p class="faint small" data-testid="open-kind">
        {#if app.pkg.kind === 'area'}One area folder; scale and stage positions from its sidecars.
        {:else if app.pkg.kind === 'loose'}Loose images{app.project?.package.loose ? `, ${scaleText(app.project.package.loose)}` : ''}. <button class="link" onclick={changeScale} disabled={!!app.busy}>Change…</button>
        {:else}Package ({app.pkg.reader}).{/if}
      </p>
      {/if}
      {#if published}<p class="faint small">Published image from the database. Stitching and shading correction need the raw package.</p>{/if}
    </aside>

    <section class="glass preview">
      <div class="head">
        <h2>{meta.name} <span class="muted mono">{meta.id}</span></h2>
        <div class="actions">
          {#if !published}<label class="check"><input type="checkbox" bind:checked={showTiles} /> tiles</label>
          <label class="check"><input type="checkbox" bind:checked={showResiduals} /> residuals</label>
          <button class="primary" onclick={() => stitch(meta.id)} disabled={!!app.busy} data-testid="stitch">{summary ? 'Re-stitch' : 'Stitch'}</button>{/if}
          <button onclick={() => (app.screen = 'canvas')} disabled={!summary} data-testid="to-canvas">Open canvas</button>
          {#if summary && !areaGrains.length && !app.readOnly && (!app.remote || bundle)}
            <button onclick={() => findGrains(meta.id)} disabled={!!app.busy} data-testid="area-find-grains">Find grains</button>
          {/if}
        </div>
      </div>
      <div class="grid" data-testid="area-view" data-live={app.liveBusy[meta.id] ? 'busy' : 'idle'}>
        {#if tiles}
          {#key viewKey}
            {@const sourceKey = viewKey}
            <Viewer bind:this={viewer} bind:view={() => view, next => setView(sourceKey, next)} core={tiles} areaId={meta.id} mosaic={summary?.mosaic ?? null} extent={frame} lut={imageLut} {imageEdits} umPerPx={viewScale} version={app.pixelsVersion}
              preview={app.preview[meta.id] ?? null} onlevel={(l) => (app.drawLevel = l)} onclick={tileClick}>
              {#snippet overlay(v)}
                {#each boxes as b (b.i)}
                  {@const failed = placement?.tiles[b.i]?.failed ?? summary?.failed_tiles?.includes(b.i) ?? false}
                  <g class="tile" class:sel={tileSel === b.i} class:failed>
                    {#if failed}<title>tile {b.i}: blank capture, left out</title>{/if}
                    {#if !summary && thumbs[`${meta.id}/${b.i}`]}
                      <image href={thumbs[`${meta.id}/${b.i}`]} x={b.x} y={b.y} width={b.w} height={b.h} opacity="0.92" />
                    {/if}
                    {#if showTiles || !summary || failed}<rect x={b.x} y={b.y} width={b.w} height={b.h} fill="none" />{/if}
                    {#if showTiles || !summary}<text x={b.x + 8 / v.zoom} y={b.y + 18 / v.zoom} font-size={13 / v.zoom}>{b.i}</text>{/if}
                  </g>
                {/each}
                {#if showResiduals && placement}
                  {#each placement.pairs as p, k (k)}
                    {@const A = boxes[p.a]}
                    {@const B = boxes[p.b]}
                    <line x1={A.x + A.w / 2} y1={A.y + A.h / 2} x2={B.x + B.w / 2} y2={B.y + B.h / 2} stroke={residualColor(p.residual, p.used)} stroke-width="3" stroke-dasharray={p.used ? '' : '8 6'} opacity="0.9">
                      <title>{p.a}–{p.b}: {p.used ? `residual ${fmt(p.residual)} px, PSR ${fmt(p.psr, 0)}` : `prior (PSR ${fmt(p.psr, 0)})`}</title>
                    </line>
                  {/each}
                {/if}
              {/snippet}
            </Viewer>
          {/key}
        {/if}
        {#if app.rendering}<span class="pill rendering" role="status">rendering…</span>{/if}
        {#if tiles}
          <EditorMountInset current={meta.id} {tiles} {lut} {view} frame={summary ? undefined : { ...frame, um_per_px: viewScale }}
            allAreasClickable currentPictureOnly={!!remoteTiles} onarea={changeArea} />
        {/if}
        <div class="viewbtns glass" data-ui data-mount-inset-bottom>
          <button class="icon" title="Zoom in" onclick={() => viewer?.zoomBy(1.5)}>+</button>
          <button class="icon" title="Zoom out" onclick={() => viewer?.zoomBy(1 / 1.5)}>−</button>
          <button class="icon" title="Fit (double-click)" onclick={() => viewer?.fitAll()}>Fit</button>
        </div>
      </div>
      {#if summary}
        <div class="stats">
          <span>Mosaic <b class="num">{summary.mosaic.w}×{summary.mosaic.h}</b> px · <b class="num">{fmt((summary.mosaic.w * summary.mosaic.um_per_px) / 1000)}×{fmt((summary.mosaic.h * summary.mosaic.um_per_px) / 1000)}</b> mm</span>
          {#if !published}<span>Pairs <b class="num">{placement?.pairs.filter((p) => p.used).length}/{placement?.pairs.length}</b> measured</span>
          <span>Residual RMS <b class="num" data-testid="rms">{fmt(placement?.rms_residual ?? 0)}</b> px, max <b class="num">{fmt(placement?.max_residual ?? 0)}</b> px</span>
          <span>Overlap RMS <b class="num">{fmt(summary.photometric.overlap_rms_before * 255, 1)} → {fmt(summary.photometric.overlap_rms_after * 255, 1)}</b> grey levels</span>
          {#if summary.photometric.fallback}<div class="faint small">{summary.photometric.fallback}</div>{/if}
          {/if}<span class="faint timings" title={summary.timings.map(([n, t]) => `${n} ${Math.round(t)} ms`).join(' · ')}>{summary.timings.map(([n, t]) => `${n} ${Math.round(t)} ms`).join(' · ')}</span>
        </div>
      {/if}
    </section>

    <aside class="glass side">
      <fieldset disabled={published && !bundle} inert={published && !bundle} aria-label="Image controls">
      <section>
        <h3>Tone</h3>
        <div class="row3">
          <button class="primary" onclick={() => autoTone(meta.id)} disabled={!summary || !!app.busy} title={bundle ? "Stretch and CLAHE chosen from the published image" : "Background leveling on, stretch and CLAHE chosen from the histogram"} data-testid="auto-tone">Auto</button>
          <button aria-pressed={compare?.showing === 'before'} disabled={!compare} onclick={() => toggleBeforeAfter(meta.id)} title="Swap between the result of Auto and what was there before" data-testid="before-after">{compare?.showing === 'before' ? 'Before' : 'After'} ⇄</button>
          <button onclick={() => resetTone(meta.id)} title={bundle ? "Back to the published image" : "Correction, blending and tone back to the defaults"} data-testid="reset-tone">Reset</button>
        </div>
        <Histogram bins={hist} bind:black={tone.black} bind:white={tone.white} gamma={tone.gamma} />
        <div class="row3">
          <label class="field">Black <input type="number" step="0.01" min="0" max="1" bind:value={tone.black} /></label>
          <label class="field">White <input type="number" step="0.01" min="0" max="1" bind:value={tone.white} /></label>
          <label class="field">Gamma <input type="number" step="0.05" min="0.1" max="5" bind:value={tone.gamma} /></label>
        </div>
        <label class="field">Gamma <input type="range" min="0.2" max="3" step="0.01" bind:value={tone.gamma} /></label>
        <div class="row3">
          <label class="field">Low % <input type="number" step="0.1" min="0" max="50" bind:value={tone.low_pct} data-testid="low-pct" /></label>
          <label class="field">High % <input type="number" step="0.1" min="50" max="100" bind:value={tone.high_pct} /></label>
          <label class="check"><input type="checkbox" bind:checked={tone.clahe} data-testid="clahe" /> CLAHE</label>
        </div>
        {#if tone.clahe}
          <div class="row3">
            <label class="field">Tile px <input type="number" min="16" step="16" bind:value={tone.clahe_tile} /></label>
            <label class="field">Clip <input type="number" min="1" step="0.5" bind:value={tone.clahe_clip} /></label>
          </div>
        {/if}
        <button onclick={() => Object.assign(tone, { black: 0, white: 1, gamma: 1 })}>Reset levels</button>
      </section>

      <section>
        <h3>Per-grain tone</h3>
        <label class="check"><input type="checkbox" bind:checked={tone.grain_norm.enabled} disabled={!areaGrains.length && !tone.grain_norm.enabled} data-testid="grain-norm" /> equalize grains (median and spread)</label>
        {#if !areaGrains.length}<p class="faint small">Find grains on the Canvas first; then each grain's tone can be matched to the area or to one grain.</p>{/if}
        {#if tone.grain_norm.enabled}
          <label class="field">Strength {Math.round(tone.grain_norm.strength * 100)} %
            <input type="range" min="0" max="1" step="0.05" bind:value={tone.grain_norm.strength} data-testid="grain-norm-strength" /></label>
          <div class="row3">
            <label class="field">Match to
              <select bind:value={tone.grain_norm.reference}>
                <option value={null}>the area</option>
                {#each areaGrains as g (g.id)}<option value={g.id}>grain {grainNumber(g)}</option>{/each}
              </select>
            </label>
            <label class="field">Soft edge µm <input type="number" min="0" step="0.5" bind:value={tone.grain_norm.feather_um} /></label>
          </div>
        {/if}
      </section>

      {#if !published}<section>
        <h3>Correction</h3>
        <label class="field">Gain / offset
          <select bind:value={params.photometric.model}>
            <option value="affine">gain and offset (SE, CL)</option>
            <option value="gain">gain only (photocraft)</option>
            <option value="none">none</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={params.photometric.background} data-testid="background" /> background leveling</label>
        <label class="check"><input type="checkbox" bind:checked={params.photometric.background_params.edge_steps} /> remove beam-exposure steps</label>
        <label class="field">Blending
          <select bind:value={params.blend.mode}>
            <option value="multiband">seams + multiband</option>
            <option value="feather">feather (fast preview)</option>
          </select>
        </label>
        <h3>Stitching <span class="faint">(Re-stitch to apply)</span></h3>
        <label class="field">Min. peak-to-sidelobe <input type="number" step="0.5" bind:value={params.stitch.min_psr} /></label>
      </section>

      <section>
        <h3>Tile {tileSel ?? '–'}</h3>
        {#if tileSel !== null}
          {@const t = meta.tiles[tileSel]}
          {@const o = override(tileSel)}
          <table class="data kv">
            <tbody>
              <tr><td>file</td><td class="mono">{t.file.split('/').pop()}</td></tr>
              <tr><td>stage</td><td class="num">{t.stage_nm ? `${(t.stage_nm[0] / 1e6).toFixed(4)}, ${(t.stage_nm[1] / 1e6).toFixed(4)} mm` : '–'}</td></tr>
              <tr><td>pixel</td><td class="num">{t.pixel_size_nm} nm</td></tr>
              {#if placement}<tr><td>drift</td><td class="num">{fmt(placement.tiles[tileSel].drift[0], 1)}, {fmt(placement.tiles[tileSel].drift[1], 1)} px</td></tr>{/if}
              {#if summary}<tr><td>auto gain / offset</td><td class="num">{fmt(summary.photometric.auto_gain[tileSel], 3)} / {fmt(summary.photometric.auto_offset[tileSel], 3)}</td></tr>{/if}
            </tbody>
          </table>
          {#if summary}
            <label class="field">Gain {fmt(o?.gain ?? summary.photometric.gain[tileSel], 3)}
              <input type="range" min="0.5" max="1.5" step="0.005" value={o?.gain ?? summary.photometric.gain[tileSel]} oninput={(e) => setOverride(tileSel!, { gain: +e.currentTarget.value })} /></label>
            <label class="field">Offset {fmt(o?.offset ?? summary.photometric.offset[tileSel], 3)}
              <input type="range" min="-0.2" max="0.2" step="0.002" value={o?.offset ?? summary.photometric.offset[tileSel]} oninput={(e) => setOverride(tileSel!, { offset: +e.currentTarget.value })} /></label>
            <div class="row3">
              <button onclick={() => matchNeighbours(tileSel!)} disabled={!o}>Match neighbours</button>
            </div>
          {/if}
          <details>
            <summary class="muted">All metadata ({Object.keys(t.meta).length})</summary>
            <table class="data kv"><tbody>{#each Object.entries(t.meta) as [k, v] (k)}<tr><td>{k}</td><td class="mono">{v}</td></tr>{/each}</tbody></table>
          </details>
        {:else}
          <p class="muted small">Click a tile in the view for its metadata and manual gain and offset.</p>
          <details>
            <summary class="muted">Area metadata ({Object.keys(meta.meta).length})</summary>
            <table class="data kv"><tbody>{#each Object.entries(meta.meta) as [k, v] (k)}<tr><td>{k}</td><td class="mono">{v}</td></tr>{/each}</tbody></table>
          </details>
        {/if}
      </section>
      {/if}<section>
        <button onclick={() => exportPng(meta.id)} disabled={!summary}>Export mosaic PNG</button>
      </section>
      </fieldset>
    </aside>
  </div>
{/if}

<style>
  fieldset { border:0; margin:0; padding:0; min-width:0; }
  .area {
    display: grid;
    grid-template-columns: 15rem minmax(0, 1fr) 20rem;
    gap: var(--space-2);
    height: 100%;
    padding: 0 var(--space-2) var(--space-2);
  }
  aside,
  .preview {
    min-height: 0;
    overflow: auto;
    padding: var(--space-3);
  }
  ul {
    list-style: none;
    padding: 0;
    margin: var(--space-2) 0;
    display: grid;
    gap: var(--space-1);
  }
  .arearow {
    width: 100%;
    display: grid;
    text-align: left;
    gap: 0;
  }
  .nm {
    font-weight: 600;
  }
  .wide {
    width: 100%;
  }
  .preview {
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: var(--space-2);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }
  .head h2 { min-width: 0; overflow-wrap: anywhere; }
  .actions {
    margin-left: auto;
    display: flex;
    min-width: 0;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }
  .grid {
    position: relative;
    min-height: 0;
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--image-bg);
  }
  .grid :global(.tile rect) {
    stroke: var(--ov-outline);
    stroke-width: 1.5;
    opacity: 0.7;
  }
  .grid :global(.tile.sel rect) {
    stroke: var(--ov-number);
    stroke-width: 2.5;
    opacity: 1;
  }
  .grid :global(.tile.failed rect) {
    stroke: var(--warning);
    stroke-width: 2.5;
    opacity: 1;
  }
  .grid :global(.tile text) {
    fill: var(--ov-number);
    font-family: var(--font-mono);
  }
  .viewbtns {
    position: absolute;
    right: var(--space-2);
    bottom: var(--space-2);
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border-radius: var(--radius-sm);
  }
  .rendering {
    position: absolute;
    left: var(--space-2);
    top: var(--space-2);
  }
  /* Measured values wrap between complete values, so no number is cut off; each row keeps room
     for two lines, so the view keeps its size when the numbers change. The timings line (full
     text in its title) stays on one line. */
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(16rem, 100%), 1fr));
    grid-auto-rows: minmax(2.6em, auto);
    gap: var(--space-1) var(--space-4);
    font-size: 0.85rem;
    line-height: 1.3;
  }
  .stats .num { white-space: nowrap; }
  .stats > .timings {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .side section {
    display: grid;
    gap: var(--space-2);
    padding-bottom: var(--space-3);
    margin-bottom: var(--space-3);
    border-bottom: 1px solid var(--hairline);
  }
  .row3 {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-2);
    align-items: end;
  }
  .kv td:first-child {
    color: var(--text-muted);
    white-space: nowrap;
  }
  .kv td {
    word-break: break-all;
  }
  .small {
    font-size: 0.8rem;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font-size: inherit;
  }
</style>
