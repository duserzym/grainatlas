import { ATLAS_EVENTS, isHosted, notify, spotAction, type BridgeEvent, type SpotActionName } from '@lab/bridge';
import { presentation, type PresentationState } from './presentation.ts';

export function sendSpotAction(action: SpotActionName, project: string | null | undefined, spot: string | null | undefined,
  state: PresentationState = presentation, hosted = isHosted(), send: (event: BridgeEvent) => unknown = notify): boolean {
  if (state.current !== 'viewer' || !hosted) return false;
  const event = spotAction({ type: ATLAS_EVENTS.spotAction, action, project, spot });
  if (!event) return false;
  void send(event);
  return true;
}
