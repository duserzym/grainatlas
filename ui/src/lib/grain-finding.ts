import type { AppState } from './store.svelte.ts';
import type { AreaParams, Grain, SegmentParams } from './core/types.ts';
import { relinkSpots, linkUnlinkedSpots } from './spots.ts';

/** Keep the operation busy until its grains, spot links and stored copy agree. */
export async function findGrainsInArea(app: AppState, id: string, missing = false) {
  if (app.readOnly || app.remote || app.busy || !app.params[id] || (missing && app.grains[id] === undefined)) return;
  const core = await app.core;
  if (app.busy) return;
  const params = JSON.parse(JSON.stringify(app.params[id].segment)) as SegmentParams;
  const previous = JSON.parse(JSON.stringify(app.grains[id] ?? [])) as Grain[];
  await app.run(missing ? 'Finding missed grains' : 'Finding grains', async () => {
    const outline = { margin_um: app.settings.outline_margin_um, smooth: app.settings.outline_smooth };
    const grains = await (missing ? core.segmentMissing(id, params, previous, outline) : core.segment(id, params, previous, outline));
    app.checkpoint();
    app.grains = { ...app.grains, [id]: grains };
    const added = grains.slice(previous.length);
    app.spots = missing ? linkUnlinkedSpots(app.spots, id, added) : relinkSpots(app.spots, id, grains);
    app.persist();
    if (missing) {
      const flagged = added.filter(g => g.artifact).length;
      const real = added.length - flagged;
      app.notice = added.length ? `Added ${real} ${real === 1 ? 'grain' : 'grains'}${flagged ? ` and ${flagged} flagged` : ''}. Review the dashed outlines; ⌘Z undoes.` : 'No missed grains found.';
    }
  });
}

/**
 * Fill areas never segmented first; otherwise find again in every project area, in order.
 * Saved placements render as saved; areas without a mosaic are stitched before finding grains.
 * One checkpoint restores all grain lists and spot links together.
 */
export async function findGrainsInAreas(app: AppState) {
  if (app.readOnly || app.remote || app.busy) return;
  const core = await app.core;
  const areas = app.project?.areas ?? [];
  const missing = areas.filter((a) => app.grains[a.id] === undefined);
  const todo = missing.length ? missing : areas;
  if (!todo.length) return;
  let checkpointed = false;
  const result = await app.run('Finding grains in all areas', async (progress) => {
    const empty: string[] = [];
    let count = 0;
    for (const a of todo) {
      const id = a.id;
      const params = JSON.parse(JSON.stringify(app.params[id])) as AreaParams;
      params.tone = JSON.parse(JSON.stringify(app.tone[id] ?? params.tone));
      params.segment.signal ??= null;
      if (!app.stitched[id]) {
        await core.setGrains(id, JSON.parse(JSON.stringify(app.grains[id] ?? [])) as Grain[]);
        // Match restoreAreas: keep the stitch settings that solved the saved placement.
        if (a.placement) params.stitch = JSON.parse(JSON.stringify(a.params.stitch));
        const rendered = a.placement
          ? await core.recompute(id, params, progress)
          : await core.stitch(id, params, progress);
        app.params = { ...app.params, [id]: params };
        app.project = JSON.parse(await core.projectJson());
        app.stitched = { ...app.stitched, [id]: rendered };
        app.preview = { ...app.preview, [id]: null };
        app.pixelsVersion++;
      }
      const prev = JSON.parse(JSON.stringify(app.grains[id] ?? [])) as Grain[];
      const grains = await core.segment(id, params.segment, prev, { margin_um: app.settings.outline_margin_um, smooth: app.settings.outline_smooth });
      if (!checkpointed) { app.checkpoint(); checkpointed = true; }
      app.grains = { ...app.grains, [id]: grains };
      app.spots = relinkSpots(app.spots, id, grains);
      count += grains.filter((g) => !g.artifact).length;
      if (!grains.length) empty.push(id);
    }
    return { count, empty };
  });
  if (checkpointed) app.persist();
  if (result) app.notice = `${result.count} ${result.count === 1 ? 'grain' : 'grains'} in ${todo.length} ${todo.length === 1 ? 'area' : 'areas'}${result.empty.length ? `; ${result.empty.length} ${result.empty.length === 1 ? 'area' : 'areas'} found nothing: ${result.empty.join(', ')}` : ''}`;
}
