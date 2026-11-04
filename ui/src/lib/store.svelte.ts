// Application state (Svelte 5 runes). The core holds pixels; the UI holds the
// project record, grains, spots and labels, and an undo history of edits.

import { getCore, type Core } from './core/core';
import { PROJECT_PREFIX, readRecent, RECENT_KEY, type DeepLink } from './deeplink';
import type { AreaParams, AreaRecord, Grain, ImageEntry, Label, LooseSpec, PackageInfo, Progress, Project, Settings, Spot, StitchSummary, ToneParams } from './core/types';

export type Screen = 'open' | 'area' | 'canvas' | 'layout' | 'settings';
export type Tool = 'select' | 'spot' | 'line' | 'raster' | 'measure' | 'label' | 'delete' | 'grain-add' | 'grain-split' | 'pair';

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
  naming_pattern: '{sample}-{grain}-{n}',
  spot_presets_um: [15, 20, 25, 30, 35, 50],
  default_diameter_um: 25,
  user: '',
  snap: true,
};

function loadSettings(): Settings {
  try {
    const s = localStorage.getItem('grainatlas.settings');
    if (s) {
      // The theme lives under its own key now (older builds saved 'dark' here by default).
      const { theme: _old, ...rest } = JSON.parse(s);
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
function loadTheme(): { pref: ThemePref; stored: boolean } {
  try {
    const q = new URLSearchParams(location.search).get('theme');
    if (q === 'light' || q === 'dark' || q === 'system') return { pref: q, stored: false };
    const s = localStorage.getItem(THEME_KEY);
    if (s === 'light' || s === 'dark' || s === 'system') return { pref: s, stored: true };
  } catch {
    /* storage may be unavailable */
  }
  return { pref: 'light', stored: false };
}

function systemDark() {
  try {
    return matchMedia('(prefers-color-scheme: dark)').matches;
  } catch {
    return false;
  }
}

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
  areaId = $state<string | null>(null);
  stitched = $state<Record<string, StitchSummary>>({});
  /** Bumps when pyramid pixels change (re-stitch, re-tone): the canvas drops its tile cache. */
  pixelsVersion = $state(0);
  tone = $state<Record<string, ToneParams>>({});
  params = $state<Record<string, AreaParams>>({});
  grains = $state<Record<string, Grain[]>>({});
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
  private theme0 = loadTheme();
  themePref = $state<ThemePref>(this.theme0.pref);
  /** False until the viewer picks a theme (or the address names one). */
  themeChosen = $state(this.theme0.stored);
  systemDark = $state(systemDark());
  /** `data-theme` the UPb Studio shell put on this page's <html>, when hosted. */
  hostTheme = $state<'light' | 'dark' | null>(null);
  /** True when hosted: with nothing chosen here, follow the system like the shell's "system". */
  hosted = $state(false);

  /** The theme in effect: the shell's when it sets one, else the choice here, else light (system when hosted). */
  get theme(): 'light' | 'dark' {
    if (this.hostTheme) return this.hostTheme;
    const pref = !this.themeChosen && this.hosted ? 'system' : this.themePref;
    return pref === 'system' ? (this.systemDark ? 'dark' : 'light') : pref;
  }

  setTheme(p: ThemePref) {
    this.themePref = p;
    this.themeChosen = true;
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
  layers = $state({ mosaic: true, outlines: true, numbers: true, spots: true, labels: true, scalebars: false, residuals: false });
  private undoStack: Snapshot[] = [];
  private redoStack: Snapshot[] = [];
  canUndo = $state(false);
  canRedo = $state(false);
  core: Promise<Core> = getCore();

  get area(): AreaRecord | null {
    return this.project?.areas.find((a) => a.id === this.areaId) ?? null;
  }

  get areaMeta() {
    return this.pkg?.areas.find((a) => a.id === this.areaId) ?? null;
  }

  saveSettings() {
    try {
      localStorage.setItem('grainatlas.settings', JSON.stringify(this.settings));
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
    const ui = p['grainatlas_ui'] as { grains?: Record<string, Grain[]>; spots?: Spot[]; labels?: Label[]; tone?: Record<string, ToneParams>; overlays?: Record<string, Overlay> } | undefined;
    if (ui?.overlays) this.overlays = ui.overlays;
    if (ui?.grains) this.grains = ui.grains;
    if (ui?.spots) this.spots = ui.spots;
    if (ui?.labels) this.labels = ui.labels;
    if (ui?.tone) this.tone = ui.tone;
    for (const a of p.areas) this.params[a.id] = a.params;
  }
}

export const app = new AppState();
