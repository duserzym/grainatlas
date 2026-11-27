import type { AreaMeta, AreaParams, AreaRecord } from './core/types';

export type StitchSource = Pick<AreaRecord, 'tiles' | 'pixel_size_nm' | 'stage_sign'>;

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v) => v && typeof v === 'object' && !Array.isArray(v)
    ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b))) : v);
}

/** Missing legacy fields mean the applied defaults; JSON key order is irrelevant. */
export function stitchSettingsChanged(applied: AreaParams['stitch'], pending: AreaParams['stitch']): boolean {
  return canonical(applied) !== canonical({ ...applied, ...pending });
}

/** Hashes are checked after the existing tile read; no extra reads or registration. */
export function stitchSourceChanged(saved: StitchSource, rendered: StitchSource, meta?: AreaMeta): boolean {
  if (canonical(saved.stage_sign) !== canonical(meta?.stage_sign ?? rendered.stage_sign)
    || saved.pixel_size_nm !== (meta?.pixel_size_nm ?? rendered.pixel_size_nm)
    || saved.tiles.length !== rendered.tiles.length || (meta && saved.tiles.length !== meta.tiles.length)) return true;
  return saved.tiles.some((t, i) => {
    const current = rendered.tiles[i], m = meta?.tiles[i] ?? current;
    return t.file !== m.file || (t.sha256 && current.sha256 && t.sha256 !== current.sha256)
      || (t.w > 0 && m.w > 0 && t.w !== m.w) || (t.h > 0 && m.h > 0 && t.h !== m.h)
      || canonical(t.pos_um) !== canonical(m.pos_um) || t.pixel_size_nm !== m.pixel_size_nm;
  });
}
