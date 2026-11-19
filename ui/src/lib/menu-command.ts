export type CommandSource = 'key' | 'menu';

/** Pair keydown/menu deliveries in either order, without swallowing rapid presses or repeats. */
export function commandDeduper(now = () => performance.now()): (what: string, source: CommandSource) => boolean {
  let pending: { what: string; source: CommandSource; at: number }[] = [];
  return (what, source) => {
    const at = now();
    pending = pending.filter(event => at - event.at < 400);
    const pair = pending.findIndex(event => event.what === what && event.source !== source);
    if (pair >= 0) { pending.splice(pair, 1); return false; }
    pending.push({ what, source, at });
    return true;
  };
}
