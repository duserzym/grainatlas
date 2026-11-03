// Application state (Svelte 5 runes). The core holds pixels; the UI holds the
// project record, grains, spots and labels, and an undo history of edits.

import { getCore, type Core } from './core/core';
import type { AreaParams, AreaRecord, Grain, Label, PackageInfo, Progress, Project, Settings, Spot, StitchSummary, ToneParams } from './core/types';

export type Screen = 'open' | 'area' | 'canvas' | 'layout' | 'settings';
export type Tool = 'select' | 'spot' | 'line' | 'raster' | 'measure' | 'label' | 'delete' | 'grain-add' | 'grain-split';

export const DEFAULT_SETTINGS: Settings = {
  naming_pattern: '{sample}-{grain}-{n}',
  spot_presets_um: [15, 20, 25, 30, 35, 50],
  default_diameter_um: 25,
  user: '',
  theme: 'dark',
  snap: true,
};

function loadSettings(): Settings {
  try {
    const s = localStorage.getItem('grainmap.settings');
    if (s) return { ...DEFAULT_SETTINGS, ...JSON.parse(s) };
  } catch {
    /* storage may be unavailable */
  }
  return { ...DEFAULT_SETTINGS };
}

interface Snapshot {
  spots: Spot[];
  labels: Label[];
  grains: Record<string, Grain[]>;
}

class AppState {
  screen = $state<Screen>('open');
  pkg = $state<PackageInfo | null>(null);
  project = $state<Project | null>(null);
  areaId = $state<string | null>(null);
  stitched = $state<Record<string, StitchSummary>>({});
  /** Bumps when pyramid pixels change (re-stitch, re-tone): the canvas drops its tile cache. */
  pixelsVersion = $state(0);
  tone = $state<Record<string, ToneParams>>({});
  params = $state<Record<string, AreaParams>>({});
  grains = $state<Record<string, Grain[]>>({});
  spots = $state<Spot[]>([]);
  labels = $state<Label[]>([]);
  selection = $state<string[]>([]);
  hover = $state<string | null>(null);
  tool = $state<Tool>('select');
  diameter = $state(25);
  busy = $state<{ label: string; progress: Progress | null } | null>(null);
  error = $state<string | null>(null);
  settings = $state<Settings>(loadSettings());
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
      localStorage.setItem('grainmap.settings', JSON.stringify(this.settings));
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
    p['grainmap_ui'] = { grains: $state.snapshot(this.grains), spots: $state.snapshot(this.spots), labels: $state.snapshot(this.labels), tone: $state.snapshot(this.tone) };
    return p;
  }

  /** Browser: keep the latest project (without pixels) per package name for "recent projects". */
  persist() {
    queueMicrotask(() => {
      const p = this.fullProject();
      if (!p) return;
      try {
        localStorage.setItem(`grainmap.project.${p.package.name}`, JSON.stringify(p));
        const recent: { name: string; when: string; areas: number; spots: number }[] = JSON.parse(localStorage.getItem('grainmap.recent') || '[]').filter((r: { name: string }) => r.name !== p.package.name);
        recent.unshift({ name: p.package.name, when: new Date().toISOString(), areas: p.areas.length, spots: this.spots.length });
        localStorage.setItem('grainmap.recent', JSON.stringify(recent.slice(0, 12)));
      } catch {
        /* quota or private mode */
      }
    });
  }

  /** Restores UI-held parts of a saved project. */
  adopt(p: Project) {
    const ui = p['grainmap_ui'] as { grains?: Record<string, Grain[]>; spots?: Spot[]; labels?: Label[]; tone?: Record<string, ToneParams> } | undefined;
    if (ui?.grains) this.grains = ui.grains;
    if (ui?.spots) this.spots = ui.spots;
    if (ui?.labels) this.labels = ui.labels;
    if (ui?.tone) this.tone = ui.tone;
    for (const a of p.areas) this.params[a.id] = a.params;
  }
}

export const app = new AppState();
