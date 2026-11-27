import type { SyncBody } from './sync-api.ts';
import { storedElement, type Kind } from './sync.ts';

/** UUIDs stay stable: ids in the 200 reply map them to database server_id values. */
export function syncWireBody(body: SyncBody) {
  const narrow = <K extends Kind>(kind: K) => body[kind].upsert.map(e => {
    const row = { ...storedElement(kind, e) } as unknown as Record<string, unknown>;
    // Sample names are hints resolved by the database, sent only in snapshots.
    if (body.mode !== 'snapshot') delete row.sample;
    else if ('sample' in e) row.sample = e.sample;
    if (kind === 'grains') {
      // The database keeps an omitted artifact; '' is how it stores "no artifact", so send it to clear a mark.
      row.artifact ??= '';
      if (!row.split_from) delete row.split_from;
      if (!(row.merged_from as string[]).length) delete row.merged_from;
    }
    if (kind === 'spots') {
      if ('comment' in row) row.comment ??= '';
      if ('kind' in row) row.kind ||= 'spot';
    }
    if (kind === 'spots' && (('detach_grain' in e && e.detach_grain === true) || ('grain_id' in e && e.grain_id === null))) { row.detach_grain = true; delete row.grain_id; }
    const keep = kind === 'spots' ? ['x_um', 'y_um', 'x2_um', 'y2_um', 'diameter_um'] : kind === 'grains' ? ['polygon_um', 'centroid_um', 'area_um2', 'number'] : [];
    for (const key of keep) if (row[key] == null) delete row[key];
    return row;
  });
  return {
    project_uuid: body.project_uuid, base_revision: body.base_revision, mode: body.mode, partial: true,
    ...(body.dry_run ? { dry_run: true } : {}), ...(body.confirm_bulk ? { confirm_bulk: true } : {}),
    spots: { upsert: narrow('spots'), delete: body.spots.delete },
    grains: { upsert: narrow('grains'), delete: [...new Set([...body.grains.delete, ...body.grains.retire])] },
    labels: { upsert: narrow('labels'), delete: body.labels.delete },
  };
}
