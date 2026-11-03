<script lang="ts">
  // Canvas screen: the infinite canvas with grains and spots.
  // Pointer-first: navigate with trackpad or mouse, click to place, drag a spot
  // to move it, drag its ring handle to resize, double-click its name to rename;
  // the floating panel edits the hovered or selected object. Keys are extras.
  import { app, type Tool } from '../lib/store.svelte';
  import Viewer, { type CanvasPoint } from '../lib/canvas/Viewer.svelte';
  import { toScreen, type View } from '../lib/canvas/view';
  import type { Core } from '../lib/core/core';
  import type { Grain, Label, SegmentParams, Spot, SpotKind } from '../lib/core/types';
  import { download } from '../lib/files';
  import { formatName, grainAt, newId, nextN, snap, snapTargets, spotDistance, stageOf, type SnapTarget } from '../lib/spots';
  import FloatingPanel from '../components/FloatingPanel.svelte';

  let core = $state<Core | null>(null);
  app.core.then((c) => (core = c));
  let viewer = $state<ReturnType<typeof Viewer>>();
  let view = $state<View | undefined>(undefined);
  let lut = $state<Uint8Array>(Uint8Array.from({ length: 256 }, (_, i) => i));
  let cursorUm = $state<[number, number] | null>(null);
  let selGrains = $state<string[]>([]);
  let tab = $state<'spots' | 'grains'>('spots');
  let panel = $state({ x: 16, y: 150, open: true });
  let hoverId = $state<string | null>(null);
  let snapHint = $state<SnapTarget | null>(null);
  // In-progress gestures.
  let splitLine = $state<[number, number, number, number] | null>(null);
  let drawing = $state<{ kind: SpotKind | 'measure'; x: number; y: number; x2: number; y2: number } | null>(null);
  let measure = $state<[number, number, number, number] | null>(null);
  let gesture: { mode: 'move' | 'resize'; ids: string[]; startX: number; startY: number; orig: Map<string, { x: number; y: number; x2: number | null; y2: number | null; d: number }>; dirty: boolean } | null = null;
  let editing = $state<{ id: string; text: string; sx: number; sy: number } | null>(null);

  const areaId = $derived(app.areaId && app.stitched[app.areaId] ? app.areaId : (Object.keys(app.stitched)[0] ?? null));
  const area = $derived(app.project?.areas.find((a) => a.id === areaId) ?? null);
  const mosaic = $derived(areaId ? app.stitched[areaId].mosaic : null);
  const umPerPx = $derived(mosaic?.um_per_px ?? 1);
  const grains = $derived(areaId ? (app.grains[areaId] ?? []) : []);
  const spots = $derived(app.spots.filter((s) => s.area_id === areaId));
  const labels = $derived(app.labels.filter((l) => l.area_id === areaId));
  const segParams = $derived(areaId ? app.params[areaId]?.segment : null);
  const sample = $derived(area?.sample || area?.name || '');
  /** screen px → um at the current zoom */
  const k = $derived(view ? umPerPx / view.zoom : 1);
  const rotDeg = $derived(view ? (view.rot * 180) / Math.PI : 0);
  const selSpots = $derived(app.spots.filter((s) => app.selection.includes(s.id)));
  const focus = $derived.by(() => {
    const id = app.selection[0] ?? hoverId;
    return app.spots.find((s) => s.id === id) ?? null;
  });
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

  // ---- spots --------------------------------------------------------------------
  function now() {
    return new Date().toISOString();
  }
  function makeSpot(x: number, y: number, kind: SpotKind = 'spot', x2: number | null = null, y2: number | null = null): Spot {
    const g = grainAt(grains, x, y);
    const n = nextN(app.spots);
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
    if (text) download(text, `${app.pkg?.name ?? 'grainmap'}_spots.${format === 'csv' ? 'csv' : 'geojson'}`, format === 'csv' ? 'text/csv' : 'application/geo+json');
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

  function dblclick(p: CanvasPoint) {
    const x = p.x * umPerPx, y = p.y * umPerPx;
    const s = hitSpot(x, y) ?? null;
    if (s) {
      app.selection = [s.id];
      startEdit(s.id);
      return;
    }
    const l = hitLabel(x, y);
    if (l) startEdit(l.id);
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
    if (tools[kk]) app.tool = tools[kk];
    else if (/^[1-9]$/.test(e.key) && app.settings.spot_presets_um[Number(e.key) - 1]) setDiameter(app.settings.spot_presets_um[Number(e.key) - 1]);
    else if (e.key === '+' || e.key === '=') viewer?.zoomBy(1.5);
    else if (e.key === '-') viewer?.zoomBy(1 / 1.5);
    else if (e.key === '0') viewer?.fitAll();
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

{#if core && areaId && mosaic}
  <div class="canvas">
    {#key areaId}
      <Viewer bind:this={viewer} bind:view {core} {areaId} {mosaic} {lut} version={app.pixelsVersion} {cursor}
        onpress={press} ondrag={drag} onrelease={release} onclick={click} ondblclick={dblclick} onhover={hover}>
        {#snippet overlay()}
          <g transform="scale({1 / umPerPx})">
            {#if app.layers.outlines}
              {#each grains as g (g.id)}
                <polygon class="grain" class:sel={selGrains.includes(g.id)} class:hov={hoverId === g.id} points={g.polygon_um.map((p) => p.join(',')).join(' ')} />
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
        {#each Object.keys(app.stitched) as id (id)}<option value={id}>{app.pkg?.areas.find((a) => a.id === id)?.name ?? id}</option>{/each}
      </select>
      <span class="sep"></span>
      {#each toolList as t (t.id)}
        <button class="icon" aria-pressed={app.tool === t.id} title="{t.title} ({t.key})" onclick={() => (app.tool = t.id)} data-testid="tool-{t.id}">{t.label}</button>
      {/each}
      <span class="sep"></span>
      <label class="diam" title="Spot diameter (µm); 1–6 pick a preset">
        ⌀
        <select value={String(app.diameter)} onchange={(e) => setDiameter(+e.currentTarget.value)}>
          {#each [...new Set([...app.settings.spot_presets_um, app.diameter])].sort((a, b) => a - b) as d (d)}<option value={String(d)}>{d}</option>{/each}
        </select>
        <input type="number" min="1" step="0.5" value={app.diameter} onchange={(e) => setDiameter(+e.currentTarget.value)} aria-label="Diameter µm" />µm
      </label>
      <span class="sep"></span>
      <button class="icon" title="Add grain: click inside it (G)" aria-pressed={app.tool === 'grain-add'} onclick={() => (app.tool = 'grain-add')}>+Grain</button>
      <button class="icon" title="Split grain: draw a line across it (K)" aria-pressed={app.tool === 'grain-split'} onclick={() => (app.tool = 'grain-split')}>Split</button>
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

    {#if panel.open && (focus || focusGrain)}
      <FloatingPanel title={focus ? `Spot ${focus.name}` : `Grain ${focusGrain?.number}`} bind:x={panel.x} bind:y={panel.y} onclose={() => (panel.open = false)}>
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
              <tr><td>stage</td><td class="num">{st ? `${(st[0] / 1e6).toFixed(4)}, ${(st[1] / 1e6).toFixed(4)} mm` : '–'}</td></tr>
              <tr><td>created</td><td>{focus.created.replace('T', ' ').slice(0, 19)} {focus.created_by}</td></tr>
              <tr><td>id</td><td class="mono small">{focus.id}</td></tr>
            </tbody>
          </table>
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
              <tr><td>id</td><td class="mono small">{focusGrain.id}</td></tr>
            </tbody>
          </table>
        {/if}
      </FloatingPanel>
    {/if}

    <aside class="glass side" data-ui>
      <div class="tabs">
        <button aria-pressed={tab === 'spots'} onclick={() => (tab = 'spots')}>Spots <span class="faint">{spots.length}</span></button>
        <button aria-pressed={tab === 'grains'} onclick={() => (tab = 'grains')}>Grains <span class="faint">{grains.length}</span></button>
      </div>
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
        <label class="check"><input type="checkbox" bind:checked={app.settings.snap} onchange={() => app.saveSettings()} /> snap (hold ⌥ to place freely)</label>
        <div class="row">
          <button onclick={() => exportSpots('csv')} disabled={!app.spots.length} data-testid="export-csv">Spots CSV</button>
          <button onclick={() => exportSpots('geojson')} disabled={!app.spots.length && !grains.length}>GeoJSON</button>
          <button onclick={() => renameFromPattern(app.selection.length ? app.selection : spots.map((s) => s.id))} disabled={!spots.length} title="Rename the selected spots (or all in this area) from the pattern">Rename</button>
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
        <label class="field">laicpmsdb mount id <input type="text" placeholder="UUID (optional)" value={app.project?.laicpmsdb.mount_id ?? ''} onchange={(e) => setMount(e.currentTarget.value)} /></label>
      {:else if segParams}
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
            <thead><tr><th>#</th><th>⌀ µm</th><th>L × W µm</th><th>spots</th></tr></thead>
            <tbody>
              {#each grains as g (g.id)}
                <tr class:selected={selGrains.includes(g.id)} onclick={(e) => { selGrains = e.shiftKey ? [...selGrains, g.id] : [g.id]; app.selection = []; panel.open = true; centerOnUm(g.centroid_um[0], g.centroid_um[1], Math.max(g.bbox_um[2] - g.bbox_um[0], g.bbox_um[3] - g.bbox_um[1])); }}>
                  <td class="num">{g.number}{g.manual ? '*' : ''}</td><td class="num">{fmt(g.eq_diameter_um)}</td><td class="num">{fmt(g.major_um)} × {fmt(g.minor_um)}</td><td class="num">{app.spots.filter((s) => s.grain_id === g.id).length || ''}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      <details class="layers">
        <summary>Layers</summary>
        <div class="two">
          <label class="check"><input type="checkbox" bind:checked={app.layers.outlines} /> outlines</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.numbers} /> numbers</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.spots} /> spots</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.labels} /> names and labels</label>
          <label class="check"><input type="checkbox" bind:checked={app.layers.scalebars} /> scale bars</label>
          <label class="check"><input type="checkbox" bind:checked={panel.open} /> info panel</label>
        </div>
      </details>
    </aside>

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
  :global(.gm-root) .grain {
    fill: none;
    stroke: var(--ov-outline);
    stroke-width: 1.4;
    opacity: 0.85;
  }
  :global(.gm-root) .grain.hov {
    opacity: 1;
    stroke-width: 2;
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
  :global(.gm-root) .gbar text,
  :global(.gm-root) .spot text,
  :global(.gm-root) .flabel,
  :global(.gm-root) .mlabel {
    fill: var(--ov-label);
    font: 12px var(--font-sans);
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.8);
    stroke-width: 3px;
  }
  :global(.gm-root) .spot circle,
  :global(.gm-root) .spot rect {
    fill: color-mix(in srgb, var(--ov-spot) 14%, transparent);
    stroke: var(--ov-spot);
    stroke-width: 1.6;
  }
  :global(.gm-root) .spot .dot {
    fill: var(--ov-spot);
    stroke: none;
  }
  :global(.gm-root) .spot.raster rect {
    stroke: var(--ov-raster);
    fill: color-mix(in srgb, var(--ov-raster) 14%, transparent);
  }
  :global(.gm-root) .spot .swath {
    stroke: color-mix(in srgb, var(--ov-line) 35%, transparent);
    vector-effect: none;
    stroke-linecap: round;
  }
  :global(.gm-root) .spot .axis {
    stroke: var(--ov-line);
    stroke-width: 1.4;
  }
  :global(.gm-root) .spot.hov circle,
  :global(.gm-root) .spot.hov rect {
    stroke-width: 2.4;
  }
  :global(.gm-root) .spot.sel circle,
  :global(.gm-root) .spot.sel rect,
  :global(.gm-root) .spot.sel .axis {
    stroke: var(--ov-spot-selected);
    stroke-width: 2.4;
  }
  :global(.gm-root) .spot.sel text {
    font-weight: 700;
  }
  :global(.gm-root) .spot .handle {
    fill: var(--ov-spot-selected);
    stroke: var(--ov-spot);
    stroke-width: 1.5;
    cursor: nwse-resize;
  }
  :global(.gm-root) .flabel.sel {
    fill: var(--accent);
  }
  :global(.gm-root) .snap circle,
  :global(.gm-root) .snap line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  :global(.gm-root) .ghost {
    fill: none;
    stroke: var(--ov-spot);
    stroke-dasharray: 4 3;
    stroke-width: 1.2;
    opacity: 0.8;
    pointer-events: none;
  }
  :global(.gm-root) .drawing,
  :global(.gm-root) .measure {
    fill: color-mix(in srgb, var(--ov-measure) 12%, transparent);
    stroke: var(--ov-measure);
    stroke-width: 1.6;
    stroke-dasharray: 6 4;
  }
  :global(.gm-root) .splitline {
    stroke: var(--danger);
    stroke-width: 2;
    stroke-dasharray: 6 4;
  }
</style>
