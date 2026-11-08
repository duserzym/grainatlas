<script lang="ts">
  // Canvas screen: the infinite canvas with grains and spots.
  // Pointer-first: navigate with trackpad or mouse, click to place, drag a spot
  // to move it, drag its ring handle to resize, double-click its name to rename;
  // the floating panel edits the hovered or selected object. Keys are extras.
  import { app, current, type Overlay, type Tool } from '../lib/store.svelte';
  import Viewer, { type CanvasPoint } from '../lib/canvas/Viewer.svelte';
  import { toMosaic, toScreen, type View } from '../lib/canvas/view';
  import { onDestroy } from 'svelte';
  import { spotContrast, type Contrast } from '../lib/canvas/contrast';
  import type { Core } from '../lib/core/core';
  import { ARTIFACT_LABELS, type ArtifactReason, type EditOp, type Grain, type GrainOwner, type Label, type SegmentParams, type Spot, type SpotKind } from '../lib/core/types';
  import { download } from '../lib/files';
  import { formatName, grainAt, newId, nextN, snap, snapTargets, spotDistance, stageOf, type SnapTarget } from '../lib/spots';
  import FloatingPanel from '../components/FloatingPanel.svelte';
  import { isHosted, open as openLink } from '@lab/bridge';
  import { mountSpotDocuments, postSpots, spotLink } from '../lib/laicpmsdb';
  import MountPicker from '../components/MountPicker.svelte';
  import LaserPanel from '../components/LaserPanel.svelte';
  import { authToken, canSaveFolder, publish, saveAtlas } from '../lib/atlas';
  import { fmtValue, matchResults, metricsOf, rampColor, RAMP_CSS, valueOf, type SpotResult } from '../lib/results';
  import { refreshResults } from '../lib/results-load';
  import { beforeCheckedAction, parseLabels, runChecks } from '../lib/checks';
  import type { Finding } from '../lib/core/types';
  import { CHECK_TITLES } from '../lib/check-presentation';
  import { reviewSpots } from '../lib/store.svelte';
  import { acquisitionPage } from '../lib/laicpmsdb';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  const areaId = $derived(app.areaId && app.stitched[app.areaId] ? app.areaId : (Object.keys(app.stitched)[0] ?? null));
  let viewer = $state<ReturnType<typeof Viewer>>();
  // The screen is made again on a tab switch: the tab keeps its view and gets it back.
  let viewport = $state<{ area: string; view: View } | null>(
    app.savedView ? { area: app.savedView.areaId, view: app.savedView.view } : null,
  );
  const view = $derived(viewport?.area === areaId ? viewport?.view : undefined);
  function setAreaView(id: string, next: View | undefined) {
    if (id !== areaId || !next) return;
    viewport = { area: id, view: next };
    app.savedView = { areaId: id, view: $state.snapshot(next) as View };
  }
  function changeArea(id: string) {
    if (id !== app.areaId) app.areaId = id;
  }
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  let cursorScreen = $state<[number, number] | null>(null);
  const cursorUm = $derived(cursorScreen && view ? toMosaic(view, ...cursorScreen).map((x) => x * umPerPx) as [number, number] : null);
  let selGrains = $state<string[]>([]);
  let tab = $state<'spots' | 'grains' | 'refOverlay' | 'link' | 'laser' | 'checks'>('spots');
  $effect(() => {
    if (app.checksRequested) { tab = 'checks'; app.checksRequested = false; }
  });
  let labelInput = $state<HTMLInputElement>();
  let checksBusy = $state(false);
  const checkGroups = $derived([...new Set(app.findings.map((f) => f.code))].map((code) => ({ code, rows: app.findings.filter((f) => f.code === code) })));
  async function checkNow(refreshLabels = false) {
    checksBusy = true;
    try { await runChecks(current(), refreshLabels); } catch (e) { app.error = `Checks: ${e instanceof Error ? e.message : e}`; }
    finally { checksBusy = false; }
  }
  async function loadLabels(file: File) {
    app.checkLabels = parseLabels(await file.text());
    await checkNow();
  }
  function selectFinding(f: Finding) {
    const selected = app.spots.filter((s) => f.spot_ids.includes(s.id));
    app.selection = selected.map((s) => s.id);
    if (!selected.length) return;
    const first = selected[0];
    if (first.area_id !== areaId) changeArea(first.area_id);
    // Resolve after the new area's viewer is ready; the effect below frames all spots there.
    pendingFinding = first.area_id;
  }
  let pendingFinding = $state<string | null>(null);
  $effect(() => {
    if (!pendingFinding || pendingFinding !== areaId || !viewer || !view) return;
    const ss = app.spots.filter((s) => app.selection.includes(s.id) && s.area_id === areaId);
    pendingFinding = null;
    if (!ss.length) return;
    const xs = ss.flatMap((s) => [Math.min(s.x_um, s.x2_um ?? s.x_um) - s.diameter_um/2, Math.max(s.x_um, s.x2_um ?? s.x_um) + s.diameter_um/2]);
    const ys = ss.flatMap((s) => [Math.min(s.y_um, s.y2_um ?? s.y_um) - s.diameter_um/2, Math.max(s.y_um, s.y2_um ?? s.y_um) + s.diameter_um/2]);
    const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
    viewer.centerOn((x0+x1)/2/umPerPx, (y0+y1)/2/umPerPx, Math.min(view.w/Math.max(100, (x1-x0)/umPerPx), view.h/Math.max(100, (y1-y0)/umPerPx)) * 0.65);
  });
  async function applySuggestions(rows: Finding[]) {
    const state = current();
    if (state.readOnly) return;
    const requested = new Map(rows.filter((f) => f.code === 'near_miss' && f.label).map((f) => [f.spot_ids[0], f.label!]));
    // A spot or label may have changed since the displayed run. Revalidate before renaming.
    const latest = await state.run('Matching acquisition labels', () => runChecks(state));
    if (!latest) return;
    const suggestions = new Map(latest.filter((f) => f.code === 'near_miss' && f.label === requested.get(f.spot_ids[0])).map((f) => [f.spot_ids[0], f.label!]));
    if (!suggestions.size) return;
    state.checkpoint();
    state.spots = state.spots.map((s) => suggestions.has(s.id) ? { ...s, name: suggestions.get(s.id)! } : s);
    await state.run('Checking renamed spots', () => runChecks(state));
  }
  function patternFor(id: string) { return app.settings.area_patterns?.[id]?.trim() ? app.settings.area_patterns[id] : app.settings.naming_pattern; }
  function setAreaPattern(value: string) {
    if (!areaId) return;
    app.settings.area_patterns = { ...app.settings.area_patterns, [areaId]: value };
    app.persist();
  }
  let pairPending = $state<[number, number] | null>(null);
  let overlayInput = $state<HTMLInputElement>();
  // Open state per visit; the position is remembered (app.layout, ux-conventions rule 7).
  let panel = $state({ open: true });
  let hoverId = $state<string | null>(null);
  let snapHint = $state<SnapTarget | null>(null);
  // In-progress gestures.
  let splitLine = $state<[number, number, number, number] | null>(null);
  let drawing = $state<{ kind: SpotKind | 'measure'; x: number; y: number; x2: number; y2: number } | null>(null);
  let measure = $state<[number, number, number, number] | null>(null);
  let gesture = $state.raw<{ mode: 'move' | 'resize'; ids: string[]; startX: number; startY: number; orig: Map<string, { x: number; y: number; x2: number | null; y2: number | null; d: number }>; dirty: boolean } | null>(null);
  let editing = $state<{ id: string; text: string; sx: number; sy: number } | null>(null);
  // Hand editing of outlines: the lasso (a freehand drag, or click by click) and the brush.
  let lasso = $state<{ pts: [number, number][]; op: EditOp; free: boolean; clicks: number; dragged: boolean } | null>(null);
  let lassoHover = $state<[number, number] | null>(null);
  let stroke = $state<{ pts: [number, number][]; op: EditOp } | null>(null);
  let snapEdges = $state(true);
  let brushUm = $state(20);
  // A typed "Next n" belongs to the sample it was typed for.
  let nextNSample: string | null = null;
  $effect(() => {
    const s = sample;
    if (nextNSample !== null && s !== nextNSample) app.nextN = null;
    nextNSample = s;
  });
  $effect(() => {
    if (app.tool !== 'grain-outline') lasso = null;
    if (app.tool !== 'grain-brush') stroke = null;
  });

  /** A bundle from a server shown read-only; `noPixels`: no stitched mosaic in the core (server tiles). */
  const ro = $derived(app.readOnly);
  const noPixels = $derived(!!app.remote);
  const tiles = $derived(app.remote && app.remoteTiles ? app.remoteTiles : core);
  const area = $derived(app.project?.areas.find((a) => a.id === areaId) ?? null);
  const mosaic = $derived(areaId ? app.stitched[areaId].mosaic : null);
  const umPerPx = $derived(mosaic?.um_per_px ?? 1);
  /** Every detection of the area: grains and flagged artifacts. */
  const detections = $derived(areaId ? (app.grains[areaId] ?? []) : []);
  const grains = $derived(detections.filter((g) => !g.artifact));
  const artifacts = $derived(detections.filter((g) => !!g.artifact));
  const artifactCounts = $derived(Object.entries(artifacts.reduce((m, g) => ({ ...m, [g.artifact!]: (m[g.artifact!] ?? 0) + 1 }), {} as Record<string, number>)).map(([k, v]) => `${k} ${v}`).join(', '));
  const reviewVisible = $derived.by(() => {
    const project = app.project?.uuid;
    const ids = project && Object.hasOwn(reviewSpots.projects, project) ? reviewSpots.projects[project] : null;
    return ids ? new Set(ids) : null;
  });
  const spots = $derived(app.spots.filter((s) => s.area_id === areaId && (!reviewVisible || reviewVisible.has(s.id))));
  const outlinedGrains = $derived(reviewVisible
    ? grains.filter(g => spots.some(s => s.grain_number === g.number))
    : grains);
  const labels = $derived(app.labels.filter((l) => l.area_id === areaId));
  const segParams = $derived(areaId ? app.params[areaId]?.segment : null);
  /** Owner of each detection here: a grain also seen in an overlapping area belongs to the area it lies deepest in. */
  const owners = $derived.by(() => {
    const m = new Map<string, GrainOwner>();
    for (const o of app.ownership?.grains ?? []) if (o.area_id === areaId) m.set(o.grain_id, o);
    return m;
  });
  const dupCount = $derived(grains.filter((g) => owners.get(g.id)?.duplicate_of).length);
  $effect(() => {
    void app.grains;
    app.refreshOwnership();
  });
  function areaLabel(id: string) {
    return app.pkg?.areas.find((a) => a.id === id)?.name ?? id;
  }
  /** "duplicate of HH4 #12" for a grain owned by another area. */
  function ownerNote(g: Grain): string {
    const o = owners.get(g.id);
    if (!o?.duplicate_of) return '';
    const og = app.grains[o.owner_area]?.find((q) => q.id === o.duplicate_of);
    return `duplicate of ${areaLabel(o.owner_area)} #${og?.number ?? '?'}`;
  }
  const sample = $derived(area?.sample || area?.name || '');
  const refOverlay = $derived(areaId ? (app.overlays[areaId] ?? null) : null);
  /** screen px → um at the current zoom */
  const k = $derived(view ? umPerPx / view.zoom : 1);
  const rotDeg = $derived(view ? (view.rot * 180) / Math.PI : 0);
  const selSpots = $derived(app.spots.filter((s) => app.selection.includes(s.id)));
  const activeIds = $derived(gesture?.ids ?? []);
  const autoColour = $derived(app.settings.spot_cursor_colour === 'auto');
  let contrasts = $state<Record<string, Contrast>>({});
  let contrastFrame = 0;
  // Store the latest geometry; rapid pointer events collapse into one frame.
  const contrastRings = $derived([
    ...(app.tool === 'spot' && cursorUm && !snapHint ? [{ id: 'ghost', x: cursorUm[0], y: cursorUm[1], r: app.diameter / 2 }] : []),
    ...spots.filter((s) => activeIds.includes(s.id)).map((s) => ({ id: s.id, x: s.x_um, y: s.y_um, r: s.diameter_um / 2 })),
  ]);
  function scheduleContrast() {
    if (!contrastFrame) contrastFrame = requestAnimationFrame(() => {
      contrastFrame = 0;
      if (!autoColour || !view || !viewer) return;
      const next: Record<string, Contrast> = {};
      for (const ring of contrastRings) {
        const [sx, sy] = toScreen(view, ring.x / umPerPx, ring.y / umPerPx);
        next[ring.id] = spotContrast(viewer.luminanceAt(sx, sy, ring.r / k), contrasts[ring.id]);
      }
      contrasts = next;
    });
  }
  $effect(() => {
    contrastRings; view; viewer; autoColour;
    scheduleContrast();
  });
  onDestroy(() => cancelAnimationFrame(contrastFrame));
  function activeColour(id: string) {
    return autoColour ? (contrasts[id] === 'dark' ? '#111' : '#fff') : app.settings.spot_cursor_colour;
  }
  function activeHalo(id: string) {
    return activeColour(id) === '#111' ? '#fff' : '#111';
  }
  const focus = $derived.by(() => {
    const id = app.selection[0] ?? hoverId;
    return app.spots.find((s) => s.id === id) ?? null;
  });
  // ---- results overlay (GET /v1/mounts/{id}/results; memory only) ------------------------------
  const mountId = $derived(app.mountId);
  const resultsBy = $derived<Record<string, SpotResult>>(app.results?.data ? matchResults(app.spots, app.results.data) : {});
  const metrics = $derived(app.results?.data ? metricsOf(app.results.data.spots) : ['date']);
  const shown = $derived(spots.filter((s) => resultsBy[s.id]).map((s) => ({ s, r: resultsBy[s.id], v: valueOf(resultsBy[s.id], app.colourBy) })));
  const range = $derived.by(() => {
    const vs = shown.map((x) => x.v?.value).filter((v): v is number => v != null && Number.isFinite(v));
    if (!vs.length) return null;
    const lo = Math.min(...vs), hi = Math.max(...vs);
    return { lo, hi, unit: shown.find((x) => x.v?.unit)?.v?.unit ?? '' };
  });
  const tOf = (v: number | undefined) => (range && v != null ? (range.hi > range.lo ? (v - range.lo) / (range.hi - range.lo) : 0.5) : 0);
  // Fetch once per server and mount; Refresh fetches again.
  $effect(() => {
    const key = app.server && mountId ? `${app.server}|${mountId}` : null;
    if (!key) {
      if (app.results) app.results = null;
      return;
    }
    if (app.results?.key !== key) void refreshResults();
  });
  const laserOf = (s: Spot) => app.project?.laser?.areas?.[s.area_id]?.fit ?? null;
  async function openAcquisition(id: string) {
    if (await openLink(`laicpmsdb://acquisition/${encodeURIComponent(id)}`)) return;
    // Not inside UPb Lab: laicpmsdb's own page, when the server is known.
    if (app.server) window.open(acquisitionPage(app.server, id), '_blank', 'noopener');
    else app.notice = `Acquisition ${id}: open it in laicpmsdb (no server set here).`;
  }

  const focusGrain = $derived(!focus ? (detections.find((g) => g.id === (selGrains[0] ?? hoverId)) ?? null) : null);

  $effect(() => {
    const t = areaId ? app.tone[areaId] : null;
    if (!t || !core) return;
    const snap = $state.snapshot(t);
    core.toneLut(snap).then((l) => (lut = l));
  });
  $effect(() => {
    if (app.diameter <= 0) app.diameter = app.settings.default_diameter_um;
  });

  const scalebar = $derived.by(() => {
    if (!view) return null;
    const target = 140 * k;
    const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];
    const len = steps.filter((s) => s <= target).pop() ?? 1;
    return { len, px: len / k };
  });

  // ---- grains -----------------------------------------------------------------
  async function findGrains() {
    if (!core || !areaId || !segParams) return;
    const id = areaId;
    const params = $state.snapshot(segParams) as SegmentParams;
    const prev = $state.snapshot(app.grains[id] ?? []) as Grain[];
    const r = await app.run('Finding grains', () => core!.segment(id, params, prev, { margin_um: app.settings.outline_margin_um, smooth: app.settings.outline_smooth }));
    if (!r) return;
    app.checkpoint();
    app.grains = { ...app.grains, [id]: r };
    relinkSpots(id);
  }
  /** Replaces the area's grains; flagged artifacts stay unless `all` is the whole list. */
  function setGrains(gs: Grain[], all = false) {
    if (!areaId) return;
    app.checkpoint();
    app.grains = { ...app.grains, [areaId]: all ? gs : [...gs, ...artifacts.filter((a) => !gs.some((g) => g.id === a.id))] };
    relinkSpots(areaId);
  }
  /** Keeps a flagged detection as a grain (the next number); re-runs keep it too. */
  function keepAsGrain(id: string) {
    const n = Math.max(0, ...grains.map((g) => g.number)) + 1;
    setGrains(detections.map((g) => (g.id === id ? { ...g, artifact: undefined, kept: true, number: n } : g)), true);
  }
  /** Marks the selected grains as artifacts (unnumbered, left off sheets); re-runs keep the mark. */
  function markArtifact() {
    if (!selGrains.length) return;
    setGrains(detections.map((g) => (selGrains.includes(g.id) ? { ...g, artifact: 'manual' as ArtifactReason, kept: false, number: 0 } : g)), true);
  }
  /** Spots follow the grain under them (number changes, merges, splits). Names are kept. */
  function relinkSpots(id: string) {
    const gs = app.grains[id] ?? [];
    app.spots = app.spots.map((s) => {
      if (s.area_id !== id) return s;
      const g = grainAt(gs, s.x_um, s.y_um);
      return { ...s, grain_id: g?.id ?? null, grain_number: g?.number ?? null };
    });
  }
  async function refine() {
    if (!core || !areaId) return;
    const id = areaId;
    const out: Grain[] = [];
    for (const g of grains.filter((g) => selGrains.includes(g.id))) {
      const r = await app.run('Refining grain', () => core!.refineGrain(id, $state.snapshot(g) as Grain));
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
    setGrains(detections.filter((g) => !selGrains.includes(g.id)), true);
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
    setGrains(await core.renumber($state.snapshot(grains) as Grain[], segParams.row_tolerance, from));
  }
  async function exportGrains() {
    if (!core || !areaId) return;
    download(await core.exportGrains(areaId, $state.snapshot(grains) as Grain[]), `${areaId}_grains.csv`, 'text/csv');
  }
  function centerOnUm(x: number, y: number, wUm = 200) {
    if (view) viewer?.centerOn(x / umPerPx, y / umPerPx, Math.max(view.zoom, Math.min(view.w, view.h) / (wUm / umPerPx) / 3));
  }

  // A deep link asked for a spot: center on it once the viewer has a size.
  $effect(() => {
    const id = app.focusSpot;
    if (!id || !view || !viewer) return;
    const s = app.spots.find((x) => x.id === id);
    if (!s || s.area_id !== areaId) return;
    app.focusSpot = null;
    if (app.shellSelection?.spot === id) tab = 'spots';
    selGrains = [];
    panel.open = true;
    centerOnUm(s.x_um, s.y_um);
  });

  // Shell grain commands use the same selection and framing as a grain-table click.
  $effect(() => {
    const target = app.focusGrain;
    if (!target || target.area !== areaId || !view || !viewer) return;
    const g = detections.find((g) => g.id === target.id);
    if (!g) return;
    app.focusGrain = null;
    selGrains = [g.id];
    tab = 'grains';
    panel.open = true;
    const [x0, y0, x1, y1] = g.bbox_um;
    centerOnUm((x0 + x1) / 2, (y0 + y1) / 2, Math.max(x1 - x0, y1 - y0, 1));
    if (target.flash) app.flash = { kind: 'grain', id: g.id, area: areaId! };
  });
  const viewReady = $derived(!!view && !!viewer);
  $effect(() => {
    const flash = app.flash;
    if (!flash || flash.area !== areaId || !viewReady) return;
    const state = current();
    const timer = setTimeout(() => { if (state.flash === flash) state.flash = null; }, 1500);
    return () => clearTimeout(timer);
  });
  const flashingSpot = $derived(app.flash?.kind === 'spot' && app.flash.area === areaId ? spots.find((s) => s.id === app.flash?.id) : null);
  const flashingGrain = $derived(app.flash?.kind === 'grain' && app.flash.area === areaId ? detections.find((g) => g.id === app.flash?.id) : null);

  // ---- laicpmsdb (through UPb Lab) -------------------------------------------
  const hosted = isHosted();
  async function openInLaicpmsdb(s: Spot) {
    const url = spotLink(s);
    const ok = await openLink(url);
    app.error = ok ? null : `UPb Lab did not open ${url}`;
  }
  /**
   * Spot sync: with a server, a mount and a token (the shell's, or one pasted in Settings) the
   * spots go straight to POST /v1/mounts/{mount}/spots and the acquisitions laicpmsdb matched by
   * label come back as the spots' acquisition ids; otherwise the same body is downloaded.
   */
  async function sendSpots() {
    const state = current();
    if (!state.project || !(await beforeCheckedAction(state))) return;
    const { documents, skipped } = mountSpotDocuments(state.project, $state.snapshot(state.spots) as Spot[]);
    const count = documents.reduce((n, d) => n + d.spots.length, 0);
    const mount = state.project.laicpmsdb.mount_id;
    const server = state.server;
    const tok = server && mount ? await authToken(state) : null;
    if (!server || !mount || !tok) {
      download(JSON.stringify({ project_id: state.project.uuid, documents }, null, 2), `${state.pkg?.name ?? state.project.package.name}_laicpmsdb-spots.json`, 'application/json');
      state.notice = `${count} spots from all areas, ${documents.length} samples for ${mount ? `mount ${mount}` : 'laicpmsdb (no mount id set)'}${skipped ? `; ${skipped} lines and rasters left out` : ''}. Downloaded: ${!server ? 'no server set' : !mount ? 'no mount linked' : 'no token'}.`;
      return;
    }
    const responses = await state.run(`Sending ${count} spots from all areas`, async () => {
      const results = [];
      for (const doc of documents) results.push(await postSpots(server, tok, mount, doc));
      return results;
    });
    if (!responses) return;
    const linked = new Map(responses.flatMap((r) => r.acquisitions ?? []).filter((a) => a.atlas_spot_id).map((a) => [a.atlas_spot_id.toLowerCase(), a.id]));
    if (linked.size) {
      state.checkpoint();
      state.spots = state.spots.map((s) => (linked.has(s.id.toLowerCase()) && !s.acquisition_id ? { ...s, acquisition_id: linked.get(s.id.toLowerCase())! } : s));
    }
    const sum = (key: 'created' | 'updated' | 'unchanged' | 'grains_created') => responses.reduce((n, r) => n + r[key], 0);
    state.notice = `Sent to mount ${mount}: ${sum('created')} created, ${sum('updated')} updated, ${sum('unchanged')} unchanged, ${sum('grains_created')} grains created; ${count} spots from all areas (${documents.length} samples)${linked.size ? `; ${linked.size} acquisitions linked` : ''}${skipped ? `; ${skipped} lines and rasters left out` : ''}.`;
  }

  // ---- spots --------------------------------------------------------------------
  function now() {
    return new Date().toISOString();
  }
  function makeSpot(x: number, y: number, kind: SpotKind = 'spot', x2: number | null = null, y2: number | null = null): Spot {
    const g = grainAt(grains, x, y);
    const n = app.nextN ?? nextN(app.spots, sample);
    if (app.nextN != null) app.nextN = n + 1;
    return {
      id: newId(),
      area_id: areaId!,
      name: formatName(patternFor(areaId!), sample, g?.number ?? null, n),
      sample,
      grain_id: g?.id ?? null,
      grain_number: g?.number ?? null,
      n,
      x_um: x,
      y_um: y,
      x2_um: x2,
      y2_um: y2,
      diameter_um: app.diameter,
      kind,
      comment: '',
      created: now(),
      created_by: app.settings.user,
    };
  }
  function addSpot(s: Spot) {
    app.checkpoint();
    app.spots = [...app.spots, s];
    app.selection = [s.id];
  }
  function updateSpot(id: string, patch: Partial<Spot>, checkpoint = true) {
    if (checkpoint) app.checkpoint();
    app.spots = app.spots.map((s) => (s.id === id ? { ...s, ...patch } : s));
  }
  /** Overwrites a spot's number and renames it from the pattern; a clash in the sample is reported. */
  function setSpotN(id: string, n: number) {
    const s = app.spots.find((q) => q.id === id);
    if (!s || !n) return;
    const name = formatName(patternFor(s.area_id), s.sample || sample, s.grain_number ?? null, n);
    updateSpot(id, { n, name });
    const clash = app.spots.find((q) => q.id !== id && q.sample === s.sample && q.n === n);
    if (clash) app.notice = `${clash.name} has number ${n} too.`;
  }
  function deleteSelection() {
    if (!app.selection.length) return false;
    app.checkpoint();
    app.spots = app.spots.filter((s) => !app.selection.includes(s.id));
    app.labels = app.labels.filter((l) => !app.selection.includes(l.id));
    app.selection = [];
    return true;
  }
  async function renameFromPattern(ids: string[]) {
    const state = current();
    if (state.renamePrompt || state.readOnly) return;
    const projectId = state.project?.uuid;
    const changed = app.spots.filter((s) => ids.includes(s.id) && s.sample !== currentSample(s.area_id));
    const useCurrent = new Set<string>();
    for (const old of [...new Set(changed.map((s) => `${s.area_id}\n${s.sample}`))]) {
      const s = changed.find((s) => `${s.area_id}\n${s.sample}` === old)!;
      const choice = await new Promise<'use' | 'keep' | 'cancel'>((resolve) => {
        state.renamePrompt = { oldSample: s.sample, newSample: currentSample(s.area_id), answer: (choice) => {
          state.renamePrompt = null;
          resolve(choice);
        } };
      });
      if (choice === 'cancel' || state.project?.uuid !== projectId || current() !== state) return;
      if (choice === 'use') useCurrent.add(old);
    }
    app.checkpoint();
    app.spots = app.spots.map((s) => {
      if (!ids.includes(s.id)) return s;
      const g = grainAt(app.grains[s.area_id] ?? [], s.x_um, s.y_um);
      const sample = useCurrent.has(`${s.area_id}\n${s.sample}`) ? currentSample(s.area_id) : s.sample;
      return { ...s, sample, grain_id: g?.id ?? null, grain_number: g?.number ?? null, name: formatName(patternFor(s.area_id), sample, g?.number ?? null, s.n) };
    });
    const clashes = app.spots.filter((s, i, ss) => ss.some((q, j) => i !== j && (q.name === s.name || (q.sample === s.sample && q.n === s.n))));
    if (clashes.length) app.notice = `Naming clashes: ${clashes.map((s) => s.name).join(', ')}`;
  }
  function currentSample(id: string) {
    const a = app.project?.areas.find((a) => a.id === id);
    return a?.sample || a?.name || '';
  }
  async function exportSpots(format: 'csv' | 'geojson') {
    if (!core) return;
    const text = await app.run(`Exporting ${format}`, () => core!.exportSpots($state.snapshot(app.spots) as Spot[], $state.snapshot(app.grains) as Record<string, Grain[]>, format));
    if (text) download(text, `${app.pkg?.name ?? 'grainatlas'}_spots.${format === 'csv' ? 'csv' : 'geojson'}`, format === 'csv' ? 'text/csv' : 'application/geo+json');
  }
  async function setSample(v: string) {
    if (!area || !core) return;
    area.sample = v;
    await core.setLink(app.project?.laicpmsdb.mount_id ?? null, Object.fromEntries((app.project?.areas ?? []).map((a) => [a.id, a.sample])));
    app.persist();
  }
  async function setMount(v: string) {
    if (!core || !app.project) return;
    const id = v.trim() || null;
    try {
      await core.setLink(id, {});
      app.project.laicpmsdb.mount_id = id;
      app.persist();
    } catch (e) {
      app.error = `Mount id must be a UUID: ${e instanceof Error ? e.message : e}`;
    }
  }

  // ---- reference refOverlay (printout photo, post-laser frame) -------------------------
  async function loadOverlay(file: File) {
    if (!areaId || !mosaic) return;
    const url = URL.createObjectURL(file);
    const im = new Image();
    try {
      await new Promise<void>((res, rej) => {
        im.onload = () => res();
        im.onerror = () => rej(new Error(`cannot read ${file.name} as an image`));
        im.src = url;
      });
    } catch (e) {
      app.error = e instanceof Error ? e.message : String(e);
      return;
    }
    const prev = app.overlays[areaId];
    const keep = prev && prev.name === file.name && prev.w === im.naturalWidth && prev.h === im.naturalHeight;
    // First placement: the whole image over the whole mosaic.
    const W = mosaic.w * umPerPx, H = mosaic.h * umPerPx;
    const k0 = Math.min(W / im.naturalWidth, H / im.naturalHeight);
    const o: Overlay = keep
      ? { ...prev, url }
      : { name: file.name, url, w: im.naturalWidth, h: im.naturalHeight, pairs: [], matrix: [k0, 0, 0, k0, 0, 0], rms_um: 0, opacity: 0.6, visible: true };
    app.overlays = { ...app.overlays, [areaId]: o };
    tab = 'refOverlay';
  }
  function invAffine(m: number[]): number[] | null {
    const det = m[0] * m[3] - m[2] * m[1];
    if (Math.abs(det) < 1e-15) return null;
    const a = m[3] / det, b = -m[1] / det, c = -m[2] / det, d = m[0] / det;
    return [a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])];
  }
  async function pairClick(x: number, y: number) {
    if (!areaId || !refOverlay || !core) return;
    if (!pairPending) {
      const inv = invAffine(refOverlay.matrix);
      if (!inv) return;
      pairPending = [inv[0] * x + inv[2] * y + inv[4], inv[1] * x + inv[3] * y + inv[5]];
      return;
    }
    const pairs = $state.snapshot([...refOverlay.pairs, [pairPending, [x, y]]]) as [[number, number], [number, number]][];
    pairPending = null;
    await refit(pairs);
  }
  async function refit(pairs: [[number, number], [number, number]][]) {
    if (!areaId || !refOverlay || !core) return;
    const r = pairs.length ? await core.register(pairs) : null;
    const base = app.overlays[areaId];
    app.overlays = { ...app.overlays, [areaId]: { ...base, pairs, matrix: r ? r.matrix : base.matrix, rms_um: r?.rms_um ?? 0, residuals_um: r?.residuals_um ?? [] } };
    app.persist();
  }
  function setOverlay(patch: Partial<Overlay>) {
    if (!areaId || !refOverlay) return;
    app.overlays = { ...app.overlays, [areaId]: { ...refOverlay, ...patch } };
  }

  // ---- hit testing (screen pixels) -------------------------------------------------
  function hitSpot(xUm: number, yUm: number): Spot | null {
    // Inside a shape scores by distance to its centre (0..1), outside by 1 + edge distance in
    // screen px (within 8 px of slack): overlapping spots resolve to the nearest centre.
    let best: Spot | null = null;
    let bs = Infinity;
    for (const s of spots) {
      const d = spotDistance(s, xUm, yUm);
      let score: number;
      if (d <= 0) {
        const cx = s.kind === 'spot' ? s.x_um : (s.x_um + (s.x2_um ?? s.x_um)) / 2;
        const cy = s.kind === 'spot' ? s.y_um : (s.y_um + (s.y2_um ?? s.y_um)) / 2;
        score = Math.hypot(xUm - cx, yUm - cy) / Math.max(s.diameter_um / 2, 1e-6) / 1e3;
      } else if (d <= 8 * k) score = 1 + d / k;
      else continue;
      if (score < bs) {
        bs = score;
        best = s;
      }
    }
    return best;
  }
  function hitHandle(xUm: number, yUm: number): Spot | null {
    for (const s of selSpots) {
      if (s.area_id !== areaId || s.kind !== 'spot') continue;
      const r = s.diameter_um / 2;
      const hx = s.x_um + r * Math.SQRT1_2, hy = s.y_um + r * Math.SQRT1_2;
      if (Math.hypot(xUm - hx, yUm - hy) <= 9 * k) return s;
    }
    return null;
  }
  function hitLabel(xUm: number, yUm: number): Label | null {
    return labels.find((l) => Math.abs(xUm - l.x_um) < 60 * k && yUm < l.y_um + 4 * k && yUm > l.y_um - 16 * k) ?? null;
  }
  function snapped(x: number, y: number, ev: MouseEvent | PointerEvent): [number, number] {
    snapHint = null;
    if (!app.settings.snap || ev.altKey) return [x, y];
    const prev = app.spots.filter((s) => s.area_id === areaId).at(-1) ?? null;
    const t = snap(x, y, snapTargets(grains, prev, app.diameter), 12 * k);
    if (!t) return [x, y];
    snapHint = t;
    return [t.x, t.y];
  }

  // ---- pointer --------------------------------------------------------------------
  function press(p: CanvasPoint): boolean {
    const x = p.x * umPerPx, y = p.y * umPerPx;
    const ev = p.ev as PointerEvent;
    if (ro && app.tool !== 'measure') {
      // Read-only: a press selects a spot, nothing moves.
      const s = hitSpot(x, y);
      if (!s) return false;
      app.selection = [s.id];
      panel.open = true;
      return true;
    }
    if (app.tool === 'grain-split') {
      splitLine = [x, y, x, y];
      return true;
    }
    if (app.tool === 'grain-outline') {
      if (!lasso) {
        lasso = { pts: [[x, y]], op: ev.altKey ? 'cut' : ev.shiftKey ? 'add' : 'new', free: ev.metaKey || ev.ctrlKey, clicks: 1, dragged: false };
      } else {
        // Click by click: a click on the first point closes the outline.
        const [fx, fy] = lasso.pts[0];
        if (lasso.pts.length > 2 && Math.hypot(x - fx, y - fy) < 8 * k) {
          void finishLasso();
          return true;
        }
        lasso.pts.push([x, y]);
        lasso.clicks += 1;
      }
      return true;
    }
    if (app.tool === 'grain-brush') {
      stroke = { pts: [[x, y]], op: ev.altKey ? 'cut' : 'add' };
      return true;
    }
    if (app.tool === 'line' || app.tool === 'raster' || app.tool === 'measure') {
      const [sx, sy] = app.tool === 'measure' ? [x, y] : snapped(x, y, ev);
      drawing = { kind: app.tool === 'measure' ? 'measure' : (app.tool as SpotKind), x: sx, y: sy, x2: sx, y2: sy };
      return true;
    }
    const h = hitHandle(x, y);
    if (h) {
      gesture = { mode: 'resize', ids: [h.id], startX: x, startY: y, orig: new Map([[h.id, { x: h.x_um, y: h.y_um, x2: h.x2_um, y2: h.y2_um, d: h.diameter_um }]]), dirty: false };
      return true;
    }
    const s = hitSpot(x, y);
    if (s) {
      if (app.tool === 'delete') {
        app.selection = [s.id];
        deleteSelection();
        return true;
      }
      if (ev.shiftKey) app.selection = app.selection.includes(s.id) ? app.selection.filter((i) => i !== s.id) : [...app.selection, s.id];
      else if (!app.selection.includes(s.id)) app.selection = [s.id];
      const ids = app.selection.filter((i) => app.spots.some((q) => q.id === i));
      gesture = { mode: 'move', ids, startX: x, startY: y, orig: new Map(app.spots.filter((q) => ids.includes(q.id)).map((q) => [q.id, { x: q.x_um, y: q.y_um, x2: q.x2_um, y2: q.y2_um, d: q.diameter_um }])), dirty: false };
      return true;
    }
    const l = hitLabel(x, y);
    if (l) {
      if (app.tool === 'delete') {
        app.selection = [l.id];
        deleteSelection();
        return true;
      }
      app.selection = [l.id];
      gesture = { mode: 'move', ids: [l.id], startX: x, startY: y, orig: new Map([[l.id, { x: l.x_um, y: l.y_um, x2: null, y2: null, d: 0 }]]), dirty: false };
      return true;
    }
    return false;
  }

  function drag(p: CanvasPoint) {
    const x = p.x * umPerPx, y = p.y * umPerPx;
    if (splitLine) {
      splitLine = [splitLine[0], splitLine[1], x, y];
      return;
    }
    if (lasso) {
      const [lx, ly] = lasso.pts[lasso.pts.length - 1];
      if (Math.hypot(x - lx, y - ly) >= 2 * k) {
        lasso.pts.push([x, y]);
        lasso.dragged = true;
      }
      return;
    }
    if (stroke) {
      const [lx, ly] = stroke.pts[stroke.pts.length - 1];
      if (Math.hypot(x - lx, y - ly) >= Math.max(2 * k, brushUm / 8)) stroke.pts.push([x, y]);
      return;
    }
    if (drawing) {
      const [sx, sy] = drawing.kind === 'measure' ? [x, y] : snapped(x, y, p.ev);
      drawing = { ...drawing, x2: sx, y2: sy };
      return;
    }
    if (!gesture) return;
    if (!gesture.dirty) {
      app.checkpoint();
      gesture.dirty = true;
    }
    if (gesture.mode === 'resize') {
      const id = gesture.ids[0];
      const o = gesture.orig.get(id)!;
      const d = Math.max(1, Math.round(2 * Math.hypot(x - o.x, y - o.y) * 2) / 2);
      app.spots = app.spots.map((s) => (s.id === id ? { ...s, diameter_um: d } : s));
      app.diameter = d;
      return;
    }
    let dx = x - gesture.startX, dy = y - gesture.startY;
    if (gesture.ids.length === 1) {
      const o = gesture.orig.get(gesture.ids[0])!;
      const [sx, sy] = snapped(o.x + dx, o.y + dy, p.ev);
      dx = sx - o.x;
      dy = sy - o.y;
    }
    const ids = gesture.ids;
    const orig = gesture.orig;
    app.spots = app.spots.map((s) => {
      const o = orig.get(s.id);
      if (!o || !ids.includes(s.id)) return s;
      return { ...s, x_um: o.x + dx, y_um: o.y + dy, x2_um: o.x2 == null ? null : o.x2 + dx, y2_um: o.y2 == null ? null : o.y2 + dy };
    });
    app.labels = app.labels.map((l) => {
      const o = orig.get(l.id);
      return o ? { ...l, x_um: o.x + dx, y_um: o.y + dy } : l;
    });
  }

  async function release() {
    snapHint = null;
    if (lasso) {
      // A single freehand drag closes on release; click by click it stays open.
      if (lasso.clicks === 1 && lasso.dragged && lasso.pts.length > 4) await finishLasso();
      return;
    }
    if (stroke) {
      const st = stroke;
      stroke = null;
      await finishStroke(st);
      return;
    }
    if (splitLine) {
      const line = splitLine;
      splitLine = null;
      await doSplit(line);
      return;
    }
    if (drawing) {
      const d = drawing;
      drawing = null;
      const len = Math.hypot(d.x2 - d.x, d.y2 - d.y);
      if (d.kind === 'measure') {
        measure = [d.x, d.y, d.x2, d.y2];
        return;
      }
      if (len < 2) return;
      if (d.kind === 'raster') addSpot(makeSpot(Math.min(d.x, d.x2), Math.min(d.y, d.y2), 'raster', Math.max(d.x, d.x2), Math.max(d.y, d.y2)));
      else addSpot(makeSpot(d.x, d.y, 'line', d.x2, d.y2));
      return;
    }
    if (gesture?.dirty) {
      // Re-link moved spots to the grain they now sit on (names are kept).
      const ids = gesture.ids;
      app.spots = app.spots.map((s) => {
        if (!ids.includes(s.id)) return s;
        const g = grainAt(app.grains[s.area_id] ?? [], s.x_um, s.y_um);
        return { ...s, grain_id: g?.id ?? null, grain_number: g?.number ?? null };
      });
      app.persist();
    }
    gesture = null;
  }

  async function doSplit(line: [number, number, number, number]) {
    if (!core || !areaId) return;
    const mid: [number, number] = [(line[0] + line[2]) / 2, (line[1] + line[3]) / 2];
    const g = grainAt(grains, ...mid) ?? grains.find((q) => selGrains.includes(q.id));
    if (!g) return;
    const id = areaId;
    const parts = await app.run('Splitting grain', () => core!.splitGrain(id, $state.snapshot(g) as Grain, line));
    if (!parts || parts.length < 2) return;
    // The largest grain part keeps the grain; a part the core recognised as a bubble becomes a
    // bubble artifact (one cut detaches a bubble), the others get new numbers.
    const real = parts.filter((q) => !q.artifact).sort((a, b) => b.area_um2 - a.area_um2);
    const flagged = parts.filter((q) => q.artifact).map((q) => ({ ...q, number: 0 }));
    if (!real.length) return;
    let max = Math.max(0, ...grains.map((q) => q.number));
    const kept = real.map((q, i) => (i === 0 ? { ...q, id: g.id, number: g.number } : { ...q, number: ++max }));
    setGrains([...grains.filter((q) => q.id !== g.id), ...kept, ...flagged].sort((a, b) => a.number - b.number));
    if (flagged.length) app.notice = `Split off ${flagged.length === 1 ? 'a bubble' : `${flagged.length} bubbles`}: flagged as not a grain (Keep as grain restores it).`;
  }

  async function click(p: CanvasPoint) {
    const ev = p.ev as MouseEvent;
    const x = p.x * umPerPx, y = p.y * umPerPx;
    if (ro && app.tool !== 'select' && app.tool !== 'measure') return;
    if (app.tool === 'grain-outline' || app.tool === 'grain-brush') return;
    if (app.tool === 'pair') {
      await pairClick(x, y).catch((e) => (app.error = e instanceof Error ? e.message : String(e)));
      return;
    }
    if (app.tool === 'spot') {
      const [sx, sy] = snapped(x, y, ev);
      snapHint = null;
      addSpot(makeSpot(sx, sy));
      return;
    }
    if (app.tool === 'label') {
      const l: Label = { id: newId(), area_id: areaId!, x_um: x, y_um: y, text: 'label' };
      app.checkpoint();
      app.labels = [...app.labels, l];
      app.selection = [l.id];
      startEdit(l.id);
      return;
    }
    if (app.tool === 'delete') {
      // Spots and labels went in press(); here the click is on a grain (or nothing).
      const g = grainAt(detections, x, y, app.layers.artifacts);
      if (g) {
        selGrains = [g.id];
        deleteGrains();
        app.notice = `Deleted ${g.artifact ? 'a flagged detection' : 'the grain'} (⌘Z brings it back).`;
      }
      return;
    }
    if (app.tool === 'grain-add' && core && areaId && segParams) {
      const id = areaId;
      const g = await app.run('Growing grain', () => core!.grainAt(id, x, y, $state.snapshot(segParams) as SegmentParams, { margin_um: app.settings.outline_margin_um, smooth: app.settings.outline_smooth }));
      if (!g) {
        if (g === null) app.error = 'No grain at that point (background).';
        return;
      }
      const added = { ...g, number: Math.max(0, ...grains.map((q) => q.number)) + 1 };
      setGrains([...grains.filter((q) => !(q.centroid_um && added.polygon_um.length && inside(added.polygon_um, q.centroid_um[0], q.centroid_um[1]))), added]);
      selGrains = [added.id];
      return;
    }
    // Select / other tools on empty space: select a grain or clear.
    app.selection = [];
    measure = null;
    const g = grainAt(detections, x, y, app.layers.artifacts);
    const multi = ev.shiftKey || ev.metaKey;
    if (!g) {
      if (!multi) selGrains = [];
      return;
    }
    selGrains = multi ? (selGrains.includes(g.id) ? selGrains.filter((s) => s !== g.id) : [...selGrains, g.id]) : [g.id];
  }

  // ---- hand-edited outlines -------------------------------------------------------------
  async function finishLasso() {
    const l = lasso;
    lasso = null;
    lassoHover = null;
    if (!l || !core || !areaId || l.pts.length < 3) return;
    const id = areaId;
    let poly = $state.snapshot(l.pts) as [number, number][];
    if (snapEdges && !l.free) {
      // Edges within ~10 screen pixels of the drawn path pull the outline onto them.
      const snapped = await app.run('Snapping to edges', () => core!.snapOutline(id, poly, Math.max(3, 10 * k), 0.08));
      if (snapped && snapped.length >= 3) poly = snapped;
    }
    await applyEdit(l.op, poly[0], (base, op) => core!.outlineGrain(id, poly, base, op, snapEdges && !l.free && op !== 'cut' ? { margin_um: app.settings.outline_margin_um, smooth: app.settings.outline_smooth } : { margin_um: 0, smooth: false }), 'Outlining grain');
  }
  async function finishStroke(st: { pts: [number, number][]; op: EditOp }) {
    if (!core || !areaId) return;
    const id = areaId;
    const pts = $state.snapshot(st.pts) as [number, number][];
    const size = brushUm;
    await applyEdit(st.op, pts[0], (base, op) => core!.brushGrain(id, pts, size, base, op), st.op === 'cut' ? 'Erasing' : 'Painting grain');
  }
  /**
   * Applies a drawn outline or brush stroke. The grain edited is the one under the start of the
   * stroke, else the selected one; with neither, Add makes a new grain. A new or grown outline
   * takes in the grains whose centres it covers (fragments of one grain), keeping the id and
   * number of the first of them so its spots stay linked.
   */
  async function applyEdit(op: EditOp, at: [number, number], run: (base: Grain | null, op: EditOp) => Promise<Grain | null>, label: string) {
    const sel = selGrains.length === 1 ? (detections.find((g) => g.id === selGrains[0]) ?? null) : null;
    const base = op === 'new' ? null : (grainAt(detections, at[0], at[1], true) ?? sel ?? null);
    if (op === 'cut' && !base) {
      app.error = 'Select the grain to cut from, or start the cut on it.';
      return;
    }
    const eff: EditOp = op === 'add' && !base ? 'new' : op;
    const r = await app.run(label, () => run(base ? ($state.snapshot(base) as Grain) : null, eff));
    if (r === undefined) return;
    if (!r) {
      app.notice = eff === 'cut' ? 'That cut would remove the whole grain: use Delete for that.' : 'Nothing to outline there.';
      return;
    }
    const g: Grain = { ...r };
    const covered = eff === 'cut' ? [] : detections.filter((q) => q.id !== g.id && q.id !== base?.id && inside(g.polygon_um, q.centroid_um[0], q.centroid_um[1]));
    if (eff === 'new') {
      const keep = covered.filter((q) => !q.artifact).sort((a, b) => a.number - b.number)[0];
      if (keep) {
        g.id = keep.id;
        g.number = keep.number;
      } else g.number = Math.max(0, ...grains.map((q) => q.number)) + 1;
    }
    const gone = new Set(covered.map((q) => q.id));
    setGrains([...detections.filter((q) => q.id !== g.id && !gone.has(q.id)), g].sort((a, b) => a.number - b.number), true);
    selGrains = [g.id];
    const what = eff === 'new' ? (covered.length ? 'Redrew the grain' : 'New grain') : eff === 'add' ? 'Grew the grain' : 'Cut the grain';
    app.notice = `${what}${covered.length > 1 ? ` (took in ${covered.length} pieces)` : ''}. ⌘Z undoes.`;
  }

  function inside(poly: [number, number][], px: number, py: number) {
    let c = false;
    for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
      const [ax, ay] = poly[i], [bx, by] = poly[j];
      if (ay > py !== by > py && px < ((bx - ax) * (py - ay)) / (by - ay) + ax) c = !c;
    }
    return c;
  }

  /** Double-click a spot or label to rename it; elsewhere the view fits (gestures.ts). */
  function dblclick(p: CanvasPoint): boolean {
    if (lasso) {
      void finishLasso();
      return true;
    }
    const x = p.x * umPerPx, y = p.y * umPerPx;
    const s = hitSpot(x, y) ?? null;
    if (s && ro) {
      app.selection = [s.id];
      return true;
    }
    if (ro) return false;
    if (s) {
      app.selection = [s.id];
      startEdit(s.id);
      return true;
    }
    const l = hitLabel(x, y);
    if (l) {
      startEdit(l.id);
      return true;
    }
    return false;
  }

  function hover(p: CanvasPoint | null) {
    cursorScreen = p ? [p.sx, p.sy] : null;
    if (!p) {
      hoverId = null;
      return;
    }
    const x = p.x * umPerPx, y = p.y * umPerPx;
    lassoHover = lasso ? [x, y] : null;
    hoverId = hitSpot(x, y)?.id ?? grainAt(detections, x, y, app.layers.artifacts)?.id ?? null;
    if (app.tool === 'spot' && app.settings.snap && !(p.ev as MouseEvent).altKey) {
      const prev = app.spots.filter((s) => s.area_id === areaId).at(-1) ?? null;
      snapHint = snap(x, y, snapTargets(grains, prev, app.diameter), 12 * k);
    } else snapHint = null;
  }

  // ---- inline rename ------------------------------------------------------------------
  function startEdit(id: string) {
    if (!view) return;
    const s = app.spots.find((q) => q.id === id);
    const l = app.labels.find((q) => q.id === id);
    const xUm = s ? s.x_um + s.diameter_um / 2 : l!.x_um;
    const yUm = s ? s.y_um : l!.y_um;
    const [sx, sy] = toScreen(view, xUm / umPerPx, yUm / umPerPx);
    editing = { id, text: s?.name ?? l?.text ?? '', sx: sx + 6, sy: sy - 12 };
  }
  function commitEdit() {
    if (!editing) return;
    const { id, text } = editing;
    editing = null;
    if (app.spots.some((s) => s.id === id)) {
      if (app.spots.find((s) => s.id === id)?.name !== text) updateSpot(id, { name: text });
    } else {
      app.checkpoint();
      app.labels = app.labels.map((l) => (l.id === id ? { ...l, text } : l));
    }
  }
  function focusInput(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  // ---- keyboard --------------------------------------------------------------------------
  function key(e: KeyboardEvent) {
    if (app.checkPrompt || app.renamePrompt) return;
    const t = e.target as HTMLElement;
    if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable) return;
    const mod = e.metaKey || e.ctrlKey;
    if (ro) {
      // Read-only: view keys only (the viewer handles zoom and pan), Escape, measure and select.
      if (mod || e.altKey) return;
      if (e.key === 'Escape') {
        app.selection = [];
        measure = null;
        app.tool = 'select';
      } else if (e.key.toLowerCase() === 'm') app.tool = 'measure';
      else if (e.key.toLowerCase() === 'v') app.tool = 'select';
      return;
    }
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      if (e.shiftKey) app.redo();
      else app.undo();
      return;
    }
    if (mod && e.key.toLowerCase() === 'y') {
      e.preventDefault();
      app.redo();
      return;
    }
    if (lasso && (e.key === 'Enter' || e.key === 'Escape' || e.key === 'Backspace' || e.key === 'Delete')) {
      e.preventDefault();
      if (e.key === 'Enter') void finishLasso();
      else if (e.key === 'Escape') lasso = null;
      else if (lasso.pts.length > 1) lasso.pts.pop();
      else lasso = null;
      return;
    }
    if (mod || e.altKey) return;
    const tools: Record<string, Tool> = { v: 'select', s: 'spot', l: 'line', r: 'raster', m: 'measure', t: 'label', d: 'delete', g: 'grain-add', k: 'grain-split', o: 'grain-outline', b: 'grain-brush' };
    const kk = e.key.toLowerCase();
    if (tools[kk] && !(noPixels && (kk === 'g' || kk === 'k' || kk === 'o' || kk === 'b'))) app.tool = tools[kk];
    else if (/^[1-9]$/.test(e.key) && app.settings.spot_presets_um[Number(e.key) - 1]) setDiameter(app.settings.spot_presets_um[Number(e.key) - 1]);
    else if (e.key === '[') viewer?.rotateBy(-Math.PI / 12);
    else if (e.key === ']') viewer?.rotateBy(Math.PI / 12);
    else if (e.key === 'Delete' || e.key === 'Backspace') {
      if (!deleteSelection() && selGrains.length) deleteGrains();
    } else if (e.key.startsWith('Arrow') && selSpots.length) {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      const [dx, dy] = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key] as [number, number];
      app.checkpoint();
      app.spots = app.spots.map((s) => (app.selection.includes(s.id) ? { ...s, x_um: s.x_um + dx, y_um: s.y_um + dy, x2_um: s.x2_um == null ? null : s.x2_um + dx, y2_um: s.y2_um == null ? null : s.y2_um + dy } : s));
    } else if (e.key === 'Escape') {
      app.selection = [];
      selGrains = [];
      measure = null;
      editing = null;
      app.tool = 'select';
    }
  }

  function setDiameter(d: number) {
    app.diameter = d;
    if (selSpots.length) {
      app.checkpoint();
      app.spots = app.spots.map((s) => (app.selection.includes(s.id) ? { ...s, diameter_um: d } : s));
    }
  }

  const cursor = $derived(app.tool === 'select' ? 'default' : app.tool === 'delete' ? 'not-allowed' : 'crosshair');
  const fmt = (v: number | null | undefined, d = 0) => (v == null || !Number.isFinite(v) ? '–' : v.toFixed(d));
  const toolList: { id: Tool; label: string; key: string; title: string }[] = [
    { id: 'select', label: 'Select', key: 'V', title: 'Select and move; drag empty space to pan' },
    { id: 'spot', label: 'Spot', key: 'S', title: 'Click to place a spot' },
    { id: 'line', label: 'Line', key: 'L', title: 'Drag to draw a line scan' },
    { id: 'raster', label: 'Raster', key: 'R', title: 'Drag to draw a raster area' },
    { id: 'measure', label: 'Measure', key: 'M', title: 'Drag to measure a distance' },
    { id: 'label', label: 'Label', key: 'T', title: 'Click to place a text label' },
    { id: 'delete', label: 'Delete', key: 'D', title: 'Click a spot, label or grain to delete it' },
  ];
