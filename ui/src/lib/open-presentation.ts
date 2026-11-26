/** Compact last-change display only; sync-state wording uses its existing helper. */
export function lastChangeTime(at: string | null | undefined, now = Date.now()): string {
  if (!at) return 'No changes yet';
  const time = Date.parse(at);
  if (!Number.isFinite(time)) return 'unknown time';
  const age = Math.max(0, now - time);
  for (const [unit, duration] of [['y', 365 * 86_400_000], ['mo', 30 * 86_400_000], ['d', 86_400_000], ['h', 3_600_000], ['min', 60_000]] as const) {
    if (age >= duration) return `${Math.floor(age / duration)} ${unit} ago`;
  }
  return 'just now';
}

export const mountColumns = [
  { label: 'Name', sort: 'label' },
  { label: 'Samples', sort: null },
  { label: 'Grains', sort: null },
  { label: 'Spots', sort: null },
  { label: 'Last change', sort: 'last_changed' },
  { label: 'Copy / sync', sort: null },
] as const;

/** Use the server's existing sort keys and direction control. No local reordering. */
export function sortFromHeader(current: string, key: string): string {
  if (!mountColumns.some(column => column.sort === key)) return current;
  return current.replace(/^-/, '') === key ? current.startsWith('-') ? key : `-${key}` : key === 'last_changed' ? `-${key}` : key;
}

export function headerSort(current: string, key: string | null): 'ascending' | 'descending' | 'none' | undefined {
  if (!key) return undefined;
  return current.replace(/^-/, '') === key ? current.startsWith('-') ? 'descending' : 'ascending' : 'none';
}
