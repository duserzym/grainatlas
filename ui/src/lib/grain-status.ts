import type { ArtifactReason, Grain } from './core/types.ts';

export interface GrainStatus {
  kind: 'never' | 'none' | 'mostly-flagged' | 'fine';
  message: string;
  warning: boolean;
}

/** Classify saved detections, keeping an absent entry distinct from an empty run. */
export function grainStatus(grains: Grain[] | undefined, coreWarning?: string | null): GrainStatus {
  if (grains === undefined) return { kind: 'never', message: 'Grain finding has not been run on this area.', warning: false };
  if (!grains.length) {
    // The core includes the actual mask threshold, even when it was automatic.
    const threshold = coreWarning?.indexOf('threshold ');
    const hint = threshold !== undefined && threshold >= 0
      ? coreWarning!.slice(threshold).replace(/^threshold/, 'Threshold')
      : 'Check the polarity, signal profile and threshold, and that the area shows grains.';
    return { kind: 'none', message: `Find grains found nothing here. ${hint}`, warning: true };
  }
  const counts = new Map<ArtifactReason, number>();
  for (const g of grains) if (g.artifact) counts.set(g.artifact, (counts.get(g.artifact) ?? 0) + 1);
  const flagged = [...counts.values()].reduce((a, b) => a + b, 0);
  if (flagged * 5 < grains.length * 4) return { kind: 'fine', message: '', warning: false };
  // Match the core's rule order, including its last-in-order tie break.
  const order: ArtifactReason[] = ['bubble', 'halo', 'sliver', 'block', 'speck', 'manual'];
  const mostly = order.reduce((best, rule) => (counts.get(rule) ?? 0) >= (counts.get(best) ?? 0) ? rule : best, order[0]);
  const hint = coreWarning?.split(': ').slice(1).join(': ')
    // The shared warning suggests a CLI flag. Point desktop users to the Signal control.
    .replace(/--signal (cl|bse)/g, (_, signal: string) => `${signal.toUpperCase()} in Signal`)
    .replace(/^the /, 'The ')
    || 'Check the signal profile in Segmentation.';
  return { kind: 'mostly-flagged', message: `${flagged} of ${grains.length} detections are flagged as not grains, mostly ${mostly}. ${hint}`, warning: true };
}

/** The core's resolved binding falls back to CL; metadata distinguishes the fallback label. */
export function autoSignalLabel(signal: 'cl' | 'bse', meta: Record<string, string>): string {
  if (signal === 'bse') return 'Auto (BSE, from the image metadata)';
  const history = meta['zag.ABCCHistory.SignalType1']?.trim().toUpperCase();
  return history === 'UVD' || history?.includes('CL')
    ? 'Auto (CL, from the image metadata)'
    : 'Auto (not recorded: CL rules)';
}
