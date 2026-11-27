/** Manual mode creates no polling timer. Callers also gate wake events and in-flight ticks. */
export function startSyncPolling(enabled: boolean, milliseconds: number, check: () => void): () => void {
  if (!enabled) return () => {};
  const timer = setInterval(check, milliseconds);
  return () => clearInterval(timer);
}
