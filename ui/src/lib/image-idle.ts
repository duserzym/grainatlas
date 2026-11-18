export const IMAGE_IDLE_MS = 10_000;
export interface ImageGates {
  elementsIdle: boolean; attention: boolean; visible: boolean; loading: boolean;
}
/** Shared by the initial start and the boundary before each file/part/request. */
export function imagesMayRun(lastEdit: number, now: number, gates: ImageGates) {
  return now - lastEdit >= IMAGE_IDLE_MS && gates.elementsIdle && !gates.attention && gates.visible && !gates.loading;
}
