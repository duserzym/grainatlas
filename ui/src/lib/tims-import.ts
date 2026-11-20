export interface PickNotesRequest { text: string; lab_id: string; first_line?: number; last_line?: number; filename?: string; started_on?: string }
interface Ref { id: string; label?: string; name?: string }
export interface PickNoteLine {
  number: number; text: string; kind: string; status: 'ok' | 'refused' | 'skipped' | 'ignored'; code: string; message: string;
  parsed: { name?: string; started_on?: string };
  resolved: { sample?: Ref; mount?: Ref; spot?: Ref; grain?: Ref; capsule_label?: string; fraction_id?: string };
}
export interface PickNotesPlan {
  lab_id: string; filename?: string; first_line: number; last_line: number; lines: PickNoteLine[];
  summary: Record<PickNoteLine['status'], number>;
  batches: { set_id: string; started_on: string; action: 'create' | 'reuse'; id: string | null; explanation?: string }[];
}
export interface PickNotesReply { plan: PickNotesPlan; created: { batches: string[]; steps: string[]; fractions: string[]; sources: string[] } }
export class PickNotesRefused extends Error {
  plan: PickNotesPlan;
  constructor(plan: PickNotesPlan, detail: string) { super(detail); this.plan = plan; }
}
export function pickNotesRequest(input: { text: string; lab: string; first?: number; last?: number; filename?: string; started?: string }): PickNotesRequest {
  if (!input.text.trim()) throw new Error('Paste notes or choose a file');
  if (!input.lab) throw new Error('Choose a lab');
  const lines = input.text.split(/\r?\n/).length;
  if (lines > 5000 || input.text.length > 1000000) throw new Error('Use at most 5000 lines and 1000000 characters');
  for (const n of [input.first, input.last]) if (n !== undefined && (!Number.isInteger(n) || n < 1 || n > lines)) throw new Error('Line numbers must be within the notes');
  if ((input.first ?? 1) > (input.last ?? lines)) throw new Error('First line must be before last line');
  return { text: input.text, lab_id: input.lab, ...(input.first !== undefined ? { first_line: input.first } : {}), ...(input.last !== undefined ? { last_line: input.last } : {}), ...(input.filename ? { filename: input.filename } : {}), ...(input.started ? { started_on: input.started } : {}) };
}
const label = (ref?: Ref) => ref?.label ?? ref?.name ?? '';
export function pickNotesRows(plan: PickNotesPlan, mount: string | null, ignored = false) {
  return plan.lines.filter(line => ignored || line.status !== 'ignored').map(line => ({
    number: line.number, text: line.text, status: line.status,
    result: line.status === 'refused' ? `refused: ${line.message}` : line.status === 'skipped' ? 'skipped: already imported' : line.status,
    sample: label(line.resolved.sample), mount: label(line.resolved.mount), spot: label(line.resolved.spot), grain: label(line.resolved.grain),
    capsule: line.resolved.capsule_label ?? '', fraction: line.parsed.name ?? '',
    openMount: !!mount && line.resolved.mount?.id === mount,
  }));
}
export function importable(plan: PickNotesPlan) {
  const selected = plan.lines.filter(l => l.number >= plan.first_line && l.number <= plan.last_line);
  return selected.some(l => l.status === 'ok' && l.kind === 'pick') && !selected.some(l => l.status === 'refused');
}
/** One key for the exact request, retained across retries and repeated previews. */
export class PickNotesIdentity {
  private signature = '';
  key = '';
  forRequest(body: PickNotesRequest) {
    const signature = JSON.stringify(body);
    if (signature !== this.signature) { this.signature = signature; this.key = crypto.randomUUID(); }
    return { key: this.key, keyChanged: (key: string) => { this.key = key; } };
  }
}
