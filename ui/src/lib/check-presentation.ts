import type { Finding } from './core/types.ts';

export const CHECK_TITLES: Record<string, string> = {
  never_segmented: 'Areas without grain finding',
  off_grain: 'Spots not on a grain',
  grain_mismatch: 'Spot on a different grain than recorded',
  ring_outside: 'Ring extends past the grain',
  duplicate_name: 'Duplicate names',
  duplicate_n: 'Duplicate sample and n',
  n_gap: 'Gaps in n',
  n_reused: 'n used in several samples',
  overlap: 'Overlapping spots',
  sample_mismatch: "Sample differs from its grain's or area's",
  pattern_mismatch: 'Names off the pattern',
  no_label: 'Spots without an acquisition label',
  no_spot: 'Labels without a spot',
  near_miss: 'Suggested renames',
};

export function findingSignature(findings: Finding[]): string {
  return JSON.stringify(findings.map((f) => [f.code, [...f.spot_ids].sort()]).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b))));
}

/** App-session memory only; project UUID also covers reopening a project in another tab. */
export const acceptedChecks = new Map<string, string>();

export function findingsSummary(findings: Finding[]): string {
  const groups = new Map<string, Finding[]>();
  for (const f of findings) groups.set(f.code, [...(groups.get(f.code) ?? []), f]);
  return [...groups].map(([code, rows]) => {
    const spots = new Set(rows.flatMap((f) => f.spot_ids)).size;
    const areas = new Set(rows.map((f) => f.area_id)).size;
    const areaPhrase = (text: string) => `${areas} ${areas === 1 ? 'area' : 'areas'} ${text}`;
    const spotPhrase = (one: string, many = one) => `${spots} ${spots === 1 ? 'spot' : 'spots'} ${spots === 1 ? one : many}`;
    switch (code) {
      case 'never_segmented': return rows.map((f) => f.message).join('; ');
      case 'pattern_mismatch': return areaPhrase('with names off the pattern');
      case 'off_grain': return spotPhrase('off a grain');
      case 'grain_mismatch': return spotPhrase('on a different grain than recorded');
      case 'ring_outside': return spotPhrase('overhanging its grain', 'overhanging their grains');
      case 'duplicate_name': return spotPhrase('with a duplicate name');
      case 'duplicate_n': return spotPhrase('with a duplicate sample and n');
      case 'n_gap': return `${rows.length} ${rows.length === 1 ? 'sample' : 'samples'} with gaps in n`;
      case 'n_reused': return spotPhrase('sharing n across samples');
      case 'overlap': return areaPhrase('with overlapping spots');
      case 'sample_mismatch': return spotPhrase("with a sample different from its grain's or area's", "with samples different from their grains' or areas'");
      case 'no_label': return areaPhrase('with spots without acquisition labels');
      case 'no_spot': return `${rows.length} ${rows.length === 1 ? 'group' : 'groups'} of labels without a spot`;
      case 'near_miss': return `${rows.length} suggested ${rows.length === 1 ? 'rename' : 'renames'}`;
      default: return `${rows.length} ${CHECK_TITLES[code]?.toLowerCase() ?? 'other warnings'}`;
    }
  }).join(', ');
}
