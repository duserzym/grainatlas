import type { AreaParams, Project, ToneParams } from './core/types.ts';

export interface BundleSource { rawCurve: ToneParams; rawDisplay: ToneParams; sha256: string }
export interface BundleTone { curve: ToneParams; display: ToneParams; on_sha256: string }
export type BundleSources = Record<string, BundleSource>;
export type BundleTones = Record<string, BundleTone>;

export function neutralTone(): ToneParams {
  return { low_pct: 0, high_pct: 100, black: 0, white: 1, gamma: 1, clahe: false, clahe_tile: 64, clahe_clip: 2.5,
    grain_norm: { enabled: false, strength: 0.8, reference: null, feather_um: 4 } };
}

/** Display handles never alter the base curve of a published source. */
export function curveTone(tone: ToneParams): ToneParams {
  return { ...tone, black: 0, white: 1, gamma: 1 };
}

// Match the core neutral value, including defaults of the disabled controls.
export function isNeutral(t: ToneParams): boolean {
  return t.low_pct === 0 && t.high_pct === 100 && t.black === 0 && t.white === 1 && t.gamma === 1 && !t.clahe && t.clahe_tile === 64 && t.clahe_clip === 2.5 && !t.grain_norm.enabled && Math.fround(t.grain_norm.strength) === Math.fround(0.8) && t.grain_norm.reference === null && t.grain_norm.feather_um === 4;
}

export function bundlePixels(app: { remote: { readOnly: boolean } | null; bundle: BundleSources | null; readOnly: boolean }): boolean {
  return !!app.remote && !app.readOnly && !app.remote.readOnly && app.bundle !== null;
}

export function storedBundleTones(project: Project): BundleTones {
  return (project.grainatlas_ui as { bundle_tone?: BundleTones } | undefined)?.bundle_tone ?? {};
}

export function overlayFor(project: Project, id: string, sha256: string): { curve: ToneParams; display: ToneParams } {
  const overlay = storedBundleTones(project)[id];
  return structuredClone(overlay?.on_sha256 === sha256 ? overlay : { curve: neutralTone(), display: neutralTone() });
}

/** Called by fullProject after collecting UI data. Raw record and identifiers stay intact. */
export function saveBundleTones(project: Project, bundle: BundleSources | null, params: Record<string, AreaParams>, tone: Record<string, ToneParams>, adopted: Project): void {
  const ui = project.grainatlas_ui as { tone: Record<string, ToneParams>; bundle_tone?: BundleTones };
  const rawDisplays = (adopted.grainatlas_ui as { tone?: Record<string, ToneParams> } | undefined)?.tone ?? {};
  ui.tone = { ...structuredClone(rawDisplays), ...ui.tone };
  const overlays = structuredClone(storedBundleTones(adopted));
  if (bundle) for (const a of project.areas) {
    const source = bundle[a.id];
    if (!source) continue;
    a.params.tone = structuredClone(source.rawCurve);
    ui.tone[a.id] = structuredClone(source.rawDisplay);
    const curve = params[a.id].tone;
    const display = tone[a.id] ?? curve;
    if (isNeutral(curve) && isNeutral(display)) delete overlays[a.id];
    else overlays[a.id] = { curve: structuredClone(curve), display: structuredClone(display), on_sha256: source.sha256 };
  }
  if (Object.keys(overlays).length) ui.bundle_tone = overlays;
  else delete ui.bundle_tone;
}
