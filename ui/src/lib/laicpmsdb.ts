// Links and hand-overs to laicpmsdb (the sessions database), through the UPb
// Studio shell. Not hosted, the bridge's open() resolves false.

import type { Spot } from './core/types';

/** The laicpmsdb link for a spot: its acquisition when known, else a search by label (= spot name). */
export function spotLink(s: Pick<Spot, 'name' | 'acquisition_id'>): string {
  const acq = s.acquisition_id?.trim();
  if (acq) return `laicpmsdb://acquisition/${encodeURIComponent(acq)}`;
  return `laicpmsdb://search?label=${encodeURIComponent(s.name)}`;
}

/** Body of laicpmsdb's `POST /v1/mounts/{mount_id}/spots` (mount id goes in the path). */
export interface MountSpots {
  project_id: string;
  sample: string;
  spots: { id: string; name: string; grain: string; x_um: number; y_um: number; diameter_um: number; kind: 'spot' }[];
}

/**
 * The point spots of one area in mosaic micrometres (y down, origin at the
 * mosaic's top-left). Lines and rasters are not part of the endpoint yet and
 * are left out; `skipped` counts them.
 */
export function mountSpots(projectId: string, sample: string, spots: Spot[]): { body: MountSpots; skipped: number } {
  const points = spots.filter((s) => s.kind === 'spot');
  const round = (v: number) => Math.round(v * 1000) / 1000;
  return {
    body: {
      project_id: projectId,
      sample,
      spots: points.map((s) => ({
        id: s.id,
        name: s.name,
        grain: s.grain_number == null ? '' : String(s.grain_number),
        x_um: round(s.x_um),
        y_um: round(s.y_um),
        diameter_um: s.diameter_um,
        kind: 'spot',
      })),
    },
    skipped: spots.length - points.length,
  };
}
