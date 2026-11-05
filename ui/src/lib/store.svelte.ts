// Application state (Svelte 5 runes). The core holds pixels; the UI holds the
// project record, grains, spots and labels, and an undo history of edits.
//
// A window holds several projects as tabs. Each tab is an `AppState` with its own core (a
// native session in the desktop app, a wasm worker on the web); what belongs to the viewer
// rather than a project (theme, panel layout, laicpmsdb link) is shared by all tabs. `app` is
// the active tab: components read it as before and follow a tab switch. Code that awaits takes
// its tab once (`current()`) so a result lands in the tab that started the work.

import { newCore, type Core, type PackageFile, type RawImage } from './core/core';
import { PROJECT_PREFIX, readRecent, RECENT_KEY, type DeepLink } from './deeplink';
import { loadLink, resolveServer, saveLink, type LinkSettings } from './laicpmsdb';
import type { AreaParams, AreaRecord, Grain, ImageEntry, Label, LooseSpec, Ownership, PackageInfo, Progress, Project, Settings, Spot, StitchSummary, ToneParams } from './core/types';

export type Screen = 'open' | 'area' | 'canvas' | 'layout' | 'settings';
export type Tool = 'select' | 'spot' | 'line' | 'raster' | 'measure' | 'label' | 'delete' | 'grain-add' | 'grain-split' | 'grain-outline' | 'grain-brush' | 'pair';

/** A reference image (printout photo, post-laser frame) registered to an area's mosaic. */
export interface Overlay {
  name: string;
  /** Object URL of the image (not saved; the file is chosen again by name). */
  url: string | null;
  w: number;
  h: number;
  /** [image px, mosaic um] pairs. */
  pairs: [[number, number], [number, number]][];
  /** Image px → mosaic um affine (a b c d e f). */
  matrix: [number, number, number, number, number, number];
  rms_um: number;
  residuals_um?: number[];
  opacity: number;
  visible: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  naming_pattern: '{sample}-{n}',
  area_patterns: {},
  spot_presets_um: [15, 20, 25, 30, 35, 50],
  default_diameter_um: 25,
  spot_cursor_colour: 'auto',
  user: '',
  snap: true,
  outline_margin_um: 4,
  outline_smooth: false,
};

function loadSettings(): Settings {
  try {
    const s = localStorage.getItem('grainatlas.settings');
    if (s) {
      // The theme lives under its own key now (older builds saved 'dark' here by default).
      const { theme: _old, area_patterns: _patterns, ...rest } = JSON.parse(s);
      return { ...DEFAULT_SETTINGS, ...rest };
    }
  } catch {
    /* storage may be unavailable */
  }
  return { ...DEFAULT_SETTINGS };
}

export type ThemePref = 'light' | 'dark' | 'system';
const THEME_KEY = 'grainatlas.theme';

/**
 * The viewer's own theme choice: stored only when picked with the switch, so
 * nothing stored means the default: light, or the system theme when hosted by
 * UPb Studio (its module webviews cannot see the shell's choice yet). A
 * `?theme=light|dark` in the address wins for this page load.
 */
function loadTheme(): { pref: ThemePref; chosen: boolean } {
  try {
    // The address counts as a choice (UPb Studio passes its theme this way), but is not stored.
    const q = new URLSearchParams(location.search).get('theme');
    if (q === 'light' || q === 'dark' || q === 'system') return { pref: q, chosen: true };
    const s = localStorage.getItem(THEME_KEY);
    if (s === 'light' || s === 'dark' || s === 'system') return { pref: s, chosen: true };
  } catch {
    /* storage may be unavailable */
  }
  return { pref: 'light', chosen: false };
}

function systemDark() {
  try {
    return matchMedia('(prefers-color-scheme: dark)').matches;
  } catch {
    return false;
  }
}

/** Panel geometry and open states of this viewer (ux-conventions rule 7), never in a project. */
export interface LayoutPrefs {
  /** The floating spot / grain panel's position on the Canvas, px from the canvas's top-left. */
  panel: { x: number; y: number };
  /** The Canvas side panel folded to a strip. */
  sideCollapsed: boolean;
  layersOpen: boolean;
}
const LAYOUT_KEY = 'grainatlas.layout';
export const DEFAULT_LAYOUT: LayoutPrefs = { panel: { x: 16, y: 150 }, sideCollapsed: false, layersOpen: false };

