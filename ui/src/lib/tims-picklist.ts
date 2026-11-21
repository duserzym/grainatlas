import { spotDiameter } from './spot-size';
import type { Core } from './core/core';
import type { Grain, LayoutResult, MosaicJson, Project, SheetRect, Spot, ToneParams } from './core/types';
import type { BatchDetail, CapsuleOut, FractionOut, SourceOut } from './tims';
import type { OutboxEntry } from './tims-outbox';
import { PickListCrops } from './tims-crop';
import { applyAffine, areaImageEdits, imageTransform, pointBounds, maskAppearance } from './image-edits';

export interface PickListFilter { set: string; batch: string }
export interface PickListGrain { id: string; label: string; area: string; areaName: string; grain?: Grain; spots: Spot[]; sources: SourceOut[] }
export interface PickListRow {
  id: string; capsule: string; capsuleNumber: number | null; fraction: string; sample: string; set: string; setName: string; batch: string; started: string;
  grains: PickListGrain[]; fragment: string; notes: string; pending: boolean;
}
export interface PickListOptions extends PickListFilter { title: string; paper: 'tabloid' | 'letter' | 'a3' | 'a4'; cropMargin: number; cropSize: number }
export function buildPickListRows(input: {
  fractions: FractionOut[]; sets: CapsuleOut[]; batches: BatchDetail[]; outbox: OutboxEntry[];
  grains: Record<string, Grain[]>; spots: Spot[]; areas: { id: string; name: string }[]; filter: PickListFilter;
}): PickListRow[] {
  const grainMap = new Map(Object.entries(input.grains).flatMap(([area, grains]) => grains.map(grain => [grain.id, { area, grain }] as const)));
  const spots = new Map<string, Spot[]>();
  for (const spot of input.spots) if (spot.grain_id) spots.set(spot.grain_id, [...(spots.get(spot.grain_id) ?? []), spot]);
  const batches = new Map(input.batches.map(b => [b.id, b])), sets = new Map(input.sets.map(s => [s.id, s]));
  const order = new Map(input.sets.map((s, i) => [s.id, i]));
  return input.fractions.filter(f => !f.retired_at).flatMap(f => {
    const batch = batches.get(f.batch_id ?? ''), set = sets.get(batch?.capsule_set_id ?? '');
    if (input.filter.set && set?.id !== input.filter.set || input.filter.batch && f.batch_id !== input.filter.batch) return [];
    const sources = f.sources.filter(s => !s.retired_at);
    // The mount list can include sample-only fractions. Keep those at the end.
    const grouped = new Map<string, SourceOut[]>();
    for (const source of sources) grouped.set(source.atlas_grain_id, [...(grouped.get(source.atlas_grain_id) ?? []), source]);
    const grains: PickListGrain[] = [...grouped].map(([id, sources]) => {
      const local = grainMap.get(id), area = local?.area ?? '';
      return { id, grain: local?.grain, label: local?.grain.label || (local ? `Grain ${local.grain.number}` : id), area,
        areaName: input.areas.find(a => a.id === area)?.name ?? area, spots: spots.get(id) ?? [], sources };
    });
    return [{ id: f.id, capsule: (f.capsule_number ? set?.capsule_labels[f.capsule_number - 1] ?? (set ? `${set.code}${f.capsule_number}` : f.capsule_label) : null) ?? '',
      capsuleNumber: f.capsule_number, fraction: f.name, sample: f.sample.name, set: set?.id ?? '', setName: set?.name ?? f.capsule_set_name ?? '', batch: f.batch_id ?? '', started: batch?.started_on ?? '',
      grains, fragment: [...new Set(sources.filter(s => s.is_fragment).map(s => s.fragment_note || 'Fragment'))].join('; '), notes: f.notes,
      pending: input.outbox.some(e => e.target === f.id || (!!f.batch_id && e.target === f.batch_id)),
    }];
  }).sort((a, b) => Number(!a.batch || !a.grains.length) - Number(!b.batch || !b.grains.length)
    || (order.get(a.set) ?? Number.MAX_SAFE_INTEGER) - (order.get(b.set) ?? Number.MAX_SAFE_INTEGER)
    || (a.capsuleNumber ?? Number.MAX_SAFE_INTEGER) - (b.capsuleNumber ?? Number.MAX_SAFE_INTEGER) || a.batch.localeCompare(b.batch) || a.fraction.localeCompare(b.fraction, undefined, { numeric: true }));
}
export const escapeSvg = (text: string) => text.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' }[c]!));
export function pickListPaper(paper: PickListOptions['paper']): [number, number] {
  return { tabloid: [279.4, 431.8], letter: [215.9, 279.4], a3: [297, 420], a4: [210, 297] }[paper] as [number, number];
}
const bodySize = 3.2, headingSize = 2.8;
const cellSizes = [7, 4, bodySize, bodySize, bodySize, bodySize, bodySize];
const lineHeight = (size: number) => size * 1.3;
function lines(text: string, width: number, size: number): string[] {
  const max = Math.max(1, Math.floor(width / (size * .55))), result: string[] = [];
  for (const paragraph of text.split('\n')) {
    let rest = paragraph;
    while (rest.length > max) {
      const space = rest.lastIndexOf(' ', max), end = space > max / 2 ? space : max;
      result.push(rest.slice(0, end)); rest = rest.slice(end).trimStart();
    }
    result.push(rest);
  }
  return result;
}
function svgText(text: string[], x: number, y: number, size = bodySize, bold = false) {
  return `<text x="${x}" y="${y}" font-size="${size}"${bold ? ' font-weight="bold"' : ''}>${text.map((line, i) => `<tspan x="${x}" dy="${i ? lineHeight(size) : 0}">${escapeSvg(line)}</tspan>`).join('')}</text>`;
}
/** One local crop per grain using the canvas pixels, LUT and image edits. */
export async function renderPickList(input: {
  core: Pick<Core, 'pyramidTile' | 'toneLut'>;
  cropImage?: PickListCrops['image']; project: Project; rows: PickListRow[]; options: PickListOptions;
  mosaics: Record<string, MosaicJson>; tone: Record<string, ToneParams>; mount: string;
}): Promise<LayoutResult> {
  const { options: o, rows } = input;
  if (!Number.isFinite(o.cropMargin) || o.cropMargin < 0 || !Number.isFinite(o.cropSize) || o.cropSize < 15 || o.cropSize > 60) throw new Error('Use a crop margin of at least 0 µm and a paper crop size from 15 to 60 mm');
  if (!rows.length) throw new Error('No plucks match these filters');
  const started = performance.now(); let cropCalls = 0;
  const [w, h] = pickListPaper(o.paper), margin = 8, imageWidth = o.cropSize, textWidth = w - margin * 2 - imageWidth - 8;
  const proportions = [.14, .11, .14, .19, .11, .14, .17];
  const widths = proportions.map(p => p * textWidth);
  const headings = ['Capsule', 'Fraction', 'Sample', 'Grain / LA spots', 'Area', 'Fragment note', 'Notes'].map((heading, i) => lines(heading, widths[i] - 2, headingSize));
  const titleY = margin + 5, headerY = titleY + lineHeight(5) + 1, headingY = headerY + lineHeight(bodySize) + 3;
  const tableTop = headingY + Math.max(...headings.map(cell => cell.length)) * lineHeight(headingSize) + 2;
  const pageRows: { row: PickListRow; cells: string[][]; height: number }[][] = [];
  let page: typeof pageRows[number] = [], used = 0;
  const capacity = h - margin - tableTop;
  for (const row of rows) {
    const values = [row.capsule || '—', row.fraction, row.sample, row.grains.map(g => `${g.label}${g.spots.length ? ` · ${g.spots.map(s => s.name).join(', ')}` : ''}`).join('\n') || 'Sample only', row.grains.map(g => g.areaName).filter(Boolean).join(', '), row.fragment, [row.notes, row.pending ? 'not yet in the database' : ''].filter(Boolean).join('\n')];
    const cells = values.map((v, i) => lines(v, widths[i] - 2, cellSizes[i]));
    const height = Math.max(o.cropSize * Math.max(1, row.grains.length) + 4, ...cells.map((cell, i) => cell.length * lineHeight(cellSizes[i]) + 4));
    if (height > capacity) throw new Error(`Fraction ${row.fraction} needs more room. Use larger paper or a smaller crop size.`);
    if (page.length && (used + height > capacity || row.batch !== page[0].row.batch || row.set !== page[0].row.set)) { pageRows.push(page); page = []; used = 0; }
    page.push({ row, cells, height }); used += height;
  }
  if (page.length) pageRows.push(page);
  const crops = new Map<string, Promise<string>>();
  const renderer = new PickListCrops(input.core);
  const cropImage = input.cropImage ?? renderer.image.bind(renderer);
  async function crop(g: PickListGrain): Promise<string> {
    const mosaic = input.mosaics[g.area];
    if (!g.grain || !mosaic) return svgText(['Image unavailable'], 1, 5);
    const key = `${g.area}/${g.id}`;
    if (crops.has(key)) return crops.get(key)!;
    const work = (async () => {
      const e = areaImageEdits(input.project, g.area), size: [number, number] = [mosaic.w * mosaic.um_per_px, mosaic.h * mosaic.um_per_px];
      const transform = imageTransform(e, size), bounds = pointBounds([[0, 0], [size[0], 0], [0, size[1]], size].map(p => applyAffine(transform, p as [number, number])));
      transform[4] -= bounds[0]; transform[5] -= bounds[1];
      const polygon = g.grain!.polygon_um.map(p => applyAffine(transform, p));
      const box = pointBounds(polygon), width = bounds[2] - bounds[0], height = bounds[3] - bounds[1];
      const side = Math.max(box[2] - box[0], box[3] - box[1]) + o.cropMargin * 2;
      const cx = (box[0] + box[2]) / 2, cy = (box[1] + box[3]) / 2;
      const rect: SheetRect = [Math.max(0, cx - side / 2), Math.max(0, cy - side / 2), Math.min(width, cx + side / 2), Math.min(height, cy + side / 2)];
      if (rect[2] <= rect[0] || rect[3] <= rect[1]) return svgText(['Outside image'], 1, 5);
      const tone = input.tone[g.area] ?? input.project.areas.find(a => a.id === g.area)?.params.tone;
      if (!tone) throw new Error(`Displayed tone is unavailable for ${g.areaName}`);
      const png = await cropImage(g.area, rect, mosaic, tone, e, transform, o.cropSize); cropCalls++;
      const rw = rect[2] - rect[0], rh = rect[3] - rect[1], stroke = Math.max(rw, rh) / o.cropSize * .22;
      const spotShapes = g.spots.map(s => {
        const [x, y] = applyAffine(transform, [s.x_um, s.y_um]), r = spotDiameter(s);
        if (r === null) return `<circle cx="${x}" cy="${y}" r="${5 * Math.max(rw, rh) / o.cropSize}" stroke-dasharray="${3 * stroke} ${3 * stroke}"><title>Size not set</title></circle>`;
        if (s.kind !== 'spot' && s.x2_um !== null && s.y2_um !== null) {
          const [x2, y2] = applyAffine(transform, [s.x2_um, s.y2_um]);
          return s.kind === 'line' ? `<path d="M${x},${y} L${x2},${y2}"/>` : `<polygon points="${[[s.x_um, s.y_um], [s.x2_um, s.y_um], [s.x2_um, s.y2_um], [s.x_um, s.y2_um]].map(p => applyAffine(transform, p as [number, number]).join(',')).join(' ')}"/>`;
        }
        return `<circle cx="${x}" cy="${y}" r="${r / 2}"/>`;
      }).join('');
      return `<svg width="${o.cropSize}" height="${o.cropSize}" viewBox="${rect[0]} ${rect[1]} ${rw} ${rh}"><image x="${rect[0]}" y="${rect[1]}" width="${rw}" height="${rh}" href="${png}"/>${imageOverlay(e, size, transform, `crop-${encodeURIComponent(g.area)}-${encodeURIComponent(g.id)}`)}<polygon points="${polygon.map(p => p.join(',')).join(' ')}" fill="none" stroke="#f8cf54" stroke-width="${stroke}"/><g fill="none" stroke="#68daff" stroke-width="${stroke}">${spotShapes}</g></svg>`;
    })();
    crops.set(key, work); return work;
  }
  const pages: string[] = [];
  try {
  for (const [index, entries] of pageRows.entries()) {
    const first = entries[0].row;
    let content = svgText([o.title], margin, titleY, 5) + svgText([`${input.mount} · ${first.setName || 'No capsule set'} · ${first.started || 'No batch'}`], margin, headerY);
    content += svgText([`${index + 1} / ${pageRows.length}`], w - margin - 17, titleY);
    let x = margin;
    headings.forEach((heading, i) => { content += svgText(heading, x + 1, headingY, headingSize); x += widths[i]; });
    content += svgText(['Grain image'], x + 1, headingY, headingSize);
    let y = tableTop;
    for (const { row, cells, height } of entries) {
      x = margin;
      content += `<g data-pick-row="${escapeSvg(row.id)}"><title>${escapeSvg(`${row.capsule} · ${row.fraction}`)}</title><path d="M${margin},${y}H${w - margin}" stroke="#aaa" stroke-width=".2"/>`;
      cells.forEach((cell, i) => { content += svgText(cell, x + 1, y + 2 + cellSizes[i], cellSizes[i], i < 2); x += widths[i]; });
      for (const [i, grain] of row.grains.entries()) content += `<g transform="translate(${x + 1},${y + 2 + i * o.cropSize})">${await crop(grain)}</g>`;
      content += `<rect x="${w - margin - 4}" y="${y + 3}" width="3.5" height="3.5" fill="none" stroke="#222" stroke-width=".3"/></g>`;
      y += height;
    }
    pages.push(`<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${w}mm" height="${h}mm" viewBox="0 0 ${w} ${h}" font-family="sans-serif" fill="#111"><rect width="${w}" height="${h}" fill="white"/>${content}</svg>`);
  }
  } finally { renderer.dispose(); }
  console.debug('TIMS pick list', { rows: rows.length, cropCalls, requests: 0, ms: Math.round(performance.now() - started) });
  return { pages, paper_mm: [w, h], items: rows.length, um_per_mm: 0 };
}

