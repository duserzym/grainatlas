import type { Project } from './core/types.ts';
import { canonicalJson } from './sync-hash.ts';

/** Bundle pixels depend on mosaics, stitching/photometric/blend controls and display tone.
 * Excludes segmentation, spots, annotations and print settings. Published images can carry an additional tone overlay. */
export function pixelSignature(project: Project): string {
  const ui = project.grainatlas_ui as { tone?: Record<string, unknown>; bundle_tone?: Record<string, { curve: unknown; display: unknown }> } | undefined;
  return canonicalJson(project.areas.filter(a => a.mosaic).map(a => ({ id: a.id, sha256: a.mosaic?.sha256, w: a.mosaic?.w, h: a.mosaic?.h, um_per_px: a.mosaic?.um_per_px, stitch: a.params.stitch, photometric: a.params.photometric, blend: a.params.blend, tone: ui?.tone?.[a.id] ?? a.params.tone, bundle_tone: ui?.bundle_tone?.[a.id] ? { curve: ui.bundle_tone[a.id].curve, display: ui.bundle_tone[a.id].display } : null })).sort((a, b) => a.id.localeCompare(b.id)));
}