function loadLayout(): LayoutPrefs {
  try {
    const v = JSON.parse(localStorage.getItem(LAYOUT_KEY) || '{}');
    return { ...DEFAULT_LAYOUT, ...v, panel: { ...DEFAULT_LAYOUT.panel, ...(v.panel ?? {}) } };
  } catch {
    return structuredClone(DEFAULT_LAYOUT);
  }
}

/** What belongs to the viewer, not to a project: the same in every tab. */
class Shared {
  /** Remembered panel geometry (Settings → Reset layout puts it back). */
  layout = $state<LayoutPrefs>(loadLayout());
  /** laicpmsdb server, token and bundle settings of this computer (Settings screen). */
  link = $state<LinkSettings>(loadLink());
  /** `?server=` of this page load (deep links from laicpmsdb); wins over Settings. */
  serverOverride = $state<string | null>(null);
  theme0 = loadTheme();
  themePref = $state<ThemePref>(this.theme0.pref);
  /** False until the viewer picks a theme (or the address names one). */
  themeChosen = $state(this.theme0.chosen);
  systemDark = $state(systemDark());
  /** `data-theme` the UPb Studio shell put on this page's <html>, when hosted. */
  hostTheme = $state<'light' | 'dark' | null>(null);
  /** True when hosted: with nothing chosen here, follow the system like the shell's "system". */
  hosted = $state(false);
}
export const shared = new Shared();

/** Live re-rendering bookkeeping of one area (actions.ts). */
export interface LiveState { seq: number; settled: number; timer: number; previewing: boolean; again: boolean; settling: Promise<void> | null }

interface Snapshot {
  spots: Spot[];
  labels: Label[];
  grains: Record<string, Grain[]>;
}

class AppState {
  screen = $state<Screen>('open');
  pkg = $state<PackageInfo | null>(null);
  /** Loose images waiting for a scale (the scale dialog is open while set). */
  pendingLoose = $state<{ name: string; images: ImageEntry[]; spec: LooseSpec | null } | null>(null);
  project = $state<Project | null>(null);
  /** Files UPb Studio handed over while a project was open: the replace dialog asks first. */
  offer = $state.raw<{ name: string; files: PackageFile[] } | null>(null);
  areaId = $state<string | null>(null);
  stitched = $state<Record<string, StitchSummary>>({});
  /** Bumps when pyramid pixels change (re-stitch, re-tone): the canvas drops its tile cache. */
  pixelsVersion = $state(0);
  /**
   * Live preview per area while a photometric control moves: the whole mosaic at pyramid
   * `level`, made for parameters newer than the pyramid when `over === pixelsVersion` (drawn
   * instead of the tiles), else a backdrop under the fresh tiles while they load.
   */
  preview = $state<Record<string, { level: number; img: RawImage; over: number } | null>>({});
  /** After Auto: the other parameter set for Before / After, and which one is shown. */
  compare = $state<Record<string, { other: AreaParams; showing: 'before' | 'after' } | null>>({});
  /** Areas with a live re-render pending or running (a change not yet in the full-resolution pyramid). */
  liveBusy = $state<Record<string, boolean>>({});
  /** Full-resolution recomputes running in the background (not modal). */
  rendering = $state(0);
  /** Pyramid level the image view draws now (previews are not finer than this). */
  drawLevel = $state(0);
  tone = $state<Record<string, ToneParams>>({});
  params = $state<Record<string, AreaParams>>({});
  grains = $state<Record<string, Grain[]>>({});
  /** Which area owns each detected grain (overlapping areas see one grain twice); null with grains in fewer than two areas. */
  ownership = $state<Ownership | null>(null);
  private ownershipTimer: ReturnType<typeof setTimeout> | null = null;
  findings = $state<import('./core/types').Finding[]>([]);
  checkLabels = $state<string[] | null>(null);
  checkLabelsError = $state<string | null>(null);
  mountLabelsCount = $state<number | null>(null);
  renamePrompt = $state<{ oldSample: string; newSample: string; answer: (choice: 'use' | 'keep' | 'cancel') => void } | null>(null);
  checksRequested = $state(false);
  checksRan = $state(false);
  checkPrompt = $state<{ summary: string; answer: (proceed: boolean) => void } | null>(null);
  spots = $state<Spot[]>([]);
  overlays = $state<Record<string, Overlay>>({});
  /** Set to force the next spot's {n} (legacy run numbers); null = continue. */
  nextN = $state<number | null>(null);
  labels = $state<Label[]>([]);
  selection = $state<string[]>([]);
  hover = $state<string | null>(null);
  tool = $state<Tool>('select');
  diameter = $state(25);
  busy = $state<{ label: string; progress: Progress | null } | null>(null);
  error = $state<string | null>(null);
  notice = $state<string | null>(null);
  settings = $state<Settings>(loadSettings());
  get layout(): LayoutPrefs {
    return shared.layout;
  }
  set layout(v: LayoutPrefs) {
    shared.layout = v;
  }
  get link(): LinkSettings {
    return shared.link;
  }
  set link(v: LinkSettings) {
    shared.link = v;
  }
  get serverOverride(): string | null {
    return shared.serverOverride;
  }
  set serverOverride(v: string | null) {
    shared.serverOverride = v;
  }
  /** Results of the linked mount (GET /v1/mounts/{id}/results), in memory only, never saved. */
  results = $state<{ key: string; data: import('./results').MountResults | null; at: string | null; error: string | null; loading: boolean } | null>(null);
  /** The results layer colours spots by this: 'date' (preferred) or a metric code. */
  colourBy = $state('date');
  /** A published bundle opened from a server (pixels from its tiles); null for local projects. */
  remote = $state<import('./remote').Remote | null>(null);
  remoteTiles: import('./remote').RemoteTiles | null = null;
  /** True while a bundle from a server is shown read-only (before "Edit a copy"). */
  get readOnly(): boolean {
    return !!this.remote?.readOnly;
  }
  /** The last publish of this session, with the server's full answer. */
  lastPublish = $state<(import('./core/types').Published & { response: Record<string, unknown> }) | null>(null);
  get themePref(): ThemePref {
    return shared.themePref;
  }
  get themeChosen(): boolean {
    return shared.themeChosen;
  }
  get systemDark(): boolean {
    return shared.systemDark;
  }
  set systemDark(v: boolean) {
    shared.systemDark = v;
  }
  get hostTheme(): 'light' | 'dark' | null {
    return shared.hostTheme;
  }
  set hostTheme(v: 'light' | 'dark' | null) {
    shared.hostTheme = v;
  }
  get hosted(): boolean {
    return shared.hosted;
  }
  set hosted(v: boolean) {
    shared.hosted = v;
  }

