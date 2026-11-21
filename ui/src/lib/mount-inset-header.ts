import type { AreaRecord } from './core/types';
import { label } from './mount-inset.ts';

/** The hosted lookup fills status even in read-only presentation; never fetch for a label. */
export function mountName(statusLabel?: string | null, atlasName?: string | null, projectName?: string | null): string {
  return [statusLabel, atlasName, projectName].find(s => s?.trim())?.trim() ?? 'Mount';
}

export function sampleNames(groups: { name: string }[], areas: Pick<AreaRecord, 'id' | 'sample' | 'name'>[]): string[] {
  return [...new Set(groups.length ? groups.map(g => g.name) : areas.map(label))].filter(s => s.trim());
}

/** Show a complete caption in the space left by the mount name, or leave it empty. */
export function sampleText(names: string[], fallback: string, available: number,
  measure: (text: string) => number = text => Array.from(text).length * 6): { text: string; title: string } {
  const title = names.join(', ') || fallback;
  const count = names.length ? `${names.length} ${names.length === 1 ? 'sample' : 'samples'}` : '';
  return { title, text: measure(title) <= available ? title : count && measure(count) <= available ? count : '' };
}

export const shortMountName = (name: string) => Array.from(name).length <= 22 ? name : `${Array.from(name).slice(0, 21).join('')}…`;
