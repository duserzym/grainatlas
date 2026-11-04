<script lang="ts">
  // Canvas screen: the infinite canvas with grains and spots.
  // Pointer-first: navigate with trackpad or mouse, click to place, drag a spot
  // to move it, drag its ring handle to resize, double-click its name to rename;
  // the floating panel edits the hovered or selected object. Keys are extras.
  import { app, type Overlay, type Tool } from '../lib/store.svelte';
  import Viewer, { type CanvasPoint } from '../lib/canvas/Viewer.svelte';
  import { toScreen, type View } from '../lib/canvas/view';
  import type { Core } from '../lib/core/core';
  import type { Grain, GrainOwner, Label, SegmentParams, Spot, SpotKind } from '../lib/core/types';
  import { download } from '../lib/files';
  import { formatName, grainAt, newId, nextN, snap, snapTargets, spotDistance, stageOf, type SnapTarget } from '../lib/spots';
  import FloatingPanel from '../components/FloatingPanel.svelte';
  import { isHosted, open as openLink } from '@lab/bridge';
  import { mountSpots, postSpots, spotLink } from '../lib/laicpmsdb';
  import MountPicker from '../components/MountPicker.svelte';
  import LaserPanel from '../components/LaserPanel.svelte';
  import { authToken, canSaveFolder, publish, saveAtlas } from '../lib/atlas';
  import { fmtValue, matchResults, metricsOf, rampColor, RAMP_CSS, refreshResults, valueOf, type SpotResult } from '../lib/results';
  import { acquisitionPage } from '../lib/laicpmsdb';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  let viewer = $state<ReturnType<typeof Viewer>>();
  let view = $state<View | undefined>(undefined);
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  let cursorUm = $state<[number, number] | null>(null);
  let selGrains = $state<string[]>([]);
  let tab = $state<'spots' | 'grains' | 'refOverlay' | 'link' | 'laser'>('spots');
  let pairPending = $state<[number, number] | null>(null);
  let overlayInput = $state<HTMLInputElement>();
  let panel = $state({ x: 16, y: 150, open: true });
  let hoverId = $state<string | null>(null);
  let snapHint = $state<SnapTarget | null>(null);
  // In-progress gestures.
  let splitLine = $state<[number, number, number, number] | null>(null);
  let drawing = $state<{ kind: SpotKind | 'measure'; x: number; y: number; x2: number; y2: number } | null>(null);
  let measure = $state<[number, number, number, number] | null>(null);
  let gesture: { mode: 'move' | 'resize'; ids: string[]; startX: number; startY: number; orig: Map<string, { x: number; y: number; x2: number | null; y2: number | null; d: number }>; dirty: boolean } | null = null;
  let editing = $state<{ id: string; text: string; sx: number; sy: number } | null>(null);

  /** A bundle from a server shown read-only; `noPixels`: no stitched mosaic in the core (server tiles). */
  const ro = $derived(app.readOnly);
  const noPixels = $derived(!!app.remote);
  const tiles = $derived(app.remote && app.remoteTiles ? app.remoteTiles : core);
  const areaId = $derived(app.areaId && app.stitched[app.areaId] ? app.areaId : (Object.keys(app.stitched)[0] ?? null));
  const area = $derived(app.project?.areas.find((a) => a.id === areaId) ?? null);
  const mosaic = $derived(areaId ? app.stitched[areaId].mosaic : null);
  const umPerPx = $derived(mosaic?.um_per_px ?? 1);
  const grains = $derived(areaId ? (app.grains[areaId] ?? []) : []);
  const spots = $derived(app.spots.filter((s) => s.area_id === areaId));
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
  const focus = $derived.by(() => {
    const id = app.selection[0] ?? hoverId;
    return app.spots.find((s) => s.id === id) ?? null;
  });
  // ---- results overlay (GET /v1/mounts/{id}/results; memory only) ------------------------------
  const mountId = $derived(app.project?.laicpmsdb.mount_id ?? null);
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
    // Not inside UPb Studio: laicpmsdb's own page, when the server is known.
    if (app.server) window.open(acquisitionPage(app.server, id), '_blank', 'noopener');
    else app.notice = `Acquisition ${id}: open it in laicpmsdb (no server set here).`;
  }

  const focusGrain = $derived(!focus ? (grains.find((g) => g.id === (selGrains[0] ?? hoverId)) ?? null) : null);

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
    panel.open = true;
    centerOnUm(s.x_um, s.y_um);
  });

  // ---- laicpmsdb (through UPb Studio) -------------------------------------------
  const hosted = isHosted();
  async function openInLaicpmsdb(s: Spot) {
    const url = spotLink(s);
    const ok = await openLink(url);
    app.error = ok ? null : `UPb Studio did not open ${url}`;
  }
  /**
   * Spot sync: with a server, a mount and a token (the shell's, or one pasted in Settings) the
   * spots go straight to POST /v1/mounts/{mount}/spots and the acquisitions laicpmsdb matched by
   * label come back as the spots' acquisition ids; otherwise the same body is downloaded.
   */
  async function sendSpots() {
    if (!app.project || !areaId) return;
    const { body, skipped } = mountSpots(app.project.uuid, sample, $state.snapshot(spots) as Spot[]);
    const mount = app.project.laicpmsdb.mount_id;
    const server = app.server;
    const tok = server && mount ? await authToken() : null;
    if (!server || !mount || !tok) {
      download(JSON.stringify(body, null, 2), `${app.pkg?.name ?? app.project.package.name}_${areaId}_laicpmsdb-spots.json`, 'application/json');
      app.notice = `${body.spots.length} spots for ${mount ? `mount ${mount}` : 'laicpmsdb (no mount id set)'}${skipped ? `; ${skipped} lines and rasters left out` : ''}. Downloaded: ${!server ? 'no server set' : !mount ? 'no mount linked' : 'no token'}.`;
      return;
    }
    const r = await app.run(`Sending ${body.spots.length} spots`, () => postSpots(server, tok, mount, body));
    if (!r) return;
    const linked = new Map(r.acquisitions.filter((a) => a.atlas_spot_id).map((a) => [a.atlas_spot_id.toLowerCase(), a.id]));
    if (linked.size) {
      app.checkpoint();
      app.spots = app.spots.map((s) => (linked.has(s.id.toLowerCase()) && !s.acquisition_id ? { ...s, acquisition_id: linked.get(s.id.toLowerCase())! } : s));
    }
    app.notice = `Sent to mount ${mount}: ${r.created} created, ${r.updated} updated, ${r.unchanged} unchanged${r.grains_created ? `, ${r.grains_created} grains created` : ''}${linked.size ? `; ${linked.size} acquisitions linked` : ''}${skipped ? `; ${skipped} lines and rasters left out` : ''}.`;
  }

  // ---- spots --------------------------------------------------------------------
  function now() {
    return new Date().toISOString();
  }
  function makeSpot(x: number, y: number, kind: SpotKind = 'spot', x2: number | null = null, y2: number | null = null): Spot {
    const g = grainAt(grains, x, y);
    const n = app.nextN ?? nextN(app.spots);
    if (app.nextN != null) app.nextN = n + 1;
    return {
      id: newId(),
      area_id: areaId!,
      name: formatName(app.settings.naming_pattern, sample, g?.number ?? null, n),
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
  function deleteSelection() {
    if (!app.selection.length) return false;
    app.checkpoint();
    app.spots = app.spots.filter((s) => !app.selection.includes(s.id));
    app.labels = app.labels.filter((l) => !app.selection.includes(l.id));
    app.selection = [];
    return true;
  }
  function renameFromPattern(ids: string[]) {
    app.checkpoint();
    app.spots = app.spots.map((s) => {
      if (!ids.includes(s.id)) return s;
      const g = grainAt(app.grains[s.area_id] ?? [], s.x_um, s.y_um);
      return { ...s, grain_id: g?.id ?? null, grain_number: g?.number ?? null, name: formatName(app.settings.naming_pattern, s.sample || sample, g?.number ?? null, s.n) };
    });
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
    const parts = await app.run(`Splitting grain ${g.number}`, () => core!.splitGrain(id, $state.snapshot(g) as Grain, line));
    if (!parts || parts.length < 2) return;
    const max = Math.max(0, ...grains.map((q) => q.number));
    parts[0] = { ...parts[0], id: g.id, number: g.number };
    for (let i = 1; i < parts.length; i++) parts[i] = { ...parts[i], number: max + i };
    setGrains([...grains.filter((q) => q.id !== g.id), ...parts].sort((a, b) => a.number - b.number));
  }

  async function click(p: CanvasPoint) {
    const ev = p.ev as MouseEvent;
    const x = p.x * umPerPx, y = p.y * umPerPx;
    if (ro && app.tool !== 'select' && app.tool !== 'measure') return;
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
    if (app.tool === 'grain-add' && core && areaId && segParams) {
      const id = areaId;
      const g = await app.run('Growing grain', () => core!.grainAt(id, x, y, $state.snapshot(segParams) as SegmentParams));
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
    const g = grainAt(grains, x, y);
    const multi = ev.shiftKey || ev.metaKey;
    if (!g) {
      if (!multi) selGrains = [];
      return;
    }
    selGrains = multi ? (selGrains.includes(g.id) ? selGrains.filter((s) => s !== g.id) : [...selGrains, g.id]) : [g.id];
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
    cursorUm = p ? [p.x * umPerPx, p.y * umPerPx] : null;
    if (!p) {
      hoverId = null;
      return;
    }
    const x = p.x * umPerPx, y = p.y * umPerPx;
    hoverId = hitSpot(x, y)?.id ?? grainAt(grains, x, y)?.id ?? null;
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
    if (mod || e.altKey) return;
    const tools: Record<string, Tool> = { v: 'select', s: 'spot', l: 'line', r: 'raster', m: 'measure', t: 'label', d: 'delete', g: 'grain-add', k: 'grain-split' };
    const kk = e.key.toLowerCase();
    if (tools[kk] && !(noPixels && (kk === 'g' || kk === 'k'))) app.tool = tools[kk];
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
    { id: 'delete', label: 'Delete', key: 'D', title: 'Click a spot or label to delete it' },
  ];
</script>

<svelte:window onkeydown={key} />

{#if core && tiles && areaId && mosaic}
  <div class="canvas">
    {#key areaId}
      <Viewer bind:this={viewer} bind:view core={tiles!} {areaId} {mosaic} {lut} version={app.pixelsVersion} {cursor}
        preview={app.preview[areaId] ?? null} onlevel={(l) => (app.drawLevel = l)} canNudge={() => !selSpots.length}
        onpress={press} ondrag={drag} onrelease={release} onclick={click} ondblclick={dblclick} onhover={hover}>
        {#snippet overlay()}
          <g transform="scale({1 / umPerPx})">
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
            {#if app.layers.outlines}
              {#each grains as g (g.id)}
                {@const own = owners.get(g.id)}
                <polygon class="grain" class:sel={selGrains.includes(g.id)} class:hov={hoverId === g.id} class:dup={!!own?.duplicate_of} class:cut={!!own?.cut && !own?.duplicate_of} points={g.polygon_um.map((p) => p.join(',')).join(' ')} data-testid={own?.duplicate_of ? 'grain-dup' : undefined}><title>{own?.duplicate_of ? ownerNote(g) : own?.cut ? 'owned here, cut by the area edge' : own ? 'owned by this area' : ''}</title></polygon>
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
                <g class="spot {s.kind}" class:sel class:hov={hoverId === s.id} data-testid="spot">
                  {#if s.kind === 'line' && s.x2_um != null && s.y2_um != null}
                    <line class="swath" x1={s.x_um} y1={s.y_um} x2={s.x2_um} y2={s.y2_um} style:stroke-width="{s.diameter_um}px" />
                    <line class="axis" x1={s.x_um} y1={s.y_um} x2={s.x2_um} y2={s.y2_um} />
                  {:else if s.kind === 'raster' && s.x2_um != null && s.y2_um != null}
                    <rect x={s.x_um} y={s.y_um} width={s.x2_um - s.x_um} height={s.y2_um - s.y_um} />
                  {:else}
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
              <circle class="ghost" cx={cursorUm[0]} cy={cursorUm[1]} r={app.diameter / 2} />
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
          </g>
        {/snippet}
      </Viewer>
    {/key}

    {#if editing}
      <input class="inline-edit" data-ui style:left="{editing.sx}px" style:top="{editing.sy}px" bind:value={editing.text} use:focusInput
        onkeydown={(e) => { if (e.key === 'Enter') commitEdit(); else if (e.key === 'Escape') editing = null; e.stopPropagation(); }} onblur={commitEdit} data-testid="rename" />
    {/if}

    <div class="glass toolbar" data-ui>
      <select bind:value={app.areaId} aria-label="Area">
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
      <FloatingPanel title={focus ? `Spot ${focus.name}` : `Grain ${focusGrain?.number}`} bind:x={panel.x} bind:y={panel.y} onclose={() => (panel.open = false)}>
          {#if focus && resultsBy[focus.id]}
          {@const r = resultsBy[focus.id]}
          <div class="resbox" data-testid="spot-results">
            <div class="resdate"><span class="num">{fmtValue(r.date)}</span> <span class="faint">{r.date?.metric ?? 'no date'} · 2σ</span></div>
            {#if r.flags.length}<div class="flags">{#each r.flags as f (f)}<span class="flag">{f}</span>{/each}</div>{/if}
            <table class="data kv">
              <tbody>
                {#each r.ratios as v (v.metric)}<tr><td>{v.metric}</td><td class="num">{fmtValue(v)}</td></tr>{/each}
                {#each r.concentrations as v (v.metric)}<tr><td>{v.metric}</td><td class="num">{fmtValue(v, 0)}</td></tr>{/each}
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
          <label class="field">Name <input type="text" value={focus.name} onchange={(e) => updateSpot(focus.id, { name: e.currentTarget.value })} data-testid="panel-name" /></label>
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
              <tr><td>grain</td><td>{focus.grain_number ?? '–'}</td></tr>
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
            title={hosted ? (focus.acquisition_id ? 'Open this acquisition in the Sessions tab' : `Search laicpmsdb for the label ${focus.name}`) : 'Available when grainatlas runs inside UPb Studio'}>Open in laicpmsdb</button>
          <div class="two">
            <button onclick={() => renameFromPattern([focus.id])}>Rename from pattern</button>
            <button class="danger" onclick={() => { app.selection = [focus.id]; deleteSelection(); }}>Delete</button>
          </div>
        {:else if focusGrain}
          <table class="data kv">
            <tbody>
              <tr><td>number</td><td>{focusGrain.number}{focusGrain.manual ? ' (manual)' : ''}</td></tr>
              <tr><td>⌀ equivalent</td><td class="num">{fmt(focusGrain.eq_diameter_um, 1)} µm</td></tr>
              <tr><td>length × width</td><td class="num">{fmt(focusGrain.major_um, 1)} × {fmt(focusGrain.minor_um, 1)} µm</td></tr>
              <tr><td>orientation</td><td class="num">{fmt(focusGrain.orientation_deg, 0)}°</td></tr>
              <tr><td>area</td><td class="num">{fmt(focusGrain.area_um2, 0)} µm²</td></tr>
              <tr><td>centroid</td><td class="num">{fmt(focusGrain.centroid_um[0], 1)}, {fmt(focusGrain.centroid_um[1], 1)} µm</td></tr>
              <tr><td>spots</td><td>{app.spots.filter((s) => s.grain_id === focusGrain.id).map((s) => s.name).join(', ') || '–'}</td></tr>
              {#if owners.get(focusGrain.id)}
                {@const own = owners.get(focusGrain.id)!}
                <tr><td>owner area</td><td data-testid="owner-area">{own.duplicate_of ? ownerNote(focusGrain) : `this area${own.cut ? ' (cut by its edge)' : ''}`}</td></tr>
              {/if}
              <tr><td>id</td><td class="mono small">{focusGrain.id}</td></tr>
            </tbody>
          </table>
        {/if}
        </fieldset>
      </FloatingPanel>
    {/if}

    <aside class="glass side" data-ui>
      <div class="tabs">
        <button aria-pressed={tab === 'spots'} onclick={() => (tab = 'spots')}>Spots <span class="faint">{spots.length}</span></button>
        <button aria-pressed={tab === 'grains'} onclick={() => (tab = 'grains')}>Grains <span class="faint">{grains.length}</span></button>
        <button aria-pressed={tab === 'refOverlay'} onclick={() => (tab = 'refOverlay')} title="Reference image (printout photo, post-laser frame)">Overlay</button>
        <button aria-pressed={tab === 'laser'} onclick={() => (tab = 'laser')} title="Laser stage registration and spot list" data-testid="tab-laser">Laser</button>
        <button aria-pressed={tab === 'link'} onclick={() => (tab = 'link')} title="Mount, atlas bundle, publishing to laicpmsdb" data-testid="tab-link">laicpmsdb</button>
      </div>
      <input type="file" accept="image/*" hidden bind:this={overlayInput} onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) loadOverlay(f); e.currentTarget.value = ''; }} />
      {#if ro}<p class="faint small-text ro-note" data-testid="read-only">Read-only: published on {app.remote ? new URL(app.remote.server).host : ''}. “Edit a copy” in the top bar makes it yours to change.</p>{/if}
      <fieldset class="plain tabbody" disabled={ro && tab !== 'link'}>
      {#if tab === 'spots'}
        <div class="two">
          <label class="field">Sample <input type="text" value={sample} onchange={(e) => setSample(e.currentTarget.value)} data-testid="sample" /></label>
          <label class="field">Pattern <input type="text" bind:value={app.settings.naming_pattern} onchange={() => app.saveSettings()} /></label>
        </div>
        <div class="presets">
          {#each app.settings.spot_presets_um as d, i (d)}
            <button class="icon" aria-pressed={app.diameter === d} title="Preset {i + 1}" onclick={() => setDiameter(d)}>{d} µm</button>
          {/each}
        </div>
        <div class="two">
          <label class="field" title="Force the {'{n}'} of the next spot (for example the Qtegra run number of a legacy session); it then counts up">Next n
            <input type="number" min="1" placeholder="auto ({nextN(app.spots)})" value={app.nextN ?? ''} oninput={(e) => (app.nextN = e.currentTarget.value === '' ? null : Math.max(1, Math.round(+e.currentTarget.value)))} />
          </label>
          <label class="check"><input type="checkbox" bind:checked={app.settings.snap} onchange={() => app.saveSettings()} /> snap (⌥ = free)</label>
        </div>
        <div class="row">
          <button onclick={() => exportSpots('csv')} disabled={!app.spots.length} data-testid="export-csv">Spots CSV</button>
          <button onclick={() => exportSpots('geojson')} disabled={!app.spots.length && !grains.length}>GeoJSON</button>
          <button onclick={() => renameFromPattern(app.selection.length ? app.selection : spots.map((s) => s.id))} disabled={!spots.length} title="Rename the selected spots (or all in this area) from the pattern">Rename</button>
          <button onclick={sendSpots} disabled={!spots.length} data-testid="send-laicpmsdb"
            title={app.server && app.project?.laicpmsdb.mount_id ? "POST this area's spots to the linked mount (with the sign-in token; downloads the JSON body when there is none)" : "Download this area's spots as the JSON body of laicpmsdb's POST /v1/mounts/{mount_id}/spots (mosaic µm)"}>Send spots to laicpmsdb</button>
        </div>
        <div class="list">
          <table class="data">
            <thead><tr><th>Name</th><th>Grain</th><th>⌀</th></tr></thead>
            <tbody>
              {#each spots as s (s.id)}
                <tr class:selected={app.selection.includes(s.id)} onclick={(e) => { app.selection = e.shiftKey ? [...app.selection, s.id] : [s.id]; panel.open = true; centerOnUm(s.x_um, s.y_um); }}>
                  <td>{s.name}</td><td class="num">{s.grain_number ?? '–'}</td><td class="num">{s.diameter_um}</td>
                </tr>
              {:else}
                <tr><td colspan="3" class="muted">Pick the Spot tool (S) and click on a grain.</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
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
          <label class="check"><input type="checkbox" bind:checked={segParams.reject_bubbles} /> drop epoxy bubbles and their bright rims</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.reject_slivers} /> drop tile-seam slivers</label>
          <label class="check"><input type="checkbox" bind:checked={segParams.reject_blocks} /> drop exposure-step tile blocks</label>
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
          <button class="danger" onclick={deleteGrains} disabled={!selGrains.length}>Delete</button>
        </div>
        <div class="row">
          <button onclick={() => renumber(false)} disabled={!grains.length}>Renumber all</button>
          <button onclick={() => renumber(true)} disabled={selGrains.length !== 1}>Renumber from here</button>
        </div>
        {#if dupCount}
          <p class="muted small-text" data-testid="dup-note">{dupCount} of {grains.length} are duplicates of grains another area owns (dashed outlines; left off print sheets).</p>
        {/if}
        <div class="list">
          <table class="data">
            <thead><tr><th>#</th><th>⌀ µm</th><th>L × W µm</th><th>spots</th><th>owner</th></tr></thead>
            <tbody>
              {#each grains as g (g.id)}
                <tr class:selected={selGrains.includes(g.id)} onclick={(e) => { selGrains = e.shiftKey ? [...selGrains, g.id] : [g.id]; app.selection = []; panel.open = true; centerOnUm(g.centroid_um[0], g.centroid_um[1], Math.max(g.bbox_um[2] - g.bbox_um[0], g.bbox_um[3] - g.bbox_um[1])); }}>
                  <td class="num">{g.number}{g.manual ? '*' : ''}</td><td class="num">{fmt(g.eq_diameter_um)}</td><td class="num">{fmt(g.major_um)} × {fmt(g.minor_um)}</td><td class="num">{app.spots.filter((s) => s.grain_id === g.id).length || ''}</td>
                  <td class="owner" title={ownerNote(g)}>{#if owners.get(g.id)?.duplicate_of}{@const o = owners.get(g.id)!}→ {areaLabel(o.owner_area)} #{app.grains[o.owner_area]?.find((q) => q.id === o.duplicate_of)?.number ?? '?'}{:else if owners.get(g.id)?.cut}cut{/if}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      </fieldset>
      <details class="layers">
        <summary>Layers</summary>
        <div class="two">
          <label class="check"><input type="checkbox" bind:checked={app.layers.outlines} /> outlines</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.numbers} /> numbers</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.spots} /> spots</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.labels} /> names and labels</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.scalebars} /> scale bars</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.results} /> results</label>
          <label class="check"><input type="checkbox" bind:checked={panel.open} /> info panel</label>
        </div>
      </details>
    </aside>

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
  .canvas {
    position: absolute;
    inset: 0;
  }
  .toolbar {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    max-width: calc(100% - 23rem);
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-2);
    align-items: center;
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
    width: 21rem;
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
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
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
  .legend {
    position: absolute;
    right: calc(21rem + 2 * var(--space-2));
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
  :global(.ga-root) .grain {
    fill: none;
    stroke: var(--ov-outline);
    stroke-width: 1.4;
    opacity: 0.85;
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
    opacity: 0.8;
    pointer-events: none;
  }
  :global(.ga-root) .drawing,
  :global(.ga-root) .measure {
    fill: color-mix(in srgb, var(--ov-measure) 12%, transparent);
    stroke: var(--ov-measure);
    stroke-width: 1.6;
    stroke-dasharray: 6 4;
  }
  :global(.ga-root) .splitline {
    stroke: var(--danger);
    stroke-width: 2;
    stroke-dasharray: 6 4;
  }
</style>