  /** The theme in effect: the shell's when it sets one, else the choice here, else light (system when hosted). */
  get theme(): 'light' | 'dark' {
    if (this.hostTheme) return this.hostTheme;
    const pref = !this.themeChosen && this.hosted ? 'system' : this.themePref;
    return pref === 'system' ? (this.systemDark ? 'dark' : 'light') : pref;
  }

  setTheme(p: ThemePref) {
    shared.themePref = p;
    shared.themeChosen = true;
    try {
      localStorage.setItem(THEME_KEY, p);
    } catch {
      /* ignore */
    }
  }
  /** A deep link (`?project=&spot=`) waiting for its package to be opened; `name` is the package, null when unknown here. */
  deepLink = $state<(DeepLink & { name: string | null }) | null>(null);
  /** Spot the canvas should center on once its view is ready (deep links). */
  focusSpot = $state<string | null>(null);
  layers = $state({ mosaic: true, outlines: true, artifacts: true, numbers: false, spots: true, labels: true, scalebars: false, residuals: false, results: true });
  private undoStack: Snapshot[] = [];
  private redoStack: Snapshot[] = [];
  canUndo = $state(false);
  canRedo = $state(false);
  /** This tab's number (the desktop session is keyed by it). */
  readonly id: number;
  core: Promise<Core>;
  /** Where the project of this tab lives on disk (desktop): a `.gaproj` file or a
   * `.grainatlas/` folder; Save writes there. Null for a package opened on its own. */
  projectPath = $state<{ kind: 'file' | 'folder'; path: string } | null>(null);
  /** Changes since the project was opened or last saved (the tab's dot). */
  dirty = $state(false);
  /** Live re-rendering per area, grains last handed to the core, and the live keys App.svelte
   * compares (actions.ts, App.svelte): per tab, since two projects can have the same area ids. */
  live: Record<string, LiveState> = {};
  syncedGrains: Record<string, string> = {};
  liveKeys: Record<string, string> = {};
  /** The Canvas view of this tab (restored when the tab is shown again). */
  savedView: { areaId: string; view: import('./canvas/view').View } | null = null;

  constructor(id: number) {
    this.id = id;
    this.core = newCore(id);
  }

  /** The tab's name: the project, else the package or the published bundle. */
  get title(): string {
    return this.remote?.atlas.name || this.project?.name || this.pkg?.name || this.pendingLoose?.name || 'New tab';
  }

  /** Nothing open: a drop or a command-line path may open here instead of in a new tab. */
  get empty(): boolean {
    return !this.pkg && !this.project && !this.remote && !this.pendingLoose && !this.busy;
  }

