import { ATLAS_EVENTS, isHosted, notify, viewerRegion, VIEWER_REGION_MAX_SPOTS,
  type BridgeEvent, type ViewerRegionEvent } from '@lab/bridge';
import { presentation, type PresentationState } from './presentation.ts';

export type RegionInput = Omit<ViewerRegionEvent, 'type' | 'spots'> & { spots: readonly string[] };

/** Preserve Review's first occurrence order and spelling; ids compare like focus/review ids. */
export function buildViewerRegion(input: RegionInput): ViewerRegionEvent | null {
  const spots: string[] = [], seen = new Set<string>();
  if (input.kind !== 'mount') for (const id of input.spots) {
    const key = id.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key); spots.push(id);
    if (spots.length === VIEWER_REGION_MAX_SPOTS) break;
  }
  return viewerRegion({ ...input, type: ATLAS_EVENTS.viewerRegion, spots });
}

/** Exploration only: no persistence, selection changes, drafts, journal or sync. */
export function sendViewerRegion(input: RegionInput, state: Pick<PresentationState, 'current'> = presentation,
  hosted = isHosted(), send: (event: BridgeEvent) => unknown = notify): boolean {
  if (state.current !== 'viewer' || !hosted) return false;
  const event = buildViewerRegion(input);
  if (!event) return false;
  void send(event);
  return true;
}
