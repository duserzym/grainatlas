<script lang="ts">
  // Laser registration of one area (roadmap phase 11): the operator drives the laser to three or
  // more spots spread over the area and types the stage position the laser software shows; the
  // core fits an affine map mosaic µm → stage (least squares) and writes the spot list in the
  // laser's units. Stored in project.json under `laser`.
  import { app } from '../lib/store.svelte';
  import type { Core } from '../lib/core/core';
  import type { AreaRegistration, Fiducial, LaserAdapterInfo, LaserFit, Spot } from '../lib/core/types';
  import { download } from '../lib/files';

  let { core, areaId, spots, selected }: { core: Core; areaId: string; spots: Spot[]; selected: string[] } = $props();

  const stored = $derived<AreaRegistration | null>(app.project?.laser?.areas?.[areaId] ?? null);
  let units = $state<'mm' | 'um'>('mm');
  let adapter = $state('generic');
  let rows = $state<Fiducial[]>([]);
  let fit = $state<LaserFit | null>(null);
  let problem = $state<string | null>(null);
  let adapters = $state<LaserAdapterInfo[]>([{ id: 'generic', label: 'Generic CSV', extension: 'csv' }]);
  let loadedFor = '';

  $effect(() => {
    core.laserAdapters().then((a) => (adapters = a)).catch(() => {});
  });
  // Load the stored registration when the area (or the project) changes.
  $effect(() => {
    const key = `${app.project?.uuid}/${areaId}`;
    if (key === loadedFor) return;
    loadedFor = key;
    const r = stored;
    units = r?.units ?? 'mm';
    adapter = r?.adapter ?? 'generic';
    rows = r ? r.fiducials.map((f) => ({ ...f })) : [];
    fit = r?.fit ?? null;
    problem = null;
  });

  const complete = $derived(rows.filter((f) => f.x_stage != null && f.y_stage != null && Number.isFinite(f.x_stage) && Number.isFinite(f.y_stage)));
  const unitLabel = $derived(units === 'mm' ? 'mm' : 'µm');

  /** Current positions of the fiducial spots (spots may have moved since). */
  function current(): Fiducial[] {
    return rows.map((f) => {
      const s = spots.find((x) => x.id === f.spot_id);
      return s ? { ...f, name: s.name, x_um: s.x_um, y_um: s.y_um } : f;
    });
  }

  async function changed() {
    const fids = current();
    rows = fids;
    let next: LaserFit | null = null;
    problem = null;
    if (complete.length >= 3) {
      try {
        next = await core.laserFit($state.snapshot(fids) as Fiducial[], units);
      } catch (e) {
        problem = e instanceof Error ? e.message : String(e);
      }
    } else if (rows.length) problem = `${complete.length} of at least 3 fiducials have stage coordinates.`;
    fit = next;
    const reg: AreaRegistration | null = rows.length ? { units, adapter, fiducials: $state.snapshot(fids) as Fiducial[], fit: next } : null;
    await core.setLaser(areaId, reg);
    app.project = JSON.parse(await core.projectJson());
    app.persist();
  }

  function add(spotId?: string) {
    const used = new Set(rows.map((f) => f.spot_id));
    const s = spots.find((x) => x.id === spotId) ?? spots.find((x) => !used.has(x.id));
    rows = [...rows, { spot_id: s?.id ?? null, name: s?.name ?? '', x_um: s?.x_um ?? 0, y_um: s?.y_um ?? 0, x_stage: null, y_stage: null }];
    void changed();
  }
  function setSpot(i: number, id: string) {
    const s = spots.find((x) => x.id === id);
    if (!s) return;
    rows[i] = { ...rows[i], spot_id: s.id, name: s.name, x_um: s.x_um, y_um: s.y_um };
    void changed();
  }
  function setStage(i: number, k: 'x_stage' | 'y_stage', v: string) {
    const n = v.trim() === '' ? null : Number(v.replace(',', '.'));
    rows[i] = { ...rows[i], [k]: n != null && Number.isFinite(n) ? n : null };
    void changed();
  }
  function remove(i: number) {
    rows = rows.filter((_, j) => j !== i);
    void changed();
  }
  /** The residual of row i (rows with both stage values are fitted in order). */
  function residual(i: number): number | null {
    if (!fit) return null;
    const k = complete.indexOf(rows[i]);
    return k >= 0 ? (fit.residuals_um[k] ?? null) : null;
  }

  async function exportList() {
    const a = adapters.find((x) => x.id === adapter) ?? adapters[0];
    const text = await app.run('Writing the spot list', () => core.laserExport($state.snapshot(app.spots) as Spot[], areaId, adapter));
    if (text) download(text, `${app.project?.package.name ?? 'grainatlas'}_${areaId}_laser_${a.id}.${a.extension}`, 'text/csv');
  }