  get area(): AreaRecord | null {
    return this.project?.areas.find((a) => a.id === this.areaId) ?? null;
  }

  get areaMeta() {
    return this.pkg?.areas.find((a) => a.id === this.areaId) ?? null;
  }

  /** The mount whose results are shown: the one the server named for a published bundle
   * (`?mount=`), else the project's link. */
  get mountId(): string | null {
    return this.remote?.mount ?? this.project?.laicpmsdb.mount_id ?? null;
  }

  /** The laicpmsdb server in effect, or null. */
  get server(): string | null {
    return resolveServer(this.serverOverride, this.link.server);
  }

  saveLayout() {
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify($state.snapshot(this.layout)));
    } catch {
      /* ignore */
    }
  }

  /** Forgets panel positions and open states; the desktop window goes back to 1280 x 800, centred. */
  async resetLayout() {
    try {
      localStorage.removeItem(LAYOUT_KEY);
    } catch {
      /* ignore */
    }
    this.layout = structuredClone(DEFAULT_LAYOUT);
    await (await this.core).resetWindow();
  }

  saveLink() {
    saveLink($state.snapshot(this.link) as LinkSettings);
  }

  saveSettings() {
    try {
      const { area_patterns: _patterns, ...defaults } = this.settings;
      localStorage.setItem('grainatlas.settings', JSON.stringify(defaults));
    } catch {
      /* ignore */
    }
  }

  async run<T>(label: string, f: (onProgress: (p: Progress) => void) => Promise<T>): Promise<T | undefined> {
    this.busy = { label, progress: null };
    this.error = null;
    try {
      return await f((p) => {
        if (this.busy) this.busy = { label, progress: p };
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg !== 'cancelled') this.error = msg;
      return undefined;
    } finally {
      this.busy = null;
    }
  }

  async cancel() {
    (await this.core).cancel();
  }

  // ---- undo / redo over spots, labels and grains ----------------------------
  private snap(): Snapshot {
    return { spots: $state.snapshot(this.spots) as Spot[], labels: $state.snapshot(this.labels) as Label[], grains: $state.snapshot(this.grains) as Record<string, Grain[]> };
  }
  /** Call before every edit. */
  checkpoint() {
    this.undoStack.push(this.snap());
    if (this.undoStack.length > 200) this.undoStack.shift();
    this.redoStack = [];
    this.canUndo = true;
    this.canRedo = false;
    this.persist();
  }
  private restore(s: Snapshot) {
    this.spots = s.spots;
    this.labels = s.labels;
    this.grains = s.grains;
    this.selection = this.selection.filter((id) => this.spots.some((x) => x.id === id) || this.labels.some((x) => x.id === id));
  }
  undo() {
    const s = this.undoStack.pop();
    if (!s) return;
    this.redoStack.push(this.snap());
    this.restore(s);
    this.canUndo = this.undoStack.length > 0;
    this.canRedo = true;
    this.persist();
  }
  redo() {
    const s = this.redoStack.pop();
    if (!s) return;
    this.undoStack.push(this.snap());
    this.restore(s);
    this.canUndo = true;
    this.canRedo = this.redoStack.length > 0;
    this.persist();
  }

  /** Recomputes cross-area ownership shortly after the grains change (in the background). */
  refreshOwnership() {
    if (this.ownershipTimer) clearTimeout(this.ownershipTimer);
    this.ownershipTimer = setTimeout(async () => {
      const grains = $state.snapshot(this.grains) as Record<string, Grain[]>;
      if (Object.values(grains).filter((g) => g.length).length < 2 || this.readOnly) {
        this.ownership = null;
        return;
      }
      try {
        this.ownership = await (await this.core).ownership(grains);
      } catch (e) {
        // Areas without stage positions have no ownership.
        console.warn('ownership:', e);
        this.ownership = null;
      }
    }, 400);
  }

  /** The project as saved: core record plus UI-held grains, spots, labels and settings. */
  fullProject(): Project | null {
    if (!this.project) return null;
    const p = $state.snapshot(this.project) as Project;
    p.settings = $state.snapshot(this.settings) as Settings;
    const overlays = Object.fromEntries(Object.entries($state.snapshot(this.overlays)).map(([k, o]) => [k, { ...o, url: null }]));
    p['grainatlas_ui'] = { grains: $state.snapshot(this.grains), spots: $state.snapshot(this.spots), labels: $state.snapshot(this.labels), tone: $state.snapshot(this.tone), overlays };
    return p;
  }

  /** Browser: keep the latest project (without pixels) per package name for "recent projects". */
  persist() {
    // A bundle shown read-only from a server is not this browser's project.
    if (this.readOnly) return;
    this.dirty = true;
    queueMicrotask(() => {
      const p = this.fullProject();
      if (!p) return;
      try {
        localStorage.setItem(PROJECT_PREFIX + p.package.name, JSON.stringify(p));
        // The id lets a deep link (?project=<uuid>) find the package name.
        const recent = readRecent().filter((r) => r.name !== p.package.name);
        recent.unshift({ id: p.uuid, name: p.package.name, when: new Date().toISOString(), areas: p.areas.length, spots: this.spots.length });
        localStorage.setItem(RECENT_KEY, JSON.stringify(recent.slice(0, 12)));
      } catch {
        /* quota or private mode */
      }
    });
  }

  /** Restores UI-held parts of a saved project. */
  adopt(p: Project) {
    this.settings.naming_pattern = p.settings?.naming_pattern ?? this.settings.naming_pattern;
    this.settings.area_patterns = p.settings?.area_patterns ?? {};
    this.findings = [];
    this.checkLabels = null;
    this.checkLabelsError = null;
    this.mountLabelsCount = null;
    this.checksRan = false;
    const ui = p['grainatlas_ui'] as { grains?: Record<string, Grain[]>; spots?: Spot[]; labels?: Label[]; tone?: Record<string, ToneParams>; overlays?: Record<string, Overlay> } | undefined;
    if (ui?.overlays) this.overlays = ui.overlays;
    if (ui?.grains) this.grains = ui.grains;
    if (ui?.spots) this.spots = ui.spots;
    if (ui?.labels) this.labels = ui.labels;
    if (ui?.tone) this.tone = ui.tone;
    for (const a of p.areas) this.params[a.id] = a.params;
  }
}