/** Mirrors ImageEditOverlay: masks and outside-crop shading remain visible in print. */
function imageOverlay(e: import('./core/types').AreaImageEdits, [w, h]: [number, number], transform: import('./image-edits').Affine, id: string) {
  const { background, ink, edgeUm, pitchUm: pitch, lineUm } = maskAppearance;
  const strokes = e.masks.filter(m => m.points_um.length && m.radius_um > 0);
  const path = (points: [number, number][]) => `M${points.map(p => p.join(' ')).join('L')}${points.length === 1 ? 'l0.001 0' : ''}`;
  let overlay = '';
  if (strokes.length) {
    const stroke = (inset: number, colour: string) => strokes.filter(m => m.radius_um > inset).map(m => `<path d="${path(m.points_um)}" fill="none" stroke="${colour}" stroke-width="${2 * (m.radius_um - inset)}" stroke-linecap="round" stroke-linejoin="round"/>`).join('');
    overlay += `<defs><clipPath id="${id}-bounds"><rect width="${w}" height="${h}"/></clipPath><pattern id="${id}-hatch" patternUnits="userSpaceOnUse" width="${pitch}" height="${pitch}"><path d="M${-pitch} ${pitch}L${pitch} ${-pitch}M0 ${pitch}L${pitch} 0M0 ${2 * pitch}L${2 * pitch} 0" fill="none" stroke="rgb(${ink},${ink},${ink})" stroke-width="${lineUm / Math.SQRT2}"/></pattern><mask id="${id}-edge" maskUnits="userSpaceOnUse" x="0" y="0" width="${w}" height="${h}" style="mask-type:luminance">${stroke(0, 'white')}${stroke(edgeUm, 'black')}</mask></defs><g clip-path="url(#${id}-bounds)">${stroke(0, `rgb(${background},${background},${background})`)}<rect width="${w}" height="${h}" fill="url(#${id}-hatch)" mask="url(#${id}-edge)"/></g>`;
  }
  if (e.crop) {
    const b = pointBounds(e.crop.points_um), points: [number, number][] = e.crop.kind === 'rect' ? [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]] : e.crop.points_um;
    overlay += `<path d="M0 0H${w}V${h}H0Z${path(points)}Z" fill="black" fill-rule="evenodd" opacity=".6"/>`;
  }
  return `<g transform="matrix(${transform.join(' ')})">${overlay}</g>`;
}
