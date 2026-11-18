/**
 * Grains drawn on the canvas. One set drives the outlines, numbers and scale
 * bars, so hiding a grain (neighbouring-area copies switched off, or spots
 * left out of a Review subset) hides every piece of it.
 */
export function shownGrains<G extends { id: string }>(
  grains: G[],
  isNeighbourCopy: (g: G) => boolean,
  showNeighbours: boolean,
  reviewVisible: Set<string> | null,
  spots: { id: string; grain_id?: string | null }[],
): G[] {
  const visible = showNeighbours ? grains : grains.filter((g) => !isNeighbourCopy(g));
  if (!reviewVisible) return visible;
  const withSpot = new Set<string>();
  for (const s of spots) if (s.grain_id && reviewVisible.has(s.id)) withSpot.add(s.grain_id);
  return visible.filter((g) => withSpot.has(g.id));
}