</script>

<div class="laser" data-testid="laser-panel">
  <p class="faint small">Drive the laser to three or more spots spread over this area (fiducials) and type the stage position the laser software shows for each. Four or more let the fit check the readings.</p>
  <div class="two">
    <label class="field">Stage units
      <select bind:value={units} onchange={changed} data-testid="laser-units"><option value="mm">mm</option><option value="um">µm</option></select>
    </label>
    <label class="field">Laser software
      <select bind:value={adapter} onchange={changed} data-testid="laser-adapter">
        {#each adapters as a (a.id)}<option value={a.id}>{a.label}</option>{/each}
      </select>
    </label>
  </div>
  <table class="data fids">
    <thead><tr><th>Spot</th><th>x ({unitLabel})</th><th>y ({unitLabel})</th><th title="Residual of the fit">±µm</th><th></th></tr></thead>
    <tbody>
      {#each rows as f, i (i)}
        {@const r = residual(i)}
        <tr data-testid="fiducial">
          <td>
            <select value={f.spot_id ?? ''} onchange={(e) => setSpot(i, e.currentTarget.value)} aria-label="Fiducial spot {i + 1}">
              {#if !f.spot_id}<option value="">pick a spot</option>{/if}
              {#each spots as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
            </select>
          </td>
          <td><input type="text" inputmode="decimal" value={f.x_stage ?? ''} onchange={(e) => setStage(i, 'x_stage', e.currentTarget.value)} aria-label="Stage x {i + 1}" data-testid="fid-x" /></td>
          <td><input type="text" inputmode="decimal" value={f.y_stage ?? ''} onchange={(e) => setStage(i, 'y_stage', e.currentTarget.value)} aria-label="Stage y {i + 1}" data-testid="fid-y" /></td>
          <td class="num" class:bad={r != null && r > 5}>{r == null ? '–' : fit?.exact ? '0' : r.toFixed(1)}</td>
          <td><button class="icon" onclick={() => remove(i)} aria-label="Remove fiducial {i + 1}">✕</button></td>
        </tr>
      {:else}
        <tr><td colspan="5" class="muted">No fiducials yet.</td></tr>
      {/each}
    </tbody>
  </table>
  <div class="row">
    <button onclick={() => add()} disabled={!spots.length} data-testid="add-fiducial">+ Fiducial</button>
    <button onclick={() => add(selected[0])} disabled={!selected.length || !spots.some((s) => s.id === selected[0])}>Use selected spot</button>
  </div>
  {#if problem}
    <p class="small muted" data-testid="laser-problem">{problem}</p>
  {:else if fit}
    <div class="fit" class:warn={fit.warn} data-testid="laser-fit">
      {#if fit.exact}
        <b>3 fiducials fit exactly.</b> Add a fourth to check the stage readings.
      {:else}
        RMS <b class="num">{fit.rms_um.toFixed(2)} µm</b>, max <b class="num">{fit.max_um.toFixed(2)} µm</b> over {fit.n} fiducials.
        {#if fit.warn}<div class="warntext" data-testid="laser-warn">A residual is above 5 µm: check the stage readings and that each fiducial is the right spot.</div>{/if}
      {/if}
      <div class="faint small">scale {(fit.scale[0] * (units === 'mm' ? 1000 : 1)).toFixed(4)} {unitLabel} per {units === 'mm' ? 'mm' : 'µm'} · rotation {fit.rotation_deg.toFixed(2)}°{fit.mirrored ? ' · y flipped' : ''}</div>
    </div>
  {/if}
  <button class="primary" onclick={exportList} disabled={!fit} data-testid="laser-export"
    title={fit ? 'All spots of this area in the laser stage frame' : 'Fit a registration first'}>Export spot list</button>
  {#if stored}<p class="faint small">Saved in the project; spots added later are included in the next export.</p>{/if}
</div>

<style>
  .laser {
    display: grid;
    gap: var(--space-2);
    min-height: 0;
    overflow: auto;
  }
  .small {
    font-size: 0.8rem;
    margin: 0;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }
  .row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .fids select,
  .fids input {
    width: 100%;
    min-width: 0;
  }
  .fids td:nth-child(2),
  .fids td:nth-child(3) {
    width: 5.5rem;
  }
  .bad {
    color: var(--danger);
    font-weight: 600;
  }
  .fit {
    font-size: 0.85rem;
    padding: var(--space-2);
    border: 1px solid var(--hairline);
    border-radius: 6px;
  }
  .fit.warn {
    border-color: var(--danger);
  }
  .warntext {
    color: var(--danger);
    margin-top: var(--space-1);
  }
</style>