</script>

<svelte:window onkeydown={key} />

{#if core && tiles && areaId && mosaic}
  <div class="canvas" class:folded={app.layout.sideCollapsed}>
    {#key areaId}
      {@const viewerArea = areaId}
      <Viewer bind:this={viewer} bind:view={() => view, (next) => setAreaView(viewerArea, next)} core={tiles!} {areaId} {mosaic} {lut} version={app.pixelsVersion} {cursor}
        preview={app.preview[areaId] ?? null} onlevel={(l) => (app.drawLevel = l)} ondraw={scheduleContrast} canNudge={() => !selSpots.length}
        onpress={press} ondrag={drag} onrelease={release} onclick={click} ondblclick={dblclick} onhover={hover}>
        {#snippet overlay()}
          <g transform="scale({1 / umPerPx})" data-testid="canvas-um-space">
            {#if refOverlay?.url && refOverlay.visible}
              <image class="refimg" href={refOverlay.url} width={refOverlay.w} height={refOverlay.h} transform="matrix({refOverlay.matrix.join(' ')})"
                opacity={app.tool === 'pair' && pairPending ? 0.12 : app.tool === 'pair' ? 0.95 : refOverlay.opacity} preserveAspectRatio="none" />
            {/if}
            {#if refOverlay && app.tool === 'pair'}
              {#each refOverlay.pairs as pr, i (i)}
                {@const m = refOverlay.matrix}
                {@const ix = m[0] * pr[0][0] + m[2] * pr[0][1] + m[4]}
                {@const iy = m[1] * pr[0][0] + m[3] * pr[0][1] + m[5]}
                <g class="pairmark"><line x1={ix} y1={iy} x2={pr[1][0]} y2={pr[1][1]} /><circle cx={pr[1][0]} cy={pr[1][1]} r={5 * k} /><text transform="translate({pr[1][0]} {pr[1][1]}) rotate({-rotDeg}) scale({k})" dx="7" dy="-5">{i + 1}</text></g>
              {/each}
            {/if}
            {#if app.layers.outlines && app.layers.artifacts}
              {#each artifacts as g (g.id)}
                <polygon class="grain artifact" class:sel={selGrains.includes(g.id)} class:hov={hoverId === g.id} points={g.polygon_um.map((p) => p.join(',')).join(' ')} data-testid="grain-artifact"><title>Not a grain: {ARTIFACT_LABELS[g.artifact!]} (select it and Keep as grain to restore)</title></polygon>
              {/each}
            {/if}
            {#if app.layers.outlines}
              {#each outlinedGrains as g (g.id)}
                {@const own = owners.get(g.id)}
                <polygon class="grain" class:sel={selGrains.includes(g.id)} class:hov={hoverId === g.id} class:dup={!!own?.duplicate_of} class:cut={!!own?.cut && !own?.duplicate_of} points={g.polygon_um.map((p) => p.join(',')).join(' ')} data-testid={own?.duplicate_of ? 'grain-dup' : undefined}><title>{own?.duplicate_of ? ownerNote(g) : own?.cut ? 'owned here, cut by the area edge' : own ? 'owned by this area' : ''}</title></polygon>
              {/each}
            {/if}
            {#if flashingSpot}
              <circle class="shell-flash spot-flash" cx={flashingSpot.x_um} cy={flashingSpot.y_um} r={flashingSpot.diameter_um / 2 + 7 * k} data-testid="shell-spot-flash" />
            {/if}
            {#if flashingGrain}
              <polygon class="shell-flash grain-flash" points={flashingGrain.polygon_um.map((p) => p.join(',')).join(' ')} data-testid="shell-grain-flash" />
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
            {#if app.layers.results && range}
              <g class="results" data-testid="results-layer">
                {#each shown as { s, r, v } (s.id)}
                  {#if v}
                    <circle class="rfill" cx={s.x_um} cy={s.y_um} r={s.diameter_um / 2} fill={rampColor(tOf(v.value))} data-testid="result-dot" />
                  {/if}
                  {#if r.flags.length}
                    <circle class="rflag" cx={s.x_um} cy={s.y_um} r={s.diameter_um / 2 + 4 * k} data-testid="result-flag"><title>{r.flags.join(', ')}</title></circle>
                  {/if}
                {/each}
              </g>
            {/if}
            {#if app.layers.spots}
              {#each spots as s (s.id)}
                {@const sel = app.selection.includes(s.id)}
                {@const active = activeIds.includes(s.id)}
                <g class="spot {s.kind}" class:sel class:active class:hov={hoverId === s.id} data-testid="spot"
                  data-contrast={active ? (autoColour ? (contrasts[s.id] ?? 'light') : 'fixed') : undefined}
                  style:--active-spot-colour={active ? activeColour(s.id) : undefined} style:--active-spot-halo={active ? activeHalo(s.id) : undefined}>
                  {#if s.kind === 'line' && s.x2_um != null && s.y2_um != null}
                    <line class="swath" x1={s.x_um} y1={s.y_um} x2={s.x2_um} y2={s.y2_um} style:stroke-width="{s.diameter_um}px" />
                    <line class="axis" x1={s.x_um} y1={s.y_um} x2={s.x2_um} y2={s.y2_um} />
                  {:else if s.kind === 'raster' && s.x2_um != null && s.y2_um != null}
                    <rect x={s.x_um} y={s.y_um} width={s.x2_um - s.x_um} height={s.y2_um - s.y_um} />
                  {:else}
                    {#if active}<circle class="contrast-halo" cx={s.x_um} cy={s.y_um} r={s.diameter_um / 2} />{/if}
                    <circle cx={s.x_um} cy={s.y_um} r={s.diameter_um / 2} />
                    <circle class="dot" cx={s.x_um} cy={s.y_um} r={1.2 * k} />
                    {#if sel}
                      <circle class="handle" cx={s.x_um + (s.diameter_um / 2) * Math.SQRT1_2} cy={s.y_um + (s.diameter_um / 2) * Math.SQRT1_2} r={5 * k} />
                    {/if}
                  {/if}
                  {#if app.layers.labels && editing?.id !== s.id}
                    <text transform="translate({s.x_um + s.diameter_um / 2 + 3 * k} {s.y_um}) rotate({-rotDeg}) scale({k})" dy="4">{s.name}</text>
                  {/if}
                </g>
              {/each}
            {/if}
            {#if app.layers.labels}
              {#each labels as l (l.id)}
                {#if editing?.id !== l.id}
                  <text class="flabel" class:sel={app.selection.includes(l.id)} transform="translate({l.x_um} {l.y_um}) rotate({-rotDeg}) scale({k})">{l.text}</text>
                {/if}
              {/each}
            {/if}
            {#if snapHint}
              <g class="snap"><circle cx={snapHint.x} cy={snapHint.y} r={7 * k} /><line x1={snapHint.x - 10 * k} x2={snapHint.x + 10 * k} y1={snapHint.y} y2={snapHint.y} /><line y1={snapHint.y - 10 * k} y2={snapHint.y + 10 * k} x1={snapHint.x} x2={snapHint.x} /></g>
            {/if}
            {#if app.tool === 'spot' && cursorUm && !snapHint}
              <circle class="ghost contrast-halo" cx={cursorUm[0]} cy={cursorUm[1]} r={app.diameter / 2} style:stroke={activeHalo('ghost')} />
              <circle class="ghost" cx={cursorUm[0]} cy={cursorUm[1]} r={app.diameter / 2}
                data-testid="spot-ghost" data-contrast={autoColour ? (contrasts.ghost ?? 'light') : 'fixed'} style:stroke={activeColour('ghost')} />
            {/if}
            {#if drawing}
              {#if drawing.kind === 'raster'}
                <rect class="drawing" x={Math.min(drawing.x, drawing.x2)} y={Math.min(drawing.y, drawing.y2)} width={Math.abs(drawing.x2 - drawing.x)} height={Math.abs(drawing.y2 - drawing.y)} />
              {:else}
                <line class="drawing" x1={drawing.x} y1={drawing.y} x2={drawing.x2} y2={drawing.y2} />
              {/if}
              {#if drawing.kind === 'measure'}
                <text class="mlabel" transform="translate({drawing.x2} {drawing.y2}) rotate({-rotDeg}) scale({k})" dx="8" dy="-6">{Math.hypot(drawing.x2 - drawing.x, drawing.y2 - drawing.y).toFixed(1)} µm</text>
              {/if}
            {/if}
            {#if measure}
              <line class="measure" x1={measure[0]} y1={measure[1]} x2={measure[2]} y2={measure[3]} />
              <text class="mlabel" transform="translate({measure[2]} {measure[3]}) rotate({-rotDeg}) scale({k})" dx="8" dy="-6">{Math.hypot(measure[2] - measure[0], measure[3] - measure[1]).toFixed(1)} µm</text>
            {/if}
            {#if splitLine}
              <line class="splitline" x1={splitLine[0]} y1={splitLine[1]} x2={splitLine[2]} y2={splitLine[3]} />
            {/if}
            {#if lasso}
              <polyline class="lasso" class:cut={lasso.op === 'cut'} points={[...lasso.pts, ...(lassoHover && !(lasso.clicks === 1 && lasso.dragged) ? [lassoHover] : [])].map((q) => q.join(',')).join(' ')} data-testid="lasso" />
              {#if lasso.pts.length > 2}<circle class="lasso-start" cx={lasso.pts[0][0]} cy={lasso.pts[0][1]} r={5 * k} />{/if}
            {/if}
            {#if stroke}
              <polyline class="brushstroke" class:cut={stroke.op === 'cut'} points={stroke.pts.map((q) => q.join(',')).join(' ')} stroke-width={brushUm} />
            {/if}
            {#if app.tool === 'grain-brush' && cursorUm && !stroke}
              <circle class="brushcursor" cx={cursorUm[0]} cy={cursorUm[1]} r={brushUm / 2} />
            {/if}
          </g>
        {/snippet}
      </Viewer>
    {/key}

    {#if editing}
      <input class="inline-edit" data-ui style:left="{editing.sx}px" style:top="{editing.sy}px" bind:value={editing.text} use:focusInput
        onkeydown={(e) => { if (e.key === 'Enter') commitEdit(); else if (e.key === 'Escape') editing = null; e.stopPropagation(); }} onblur={commitEdit} data-testid="rename" />
    {/if}

    <div class="glass toolbar" data-ui>
      <select value={app.areaId} onchange={(e) => changeArea(e.currentTarget.value)} aria-label="Area">
        {#each Object.keys(app.stitched) as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? app.project?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
      </select>
      <span class="sep"></span>
      {#each ro ? toolList.filter((t) => t.id === 'select' || t.id === 'measure') : toolList as t (t.id)}
        <button class="icon" aria-pressed={app.tool === t.id} title="{t.title} ({t.key})" onclick={() => (app.tool = t.id)} data-testid="tool-{t.id}">{t.label}</button>
      {/each}
      <span class="sep" class:hidden={ro}></span>
      <label class="diam" class:hidden={ro} title="Spot diameter (µm); 1–6 pick a preset">
        ⌀
        <select value={String(app.diameter)} onchange={(e) => setDiameter(+e.currentTarget.value)}>
          {#each [...new Set([...app.settings.spot_presets_um, app.diameter])].sort((a, b) => a - b) as d (d)}<option value={String(d)}>{d}</option>{/each}
        </select>
        <input type="number" min="1" step="0.5" value={app.diameter} onchange={(e) => setDiameter(+e.currentTarget.value)} aria-label="Diameter µm" />µm
      </label>
      {#if !noPixels}
        <span class="sep"></span>
        <button class="icon" title="Add grain: click inside it (G)" aria-pressed={app.tool === 'grain-add'} onclick={() => (app.tool = 'grain-add')}>+Grain</button>
        <button class="icon" title="Split grain: draw a line across it (K)" aria-pressed={app.tool === 'grain-split'} onclick={() => (app.tool = 'grain-split')}>Split</button>
        <button class="icon" title="Outline a grain (O): drag around it, or click its corners and double-click (or Enter) to close. ⇧ adds to the grain, ⌥ cuts from it, ⌘ draws without snapping" aria-pressed={app.tool === 'grain-outline'} onclick={() => (app.tool = 'grain-outline')} data-testid="tool-outline">Outline</button>
        <button class="icon" title="Grain brush (B): paint over a grain to add it (grows to its edges), ⌥ to erase" aria-pressed={app.tool === 'grain-brush'} onclick={() => (app.tool = 'grain-brush')} data-testid="tool-brush">Brush</button>
        {#if app.tool === 'grain-outline'}
          <label class="check" title="Pull the outline onto the grain edges near the path"><input type="checkbox" bind:checked={snapEdges} data-testid="snap-edges" /> snap to edges</label>
        {:else if app.tool === 'grain-brush'}
          <label class="diam" title="Brush diameter (µm)">⌀ <input type="number" min="2" step="2" bind:value={brushUm} aria-label="Brush µm" />µm</label>
        {/if}
      {/if}
      {#if !ro}
        <span class="sep"></span>
        <button class="icon" title="Undo (⌘Z)" disabled={!app.canUndo} onclick={() => app.undo()}>↶</button>
        <button class="icon" title="Redo (⇧⌘Z)" disabled={!app.canRedo} onclick={() => app.redo()}>↷</button>
      {/if}
      <span class="sep"></span>
      <button class="icon" title="Zoom in (+)" onclick={() => viewer?.zoomBy(1.5)}>+</button>
      <button class="icon" title="Zoom out (−)" onclick={() => viewer?.zoomBy(1 / 1.5)}>−</button>
      <button class="icon" title="Fit (0)" onclick={() => viewer?.fitAll()}>Fit</button>
      <button class="icon" title="Pixels 1:1" onclick={() => viewer?.oneToOne()}>1:1</button>
      <button class="icon" title="Rotate left 15° ([)" onclick={() => viewer?.rotateBy(-Math.PI / 12)}>↺</button>
      <button class="icon" title="Rotate right 15° (])" onclick={() => viewer?.rotateBy(Math.PI / 12)}>↻</button>
    </div>

    {#if panel.open && (focus || focusGrain)}
      <FloatingPanel title={focus ? `Spot ${focus.name}` : focusGrain?.artifact ? 'Not a grain' : 'Grain'} bind:x={app.layout.panel.x} bind:y={app.layout.panel.y} onmoved={() => app.saveLayout()} onclose={() => (panel.open = false)}
        minimized={app.layout.infoCollapsed} onminimize={() => { app.layout.infoCollapsed = !app.layout.infoCollapsed; app.saveLayout(); }}>
          {#if focus && resultsBy[focus.id]}
          {@const r = resultsBy[focus.id]}
          <div class="resbox" data-testid="spot-results">
            <div class="resdate"><span class="num">{fmtValue(r.date)}</span> <span class="faint">{r.date ? `${r.date.method ?? r.date.metric} · ${r.date.utype ?? '2σ'}${r.date.passes === false ? ' · fails filters' : ''}` : 'no date yet'}</span></div>
            {#if r.flags.length}<div class="flags">{#each r.flags as f (f)}<span class="flag">{f}</span>{/each}</div>{/if}
            <table class="data kv">
              <tbody>
                {#each r.ratios as v (v.metric)}<tr><td>{v.metric}</td><td class="num">{fmtValue(v)}{v.utype ? ` (${v.utype})` : ''}</td></tr>{/each}
                {#each r.concentrations as v (v.metric)}<tr><td>{v.metric}</td><td class="num">{fmtValue(v, Math.abs(v.value) >= 100 ? 0 : 1)}</td></tr>{/each}
                {#if r.session_key}<tr><td>session</td><td>{r.session_key}</td></tr>{/if}
              </tbody>
            </table>
            {#if r.acquisition_id}
              <button class="linkout" onclick={() => openAcquisition(r.acquisition_id!)} disabled={false} data-testid="open-acquisition">Open acquisition</button>
            {/if}
          </div>
        {/if}
        <fieldset class="plain" disabled={ro}>
        {#if focus}
          {@const st = stageOf(app.project?.areas.find((a) => a.id === focus.area_id) ?? null, focus.x_um, focus.y_um)}
          <div class="two">
            <label class="field">Name <input type="text" value={focus.name} onchange={(e) => updateSpot(focus.id, { name: e.currentTarget.value })} data-testid="panel-name" /></label>
            <label class="field" title="The spot's number; the name follows the pattern">n <input type="number" min="1" value={focus.n} onchange={(e) => setSpotN(focus.id, Math.max(1, Math.round(+e.currentTarget.value)))} data-testid="panel-n" /></label>
          </div>
          <div class="two">
            <label class="field">Diameter µm <input type="number" min="1" step="0.5" value={focus.diameter_um} onchange={(e) => updateSpot(focus.id, { diameter_um: +e.currentTarget.value })} /></label>
            <label class="field">Kind
              <select value={focus.kind} onchange={(e) => updateSpot(focus.id, { kind: e.currentTarget.value as SpotKind })}><option value="spot">spot</option><option value="line" disabled={focus.x2_um == null}>line</option><option value="raster" disabled={focus.x2_um == null}>raster</option></select>
            </label>
            <label class="field">x µm <input type="number" step="0.5" value={fmt(focus.x_um, 1)} onchange={(e) => updateSpot(focus.id, { x_um: +e.currentTarget.value })} /></label>
            <label class="field">y µm <input type="number" step="0.5" value={fmt(focus.y_um, 1)} onchange={(e) => updateSpot(focus.id, { y_um: +e.currentTarget.value })} /></label>
          </div>
          <label class="field">Comment <textarea rows="2" value={focus.comment} onchange={(e) => updateSpot(focus.id, { comment: e.currentTarget.value })}></textarea></label>
          <table class="data kv">
            <tbody>
              <tr><td>grain</td><td>{focus.grain_id ? 'on a grain' : 'not on a grain'}</td></tr>
              <tr><td>sample</td><td>{focus.sample}</td></tr>
              {#if laserOf(focus)}
                {@const lf = laserOf(focus)!}
                {@const lu = app.project?.laser?.areas?.[focus.area_id]?.units === 'um' ? 'µm' : 'mm'}
                <tr><td>laser</td><td class="num" data-testid="panel-laser">{(lf.matrix[0] * focus.x_um + lf.matrix[2] * focus.y_um + lf.matrix[4]).toFixed(lu === 'mm' ? 4 : 1)}, {(lf.matrix[1] * focus.x_um + lf.matrix[3] * focus.y_um + lf.matrix[5]).toFixed(lu === 'mm' ? 4 : 1)} {lu}</td></tr>
              {/if}
              <tr><td>stage</td><td class="num">{st ? `${(st[0] / 1e6).toFixed(4)}, ${(st[1] / 1e6).toFixed(4)} mm` : '–'}</td></tr>
              <tr><td>created</td><td>{focus.created.replace('T', ' ').slice(0, 19)} {focus.created_by}</td></tr>
              <tr><td>id</td><td class="mono small">{focus.id}</td></tr>
            </tbody>
          </table>
          <label class="field">laicpmsdb acquisition <input type="text" placeholder="id (optional)" value={focus.acquisition_id ?? ''} onchange={(e) => updateSpot(focus.id, { acquisition_id: e.currentTarget.value.trim() || null })} data-testid="panel-acquisition" /></label>
          <button class="linkout" onclick={() => openInLaicpmsdb(focus)} disabled={!hosted} data-testid="open-laicpmsdb"
            title={hosted ? (focus.acquisition_id ? 'Open this acquisition in the Sessions tab' : `Search laicpmsdb for the label ${focus.name}`) : 'Available when grainatlas runs inside UPb Lab'}>Open in laicpmsdb</button>
          <div class="two">
            <button onclick={() => renameFromPattern([focus.id])}>Rename from pattern</button>
            <button class="danger" onclick={() => { app.selection = [focus.id]; deleteSelection(); }}>Delete</button>
          </div>
        {:else if focusGrain}
          <table class="data kv">
            <tbody>
              <tr><td>⌀ equivalent</td><td class="num">{fmt(focusGrain.eq_diameter_um, 1)} µm</td></tr>
              <tr><td>length × width</td><td class="num">{fmt(focusGrain.major_um, 1)} × {fmt(focusGrain.minor_um, 1)} µm</td></tr>
              <tr><td>orientation</td><td class="num">{fmt(focusGrain.orientation_deg, 0)}°</td></tr>
              <tr><td>area</td><td class="num">{fmt(focusGrain.area_um2, 0)} µm²</td></tr>
              <tr><td>centroid</td><td class="num">{fmt(focusGrain.centroid_um[0], 1)}, {fmt(focusGrain.centroid_um[1], 1)} µm</td></tr>
              <tr><td>spots</td><td>{app.spots.filter((s) => s.grain_id === focusGrain.id).map((s) => s.name).join(', ') || '–'}</td></tr>
              {#if focusGrain.artifact}
                <tr><td>not a grain</td><td data-testid="artifact-reason">{ARTIFACT_LABELS[focusGrain.artifact]} <button class="link" onclick={() => keepAsGrain(focusGrain!.id)} data-testid="keep-grain">Keep as grain</button></td></tr>
              {:else if focusGrain.kept}
                <tr><td>kept</td><td>flagged, kept by you as a grain</td></tr>
              {/if}
              {#if focusGrain.drawn}
                <tr><td>outline</td><td>drawn by hand (Find again keeps it)</td></tr>
              {/if}
              {#if owners.get(focusGrain.id)}
                {@const own = owners.get(focusGrain.id)!}
                <tr><td>owner area</td><td data-testid="owner-area">{own.duplicate_of ? ownerNote(focusGrain) : `this area${own.cut ? ' (cut by its edge)' : ''}`}</td></tr>
              {/if}
              <tr><td>id</td><td class="mono small">{focusGrain.id}</td></tr>
            </tbody>
          </table>
          {#if selGrains.includes(focusGrain.id)}
            <div class="row">
              {#if !focusGrain.artifact}
                <button onclick={markArtifact} title="Unnumbered, left off print sheets and spots; Find again keeps the mark" data-testid="card-not-grain">Not a grain</button>
              {/if}
              <button class="danger" onclick={deleteGrains} title="Remove it from the project (⌘Z undoes)" data-testid="card-delete-grain">Delete grain</button>
            </div>
          {/if}
        {/if}
        </fieldset>
      </FloatingPanel>
    {/if}

    {#if app.layout.sideCollapsed}
      <aside class="glass side-mini" data-ui>
        <button class="icon" onclick={() => { app.layout.sideCollapsed = false; app.saveLayout(); }} title="Show the side panel" aria-label="Show the side panel" aria-expanded={false} data-testid="side-expand">‹ Panel</button>
      </aside>
    {:else}
    <aside class="glass side" data-ui>
      <div class="panel-heading">
        <span>Image panels</span>
        <button class="icon fold" onclick={() => { app.layout.sideCollapsed = true; app.saveLayout(); }} title="Fold the side panel away" aria-label="Fold the side panel" aria-expanded={true} data-testid="side-collapse">›</button>
      </div>
      <div class="tabs">
        <button aria-pressed={tab === 'spots'} onclick={() => (tab = 'spots')}>Spots <span class="faint">{spots.length}</span></button>
        <button aria-pressed={tab === 'checks'} onclick={() => (tab = 'checks')} data-testid="tab-checks">Checks</button>
        <button aria-pressed={tab === 'grains'} onclick={() => (tab = 'grains')}>Grains <span class="faint">{grains.length}</span></button>
        <button aria-pressed={tab === 'refOverlay'} onclick={() => (tab = 'refOverlay')} title="Reference image (printout photo, post-laser frame)">Overlay</button>
        <button aria-pressed={tab === 'laser'} onclick={() => (tab = 'laser')} title="Laser stage registration and spot list" data-testid="tab-laser">Laser</button>
        <button aria-pressed={tab === 'link'} onclick={() => (tab = 'link')} title="Mount, atlas bundle, publishing to laicpmsdb" data-testid="tab-link">laicpmsdb</button>
      </div>
      <input type="file" accept="image/*" hidden bind:this={overlayInput} onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) loadOverlay(f); e.currentTarget.value = ''; }} />
      {#if ro}<p class="faint small-text ro-note" data-testid="read-only">Published atlas · read-only</p>{/if}
      <fieldset class="plain tabbody" disabled={ro && tab !== 'link' && tab !== 'checks'}>
      {#if tab === 'spots'}
        {#if !ro}
        <div class="two">
          <label class="field">Sample <input type="text" value={sample} onchange={(e) => setSample(e.currentTarget.value)} data-testid="sample" /></label>
          <label class="field">Pattern <input type="text" bind:value={app.settings.naming_pattern} onchange={() => app.saveSettings()} /></label>
        </div>
        <label class="field">Name pattern for this area
          <input type="text" value={app.settings.area_patterns?.[areaId!] ?? ''} placeholder={app.settings.naming_pattern} onchange={(e) => setAreaPattern(e.currentTarget.value)} data-testid="area-pattern" />
        </label>
        <div class="presets">
          {#each app.settings.spot_presets_um as d, i (d)}
            <button class="icon" aria-pressed={app.diameter === d} title="Preset {i + 1}" onclick={() => setDiameter(d)}>{d} µm</button>
          {/each}
        </div>
        <div class="two">
          <label class="field" title="Start this sample's next spot at this number (for example after other samples' spots took the numbers in between, or a Qtegra run number); later spots count up from it. Empty = one more than this sample's highest">Next n
            <input type="number" min="1" placeholder="auto ({nextN(app.spots, sample)})" data-testid="next-n" value={app.nextN ?? ''} oninput={(e) => (app.nextN = e.currentTarget.value === '' ? null : Math.max(1, Math.round(+e.currentTarget.value)))} />
          </label>
          <label class="check"><input type="checkbox" bind:checked={app.settings.snap} onchange={() => app.saveSettings()} /> snap (⌥ = free)</label>
        </div>
        <div class="row">
          <button onclick={() => exportSpots('csv')} disabled={!app.spots.length} data-testid="export-csv">Spots CSV</button>
          <button onclick={() => exportSpots('geojson')} disabled={!app.spots.length && !grains.length}>GeoJSON</button>
          <button onclick={() => renameFromPattern(app.selection.length ? app.selection : spots.map((s) => s.id))} disabled={!spots.length} title="Rename the selected spots (or all in this area) from the pattern">Rename</button>
          <button onclick={sendSpots} disabled={!app.spots.length} data-testid="send-laicpmsdb"
            title={app.server && app.project?.laicpmsdb.mount_id ? "POST all areas’ spots, grouped by sample to the linked mount (with the sign-in token; downloads the JSON body when there is none)" : "Download all areas’ spots, grouped by sample as the JSON body of laicpmsdb's POST /v1/mounts/{mount_id}/spots (mosaic µm)"}>Send spots to laicpmsdb</button>
        </div>
        {/if}
        <div class="list">
          <table class="data">
            <thead><tr><th>Name</th><th>⌀ (µm)</th></tr></thead>
            <tbody>
              {#each spots as s (s.id)}
                <tr class:selected={app.selection.includes(s.id)} onclick={(e) => { app.selection = e.shiftKey ? [...app.selection, s.id] : [s.id]; panel.open = true; centerOnUm(s.x_um, s.y_um); }}>
                  <td>{s.name}</td><td class="num">{s.diameter_um}</td>
                </tr>
              {:else}
                <tr><td colspan="2" class="muted">{ro ? 'No registered spots in this area.' : 'Pick the Spot tool (S) and click on a grain.'}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else if tab === 'checks'}
        <h3>Project checks</h3>
        <p class="faint small-text">Warnings for all areas. Running checks changes nothing. Results describe the last run.</p>
        <div class="row wrap">
          <button onclick={() => checkNow()} disabled={checksBusy} data-testid="run-checks">{checksBusy ? 'Checking…' : 'Run checks'}</button>
          <button onclick={() => labelInput?.click()}>Load labels…</button>
          {#if app.server && app.mountId}
            <button onclick={() => { app.checkLabels = null; void checkNow(true); }} disabled={checksBusy} data-testid="refresh-labels">Refresh labels</button>
          {/if}
          {#if app.checkLabels !== null}<button onclick={() => { app.checkLabels = null; void checkNow(); }}>Use mount labels</button>{/if}
        </div>
        <input type="file" accept=".txt,.csv,text/plain,text/csv" hidden bind:this={labelInput} data-testid="labels-input" onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) void loadLabels(f); e.currentTarget.value = ''; }} />
        <p class="faint small-text">{app.checkLabels !== null ? `${app.checkLabels.length} labels loaded from file` : app.mountLabelsCount !== null ? `${app.mountLabelsCount} acquisition labels from the linked mount` : 'Load a label list or link a mount to compare names.'}</p>
        {#if app.checkLabelsError}<p class="faint small-text" role="status" data-testid="labels-error">{app.checkLabelsError}</p>{/if}
        {#if app.findings.some((f) => f.code === 'near_miss')}
          <button onclick={() => applySuggestions(app.findings)} disabled={ro} data-testid="apply-suggestions">Apply all suggestions</button>
        {/if}
        {#if app.checksRan && !app.findings.length}<p>No warnings.</p>{/if}
        {#each checkGroups as group (group.code)}
          <section class="check-group" data-testid={`check-${group.code}`}>
            <h4>{CHECK_TITLES[group.code] ?? group.code} ({group.rows.length})</h4>
            {#each group.rows as f}
              <div class="check-row">
                <button class="finding" onclick={() => selectFinding(f)}>{f.message}</button>
                {#if f.code === 'near_miss'}<button onclick={() => applySuggestions([f])} disabled={ro}>Rename to label</button>{/if}
              </div>
            {/each}
          </section>
        {/each}
      {:else if tab === 'laser'}
        <LaserPanel {core} {areaId} {spots} selected={app.selection} />
      {:else if tab === 'link'}
        <MountPicker value={app.project?.laicpmsdb.mount_id ?? null} onchange={setMount} disabled={ro} />
        <p class="faint small-text">Server: {app.server ?? 'none — set it in Settings'}</p>
        <h3 class="sub">Atlas bundle</h3>
        <p class="faint small-text">The corrected mosaic as 256 px WebP tiles ({app.link.quality >= 100 ? 'lossless' : `near-lossless ${app.link.quality}`}), grains, spots, scale bars and this project, with a hash of every file. The raw SEM package is not included.</p>
        <div class="row">
          <button onclick={() => saveAtlas(false)} disabled={noPixels || (core?.kind !== 'tauri' && !canSaveFolder())} title={core?.kind === 'tauri' || canSaveFolder() ? 'Write a <project id>.atlas folder' : 'This browser cannot write folders: save the zip'} data-testid="save-atlas-folder">Save folder</button>
          <button onclick={() => saveAtlas(true)} disabled={noPixels} title={noPixels ? 'Needs the stitched mosaic: open the raw package' : ''} data-testid="save-atlas-zip">Save zip</button>
          <button class="primary" onclick={publish} disabled={noPixels || !app.server || !app.project?.laicpmsdb.mount_id} data-testid="publish"
            title={noPixels ? 'New pixels need the raw package: open it to re-stitch and publish' : !app.server ? 'Set the laicpmsdb server in Settings' : !app.project?.laicpmsdb.mount_id ? 'Link the project to a mount first' : 'POST the bundle to /v1/mounts/{mount}/atlas'}>Publish to laicpmsdb</button>
        </div>
        {#if app.project?.laicpmsdb.published?.length}
          <table class="data" data-testid="published">
            <thead><tr><th>Published</th><th>Version</th><th>atlas.json</th></tr></thead>
            <tbody>
              {#each [...app.project.laicpmsdb.published].reverse() as pb (pb.at)}
                <tr title="{pb.server} · mount {pb.mount_id}"><td>{pb.at.replace('T', ' ').slice(0, 16)}</td><td class="num">{pb.version ?? '–'}</td><td class="mono small">{pb.atlas_sha256.slice(0, 12)}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
        <h3 class="sub">Results</h3>
        {#if !mountId || !app.server}
          <p class="faint small-text">With a server and a linked mount, the dates, ratios, concentrations and flags laicpmsdb holds for these spots show here and on the canvas.</p>
        {:else}
          <p class="faint small-text" data-testid="results-status">
            {#if app.results?.loading}Loading results…{:else if app.results?.error}{app.results.error}{:else if app.results?.data}{Object.keys(resultsBy).length} of {app.spots.length} spots have results ({app.results.data.run ?? 'preferred'} run){app.results.at ? `, fetched ${app.results.at.slice(11, 16)}` : ''}.{/if}
          </p>
          <div class="row">
            <button onclick={() => refreshResults()} disabled={app.results?.loading} data-testid="refresh-results">Refresh results</button>
            <label class="check"><input type="checkbox" bind:checked={app.layers.results} /> show on the canvas</label>
          </div>
          <label class="field">Colour by
            <select bind:value={app.colourBy} data-testid="colour-by">
              {#each metrics as m (m)}<option value={m}>{m === 'date' ? 'preferred date' : m}</option>{/each}
            </select>
          </label>
          <p class="faint small-text">Read from laicpmsdb each time; nothing is stored in the project.</p>
        {/if}
      {:else if tab === 'refOverlay'}
        <p class="muted small-text">Lay a reference image over the mosaic (a photo of an annotated printout, a post-laser frame) and register it by clicking pairs of the same feature: first on the image, then on the mosaic. One pair moves it, two scale and rotate it, three or more fit an affine transform.</p>
        <div class="row">
          <button class="primary" onclick={() => overlayInput?.click()}>{refOverlay ? 'Load again / replace' : 'Load image'}</button>
          {#if refOverlay}<button class="danger" onclick={() => { const { [areaId!]: _, ...rest } = app.overlays; app.overlays = rest; if (app.tool === 'pair') app.tool = 'select'; }}>Remove</button>{/if}
        </div>
        {#if refOverlay}
          <p class="faint">{refOverlay.name} · {refOverlay.w}×{refOverlay.h} px{refOverlay.url ? '' : ' (choose the file again to show it)'}</p>
          <label class="check"><input type="checkbox" checked={refOverlay.visible} onchange={(e) => setOverlay({ visible: e.currentTarget.checked })} /> show</label>
          <label class="field">Opacity {Math.round(refOverlay.opacity * 100)}%
            <input type="range" min="0" max="1" step="0.05" value={refOverlay.opacity} oninput={(e) => setOverlay({ opacity: +e.currentTarget.value })} />
          </label>
          <div class="row">
            <button aria-pressed={app.tool === 'pair'} onclick={() => { app.tool = app.tool === 'pair' ? 'select' : 'pair'; pairPending = null; }} data-testid="pair-tool">{app.tool === 'pair' ? 'Done pairing' : 'Add point pairs'}</button>
            <button onclick={() => refit(refOverlay.pairs.slice(0, -1))} disabled={!refOverlay.pairs.length}>Undo last pair</button>
          </div>
          {#if app.tool === 'pair'}
            <p class="accent">{pairPending ? '2 · now click the same feature on the mosaic (image faded)' : '1 · click a feature on the image'}</p>
          {/if}
          <table class="data">
            <thead><tr><th>Pair</th><th>Residual µm</th></tr></thead>
            <tbody>
              {#each refOverlay.pairs as _, i (i)}<tr><td>{i + 1}</td><td class="num">{refOverlay.pairs.length > 3 && refOverlay.residuals_um?.[i] != null ? refOverlay.residuals_um[i].toFixed(1) : '–'}</td></tr>{/each}
            </tbody>
          </table>
          {#if refOverlay.pairs.length > 3}<p class="muted">Fit RMS <b class="num">{refOverlay.rms_um.toFixed(1)} µm</b> over {refOverlay.pairs.length} pairs.</p>{/if}
          <p class="faint small-text">Then use the Spot tool with a pattern such as <code>{'{sample}_{n}'}</code> and set "Next n" to the run number before each click, so spot names equal the instrument labels.</p>
        {/if}
      {:else if segParams}
        <details class="params">
          <summary>Segmentation</summary>
          <label class="field">Grains are
            <select bind:value={segParams.polarity} data-testid="polarity"><option value="bright">brighter than the epoxy</option><option value="both">brighter or darker (CL: also CL-dark grains)</option><option value="dark">darker (transmitted light)</option></select>
          </label>
          {#if segParams.polarity === 'both'}
            <label class="field">Dark below the local epoxy level by (fraction) <input type="number" step="0.05" min="0.05" max="0.95" bind:value={segParams.dark_rel} /></label>
          {/if}
          <p class="faint small-text">Artifacts are flagged, not deleted: they show dimmed with their reason and can be kept as grains.</p>
          <label class="check"><input type="checkbox" bind:checked={segParams.flag_bubbles} /> flag epoxy bubbles and their halos</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.flag_slivers} /> flag tile-seam slivers (and cut seam lines)</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.flag_blocks} /> flag exposure-step tile blocks</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.flag_specks} /> flag specks</label>
          <label class="field">Reference grain size (µm, 0 = the area's median) <input type="number" min="0" bind:value={segParams.reference_size_um} /></label>
          <label class="field">Maximum size (µm, 0 = none) <input type="number" min="0" bind:value={segParams.max_size_um} /></label>
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
          <button class="primary" onclick={findGrains} disabled={noPixels} title={noPixels ? 'Needs the stitched mosaic: open the raw package' : ''} data-testid="find-grains">{grains.length ? 'Find again' : 'Find grains'}</button>
          <button onclick={exportGrains} disabled={!grains.length}>Grain CSV</button>
        </div>
        <div class="row">
          <button onclick={refine} disabled={!selGrains.length || noPixels} title="GrabCut on the selected grains">Refine</button>
          <button onclick={merge} disabled={selGrains.length < 2 || noPixels} title="Shift-click grains to select several">Merge</button>
          <button onclick={markArtifact} disabled={!selGrains.some((id) => grains.some((g) => g.id === id))} title="Not a grain: unnumbered, left off print sheets and spots" data-testid="mark-artifact">Not a grain</button>
          <button class="danger" onclick={deleteGrains} disabled={!selGrains.length}>Delete</button>
        </div>
        {#if dupCount}
          <p class="muted small-text" data-testid="dup-note">{dupCount} of {grains.length} are duplicates of grains another area owns (dashed outlines; left off print sheets).</p>
        {/if}
        <div class="list">
          <table class="data">
            <thead><tr><th>⌀ µm</th><th>L × W µm</th><th>spots</th><th>owner</th></tr></thead>
            <tbody>
              {#each grains as g (g.id)}
                <tr class:selected={selGrains.includes(g.id)} onclick={(e) => { selGrains = e.shiftKey ? [...selGrains, g.id] : [g.id]; app.selection = []; panel.open = true; centerOnUm(g.centroid_um[0], g.centroid_um[1], Math.max(g.bbox_um[2] - g.bbox_um[0], g.bbox_um[3] - g.bbox_um[1])); }}>
                  <td class="num">{fmt(g.eq_diameter_um)}</td><td class="num">{fmt(g.major_um)} × {fmt(g.minor_um)}</td><td class="num">{app.spots.filter((s) => s.grain_id === g.id).length || ''}</td>
                  <td class="owner" title={ownerNote(g)}>{#if owners.get(g.id)?.duplicate_of}{@const o = owners.get(g.id)!}→ {areaLabel(o.owner_area)}{:else if owners.get(g.id)?.cut}cut{/if}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if artifacts.length}
          <details class="artifacts" data-testid="artifact-list">
            <summary>Not grains <span class="faint">{artifacts.length}: {artifactCounts}</span></summary>
            <table class="data">
              <thead><tr><th>why</th><th>⌀ µm</th><th></th></tr></thead>
              <tbody>
                {#each artifacts as g (g.id)}
                  <tr class:selected={selGrains.includes(g.id)} onclick={() => { selGrains = [g.id]; app.selection = []; panel.open = true; centerOnUm(g.centroid_um[0], g.centroid_um[1], Math.max(g.bbox_um[2] - g.bbox_um[0], g.bbox_um[3] - g.bbox_um[1], 150)); }}>
                    <td title={ARTIFACT_LABELS[g.artifact!]}>{g.artifact}</td><td class="num">{fmt(g.eq_diameter_um)}</td>
                    <td><button class="link" onclick={(e) => { e.stopPropagation(); keepAsGrain(g.id); }} data-testid="keep-artifact">Keep as grain</button></td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </details>
        {/if}
      {/if}
      </fieldset>
      <details class="layers" open={app.layout.layersOpen} ontoggle={(e) => { app.layout.layersOpen = e.currentTarget.open; app.saveLayout(); }}>
        <summary>Layers</summary>
        <div class="two">
          <label class="check"><input type="checkbox" bind:checked={app.layers.outlines} /> outlines</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.artifacts} /> not grains</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.numbers} /> numbers</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.spots} /> spots</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.labels} /> names and labels</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.scalebars} /> scale bars</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.results} /> results</label>
          <label class="check"><input type="checkbox" bind:checked={panel.open} /> info panel</label>
        </div>
      </details>
    </aside>
    {/if}

    {#if app.layers.results && range}
      <div class="glass legend" data-ui data-testid="results-legend">
        <div class="lt">{app.colourBy === 'date' ? 'Preferred date' : app.colourBy}{range.unit ? ` (${range.unit})` : ''}</div>
        <div class="ramp" style:background={RAMP_CSS}></div>
        <div class="ends num"><span>{range.lo.toPrecision(4)}</span><span>{range.hi.toPrecision(4)}</span></div>
        <div class="faint">{shown.length} of {spots.length} spots · <span class="flagkey"></span> flagged</div>
      </div>
    {/if}
    <div class="glass status" data-ui>
      {#if cursorUm}<span class="num">x {cursorUm[0].toFixed(1)} µm, y {cursorUm[1].toFixed(1)} µm</span>{/if}
      {#if snapHint}<span class="accent">snap: {snapHint.what}</span>{/if}
      {#if view}<span class="num muted">{(view.zoom * 100).toFixed(view.zoom < 0.1 ? 1 : 0)}% · {rotDeg.toFixed(0)}°</span>{/if}
      {#if scalebar}
        <span class="bar"><span style:width="{scalebar.px}px"></span>{scalebar.len >= 1000 ? `${scalebar.len / 1000} mm` : `${scalebar.len} µm`}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .check-group { min-width: 0; }
  .check-group h4 { margin: var(--space-2) 0; }
  .check-row { display: grid; gap: var(--space-1); margin-bottom: var(--space-2); }
  .finding { text-align: left; white-space: normal; overflow-wrap: anywhere; }

  .canvas {
    --side-width:clamp(12rem, 37%, 21rem);
    position: absolute;
    inset: 0;
  }
  .toolbar {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    max-width: calc(100% - var(--side-width) - 2rem);
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-2);
    align-items: center;
  }
  .folded .toolbar {
    max-width: calc(100% - 5.5rem);
  }
  .folded .legend {
    right: calc(3rem + 2 * var(--space-2));
  }
  .side-mini {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: var(--space-1);
  }
  .panel-heading {display:flex;align-items:center;gap:var(--space-2);font-size:0.75rem;color:var(--text-muted);}
  .fold {
    margin-left: auto;
  }
  .sep {
    width: 1px;
    height: 1.4rem;
    background: var(--hairline);
    margin: 0 var(--space-1);
  }
  .diam {
    display: inline-flex;
    gap: var(--space-1);
    align-items: center;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .diam input {
    width: 4.2rem;
  }
  .side {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    bottom: var(--space-2);
    width: var(--side-width);
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: hidden;
  }
  .tabs,
  .row,
  .presets {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .tabs button {
    flex: 1;
  }
  .two {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-2);
  }
  .two > .field {
    min-width: 0;
  }
  .params,
  .layers {
    display: grid;
    gap: var(--space-1);
    font-size: 0.88rem;
  }
  .list {
    flex: 1;
    overflow: auto;
    min-height: 6rem;
  }
  .list tr {
    cursor: pointer;
  }
  .kv td:first-child {
    color: var(--text-muted);
    white-space: nowrap;
  }
  .small {
    font-size: 0.72rem;
    word-break: break-all;
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
  .accent {
    color: var(--accent);
  }
  .small-text {
    font-size: 0.8rem;
    margin: 0;
  }
  .hidden {
    display: none;
  }
  fieldset.plain {
    border: none;
    margin: 0;
    padding: 0;
    min-width: 0;
    display: contents;
  }
  /* The side panel's tab body scrolls on its own, so the tabs and Layers stay in reach on a
     laptop screen; lists inside keep at least a few rows. */
  fieldset.tabbody {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    /* 2px outline + 2px offset; keep the fields in their existing position. */
    padding: var(--space-1);
    margin: calc(-1 * var(--space-1));
  }
  fieldset.tabbody .list {
    flex: 1 0 auto;
    max-height: 60vh;
  }
  .legend {
    position: absolute;
    right: calc(var(--side-width) + 2 * var(--space-2));
    bottom: 3rem;
    padding: var(--space-2) var(--space-3);
    font-size: 0.8rem;
    display: grid;
    gap: 2px;
    min-width: 12rem;
  }
  .legend .ramp {
    height: 10px;
    border-radius: 3px;
  }
  .legend .ends {
    display: flex;
    justify-content: space-between;
  }
  .flagkey {
    display: inline-block;
    width: 0.8em;
    height: 0.8em;
    border-radius: 50%;
    border: 2px dashed #ff6b6b;
    vertical-align: middle;
  }
  .resbox {
    display: grid;
    gap: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-sm, 6px);
    background: var(--surface-2, transparent);
    border: 1px solid var(--hairline);
  }
  .resdate {
    font-size: 1.05rem;
    font-weight: 600;
  }
  .resdate .faint {
    font-size: 0.75rem;
    font-weight: 400;
  }
  .flags {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .flag {
    font-size: 0.75rem;
    padding: 0 6px;
    border-radius: 999px;
    border: 1px solid #ff6b6b;
    color: #ff6b6b;
  }
  :global(.ga-root) .results .rfill {
    stroke: none;
    opacity: 0.85;
  }
  :global(.ga-root) .results .rflag {
    fill: none;
    stroke: #ff6b6b;
    stroke-width: 2px;
    stroke-dasharray: 3 2;
    vector-effect: non-scaling-stroke;
  }
  .sub {
    font-size: 0.85rem;
    margin: var(--space-2) 0 0;
  }
  :global(.ga-root) .refimg {
    vector-effect: none;
  }
  :global(.ga-root) .pairmark line {
    stroke: var(--ov-measure);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  :global(.ga-root) .pairmark circle {
    fill: none;
    stroke: var(--ov-measure);
    stroke-width: 1.5;
  }
  :global(.ga-root) .pairmark text {
    fill: var(--ov-measure);
    font: 600 12px var(--font-sans);
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
  .inline-edit {
    position: absolute;
    z-index: 30;
    width: 12rem;
    font-family: var(--font-mono);
  }
  :global(.ga-root) .shell-flash {
    fill: none;
    stroke: var(--ov-number);
    stroke-width: 4;
    opacity: 1;
    pointer-events: none;
  }
  :global(.ga-root) .spot-flash {
    stroke: var(--ov-spot);
    animation: shell-pulse 500ms ease-in-out infinite alternate;
  }
  @keyframes shell-pulse { from { opacity: 0.4; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) {
    :global(.ga-root) .spot-flash { animation: none; }
  }
  :global(.ga-root) .grain {
    fill: none;
    stroke: var(--ov-outline);
    stroke-width: 1.4;
    opacity: 0.85;
  }
  :global(.ga-root) .grain.artifact {
    stroke: var(--ov-artifact, #9a9a9a);
    stroke-dasharray: 2 3;
    opacity: 0.55;
  }
  button.link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent, inherit);
    text-decoration: underline;
    cursor: pointer;
    font: inherit;
  }
  .artifacts summary {
    cursor: pointer;
    margin: var(--space-1) 0;
  }
  :global(.ga-root) .grain.dup {
    stroke-dasharray: 6 4;
    opacity: 0.5;
  }
  :global(.ga-root) .grain.cut {
    stroke-dasharray: 2 3;
  }
  .owner {
    font-size: 0.8em;
    white-space: nowrap;
    color: var(--text-muted, inherit);
  }
  :global(.ga-root) .grain.hov {
    opacity: 1;
    stroke-width: 2;
  }
  :global(.ga-root) .grain.sel {
    stroke: var(--ov-number);
    stroke-width: 2.4;
    fill: color-mix(in srgb, var(--ov-number) 12%, transparent);
  }
  :global(.ga-root) .gnum {
    fill: var(--ov-number);
    font: 600 13px var(--font-sans);
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.75);
    stroke-width: 3px;
  }
  :global(.ga-root) .gbar line {
    stroke: var(--ov-label);
    stroke-width: 2;
  }
  :global(.ga-root) .gbar text,
  :global(.ga-root) .spot text,
  :global(.ga-root) .flabel,
  :global(.ga-root) .mlabel {
    fill: var(--ov-label);
    font: 12px var(--font-sans);
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.8);
    stroke-width: 3px;
  }
  :global(.ga-root) .spot circle,
  :global(.ga-root) .spot rect {
    fill: color-mix(in srgb, var(--ov-spot) 14%, transparent);
    stroke: var(--ov-spot);
    stroke-width: 1.6;
  }
  :global(.ga-root) .spot .dot {
    fill: var(--ov-spot);
    stroke: none;
  }
  :global(.ga-root) .spot.raster rect {
    stroke: var(--ov-raster);
    fill: color-mix(in srgb, var(--ov-raster) 14%, transparent);
  }
  :global(.ga-root) .spot .swath {
    stroke: color-mix(in srgb, var(--ov-line) 35%, transparent);
    vector-effect: none;
    stroke-linecap: round;
  }
  :global(.ga-root) .spot .axis {
    stroke: var(--ov-line);
    stroke-width: 1.4;
  }
  :global(.ga-root) .spot.hov circle,
  :global(.ga-root) .spot.hov rect {
    stroke-width: 2.4;
  }
  :global(.ga-root) .spot.sel circle,
  :global(.ga-root) .spot.sel rect,
  :global(.ga-root) .spot.sel .axis {
    stroke: var(--ov-spot-selected);
    stroke-width: 2.4;
  }
  :global(.ga-root) .spot.sel text {
    font-weight: 700;
  }
  :global(.ga-root) .spot .handle {
    fill: var(--ov-spot-selected);
    stroke: var(--ov-spot);
    stroke-width: 1.5;
    cursor: nwse-resize;
  }
  :global(.ga-root) .spot.active circle,
  :global(.ga-root) .spot.active rect,
  :global(.ga-root) .spot.active .axis {
    stroke: var(--active-spot-colour);
  }
  :global(.ga-root) .spot.active circle.contrast-halo {
    fill: none;
    stroke: var(--active-spot-halo);
    stroke-width: 4.8;
    pointer-events: none;
  }
  :global(.ga-root) .flabel.sel {
    fill: var(--accent);
  }
  :global(.ga-root) .snap circle,
  :global(.ga-root) .snap line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  :global(.ga-root) .ghost {
    fill: none;
    stroke: var(--ov-spot);
    stroke-dasharray: 4 3;
    stroke-width: 1.2;
    pointer-events: none;
  }
  :global(.ga-root) .ghost.contrast-halo {
    stroke-width: 3.6;
  }
  :global(.ga-root) .drawing,
  :global(.ga-root) .measure {
    fill: color-mix(in srgb, var(--ov-measure) 12%, transparent);
    stroke: var(--ov-measure);
    stroke-width: 1.6;
    stroke-dasharray: 6 4;
  }
  :global(.ga-root) .lasso {
    fill: color-mix(in srgb, var(--ov-number) 10%, transparent);
    stroke: var(--ov-number);
    stroke-width: 1.6;
    stroke-dasharray: 5 3;
  }
  :global(.ga-root) .lasso.cut,
  :global(.ga-root) .brushstroke.cut {
    stroke: var(--danger);
  }
  :global(.ga-root) .lasso-start {
    fill: var(--ov-number);
  }
  :global(.ga-root) .brushstroke {
    fill: none;
    stroke: color-mix(in srgb, var(--ov-number) 45%, transparent);
    stroke-linecap: round;
    stroke-linejoin: round;
    vector-effect: none;
  }
  :global(.ga-root) .brushcursor {
    fill: none;
    stroke: var(--ov-number);
    stroke-width: 1;
  }
  :global(.ga-root) .splitline {
    stroke: var(--danger);
    stroke-width: 2;
    stroke-dasharray: 6 4;
  }
</style>
