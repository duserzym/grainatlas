import { ATLAS_EVENTS, isHosted, notify, viewerInset, type BridgeEvent, type ViewerInset } from '@lab/bridge';
import { presentation, type PresentationState } from './presentation.ts';

/**
 * The user resized, collapsed or expanded the viewer's mount inset: keep it in memory and tell
 * UPb Lab, which remembers it (bridge v5.1). Viewer presentation only; never called when an
 * inset received from the shell is applied. Returns whether an event was sent.
 */
export function sendViewerInset(inset: ViewerInset, state: PresentationState = presentation,
  hosted = isHosted(), send: (event: BridgeEvent) => unknown = notify): boolean {
  if (state.current !== 'viewer') return false;
  state.keepInset(inset);
  if (!hosted) return false;
  const event = viewerInset({ type: ATLAS_EVENTS.viewerInset, width: inset.width, collapsed: inset.collapsed });
  if (!event) return false;
  void send(event);
  return true;
}