export type { AppState };

/** The tabs of this window, in their order, and the active one. */
class Tabs {
  list = $state.raw<AppState[]>([]);
  activeId = $state(0);
  private next = 1;

  constructor() {
    this.add();
  }

  get active(): AppState {
    return this.list.find((t) => t.id === this.activeId) ?? this.list[0];
  }

  /** A new, empty tab after the active one (or at the end), made active. */
  add(): AppState {
    const t = new AppState(this.next++);
    const i = this.list.findIndex((x) => x.id === this.activeId);
    const list = [...this.list];
    list.splice(i < 0 ? list.length : i + 1, 0, t);
    this.list = list;
    this.activeId = t.id;
    return t;
  }

  select(id: number) {
    if (this.list.some((t) => t.id === id)) this.activeId = id;
  }

  /** Cmd+1..8 select that tab, Cmd+9 the last (as in browsers). */
  selectIndex(i: number) {
    const t = i >= 8 ? this.list[this.list.length - 1] : this.list[i];
    if (t) this.activeId = t.id;
  }

  /** Moves a tab to position `to` (drag and drop in the tab bar). */
  move(id: number, to: number) {
    const list = this.list.filter((t) => t.id !== id);
    const t = this.list.find((x) => x.id === id);
    if (!t) return;
    list.splice(Math.max(0, Math.min(to, list.length)), 0, t);
    this.list = list;
  }

  /** Drops a tab and its core (the caller asked about unsaved changes). The last tab is replaced
   * by an empty one: the window always has a tab. */
  remove(id: number) {
    const i = this.list.findIndex((t) => t.id === id);
    if (i < 0) return;
    const [t] = this.list.slice(i, i + 1);
    const list = this.list.filter((x) => x.id !== id);
    this.list = list;
    if (this.activeId === id) {
      if (list.length) this.activeId = list[Math.min(i, list.length - 1)].id;
      else this.add();
    }
    void t.core.then((c) => c.dispose()).catch(() => {});
  }
}

export const tabs = new Tabs();

/** The active tab, read once: what an action that awaits keeps working on. */
export function current(): AppState {
  return tabs.active;
}

/**
 * The active tab, followed live: every property read goes to the tab active at that moment (and
 * Svelte tracks the switch), methods run on that tab.
 */
export const app: AppState = new Proxy({} as AppState, {
  get(_, k) {
    const t = tabs.active;
    const v = Reflect.get(t, k, t);
    return typeof v === 'function' ? v.bind(t) : v;
  },
  set(_, k, v) {
    const t = tabs.active;
    return Reflect.set(t, k, v, t);
  },
  has(_, k) {
    return k in tabs.active;
  },
});
