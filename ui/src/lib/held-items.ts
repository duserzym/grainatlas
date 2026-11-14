// Show a held-back item on the canvas with the same selection and framing as a table click.
import { stitch } from './actions';
import type { AppState } from './store.svelte';
import type { Kind } from './sync';

/** The area that holds this element here, or null when it is not in this copy (a kept delete). */
export function elementArea(app: Pick<AppState, 'spots' | 'labels' | 'grains'>, kind: Kind, id: string): string | null {
  if (kind === 'spots') return app.spots.find(s => s.id === id)?.area_id ?? null;
  if (kind === 'labels') return app.labels.find(l => l.id === id)?.area_id ?? null;
  return Object.entries(app.grains).find(([, list]) => list.some(g => g.id === id))?.[0] ?? null;
}

export async function showElement(app: AppState, kind: Kind, id: string): Promise<boolean> {
  const area = elementArea(app, kind, id);
  if (!area) return false;
  if (!app.stitched[area] && app.pkg && !(await stitch(area, app))) return false;
  app.screen = 'canvas';
  app.areaId = area;
  if (kind === 'grains') { app.selection = []; app.focusGrain = { area, id, flash: true }; }
  else { app.selection = [id]; if (kind === 'spots') app.focusSpot = id; }
  return true;
}
