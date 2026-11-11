import type { SheetParams, WholeParams } from './core/types.ts';

/** Shared paper and header fields, with the whole-area controls kept independent of grain scale. */
export function wholePrintOptions(sheet: SheetParams, options: {
  areaId: string; orientation: WholeParams['orientation']; scale: number | 'auto'; grid: boolean;
}): WholeParams {
  return {
    paper: sheet.paper, margin_mm: sheet.margin_mm, title: sheet.title, lab: sheet.lab,
    registration_file: sheet.registration_file, project: sheet.project, printed: sheet.printed,
    area_id: options.areaId, orientation: options.orientation, grid: options.grid,
    um_per_mm: typeof options.scale === 'number' && options.scale > 0 ? options.scale : 'auto',
    dpi: 600, name: '',
  };
}
