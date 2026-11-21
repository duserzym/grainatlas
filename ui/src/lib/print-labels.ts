import type { Project, SpotLabelOptions, SpotLabelReport, SheetPlan, Spot } from './core/types.ts';

export const labelKey = (label: { spot_id: string; occurrence: string }) => `${label.spot_id}/${label.occurrence}`;
export function readLabelOptions(ui: unknown): SpotLabelOptions {
  const record = ui && typeof ui === 'object' ? ui as Record<string, unknown> : {};
  const pins: Record<string, [number, number]> = {};
  if (record.print_label_pins && typeof record.print_label_pins === 'object') {
    for (const [key, value] of Object.entries(record.print_label_pins)) {
      if (Array.isArray(value) && value.length === 2 && value.every(v => typeof v === 'number' && Number.isFinite(v))) pins[key] = [value[0], value[1]];
    }
  }
  const size = record.print_label_size_mm;
  return { size_mm: typeof size === 'number' && Number.isFinite(size) ? Math.max(1.6, Math.min(4, size)) : 'auto', pins };
}
export function labelUI(options: SpotLabelOptions) {
  return { print_label_pins: Object.keys(options.pins).length ? options.pins : undefined, print_label_size_mm: options.size_mm };
}
/** Publishing images must not share this workstation's print presentation. */
export function withoutPrintLabelUI(project: Project): Project {
  if (!project.grainatlas_ui || typeof project.grainatlas_ui !== 'object') return project;
  const { print_label_pins: _pins, print_label_size_mm: _size, ...ui } = project.grainatlas_ui as Record<string, unknown>;
  return { ...project, grainatlas_ui: ui };
}
export function pinLabel(options: SpotLabelOptions, label: SpotLabelReport['labels'][number], origin: [number, number] | null): SpotLabelOptions {
  const pins = { ...options.pins };
  if (origin) pins[labelKey(label)] = [origin[0] - label.anchor_mm[0], origin[1] - label.anchor_mm[1]];
  else delete pins[labelKey(label)];
  return { ...options, pins };
}
/** The display snapshot belongs to the render plan, never its cutting parameters. */
export function spotDisplay(plan: SheetPlan, show: boolean, spots: Spot[], options: SpotLabelOptions): SheetPlan {
  const { print_spots: _spots, print_labels: _labels, ...base } = plan;
  return show ? { ...base, print_spots: spots, print_labels: options } : base;
}
export const isSheetsStyle = (style: string) => style === 'sheets' || style === 'spots';

/** Match an existing cut only when every non-display input still matches. */
export function sheetGeometryControls<T extends { style: string; spotLabels?: unknown }>(controls: T) {
  return { ...controls, style: 'sheets' as const, spotLabels: undefined };
}

/** A combobox accepts Auto or any fixed paper size in the control's range. */
export function parseLabelSize(value: string): SpotLabelOptions['size_mm'] | null {
  if (value.trim().toLowerCase() === 'auto') return 'auto';
  const size = Number(value);
  return value.trim() && Number.isFinite(size) && size >= 1.6 && size <= 4 ? size : null;
}
