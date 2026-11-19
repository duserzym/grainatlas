import type { Project } from './core/types.ts';
import { canonicalJson } from './sync-hash.ts';

/** Published pixels depend on mosaics and display tone. A re-stitch or correction
 * changes the mosaic hash; its controls alone do not change published images.
 * Excludes segmentation, spots, annotations and print settings. Published images can carry an additional tone overlay. */
export function pixelSignature(project: Project): string {
  const ui = project.grainatlas_ui as { tone?: Record<string, unknown>; bundle_tone?: Record<string, { curve: unknown; display: unknown }> } | undefined;
  return canonicalJson(project.areas.filter(a => a.mosaic).map(a => ({ id: a.id, ...(project.image_edits ? { image_edits: project.image_edits.areas[a.id] ?? null } : {}), sha256: a.mosaic?.sha256, w: a.mosaic?.w, h: a.mosaic?.h, um_per_px: a.mosaic?.um_per_px, tone: ui?.tone?.[a.id] ?? a.params.tone, bundle_tone: ui?.bundle_tone?.[a.id] ? { curve: ui.bundle_tone[a.id].curve, display: ui.bundle_tone[a.id].display } : null })).sort((a, b) => a.id.localeCompare(b.id)));
}

/** Match the core's f32 values without changing integer controls or text. */
function f32Numbers(value: unknown): unknown {
  if (typeof value === 'number') return Number.isInteger(value) ? value : Math.fround(value);
  if (Array.isArray(value)) return value.map(f32Numbers);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, v]) => [key, f32Numbers(v)]));
  return value;
}

/** Read both legacy signatures (with processing controls) and current keys. */
export function comparablePixelKey(stored: string | null | undefined): string | null {
  if (stored == null) return null;
  try {
    const areas: unknown = JSON.parse(stored);
    if (!Array.isArray(areas)) return null;
    const keys = areas.map(a => {
      if (!a || typeof a.id !== 'string' || typeof a.sha256 !== 'string'
        || typeof a.w !== 'number' || typeof a.h !== 'number' || typeof a.um_per_px !== 'number'
        || !a.tone || typeof a.tone !== 'object'
        || (a.bundle_tone != null && (!a.bundle_tone.curve || !a.bundle_tone.display))) throw new Error('Invalid pixel key');
      return { id: a.id, ...('image_edits' in a ? { image_edits: a.image_edits } : {}), sha256: a.sha256, w: a.w, h: a.h, um_per_px: f32Numbers(a.um_per_px), tone: f32Numbers(a.tone),
        bundle_tone: a.bundle_tone ? f32Numbers({ curve: a.bundle_tone.curve, display: a.bundle_tone.display }) : null };
    });
    return canonicalJson(keys.sort((a, b) => a.id.localeCompare(b.id)));
  } catch { return null; }
}
