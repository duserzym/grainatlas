import { BackgroundSync } from './background-sync';
import { SyncApiError } from './sync-api';
import { requireWrite } from './presentation';
import { ServerReadOnlyError } from './oversight';
import type { MirrorStorage } from './mount-mirror';
import { localSuggestion, nextFreeCapsule, type BatchDetail, type BatchInput, type CapsuleOut, type FractionFields, type FractionInput, type FractionOut, type Mineral, type SampleRef, type SourceInput, type SourceOut, type TimsPages, type TimsTransport } from './tims';

export type Operation = 'batch.create' | 'fraction.create' | 'fraction.patch' | 'fraction.retire' | 'source.add' | 'source.retire';
export interface OutboxEntry {
  id: string; key: string; op: Operation; target: string; source?: string;
  body: Record<string, unknown>; sample?: SampleRef; sourceIds?: string[]; mineral?: Mineral; state: 'pending' | 'waiting' | 'failed';
  error?: string; attempted?: boolean;
}
export interface TimsData {
  version: 1; fractions: FractionOut[]; sets: CapsuleOut[]; batches: BatchDetail[];
  pages: TimsPages; batchFractions: Record<string, FractionOut[]>; details: Record<string, { value: BatchDetail; etag: string | null }>;
  outbox: OutboxEntry[]; ids: Record<string, string>; samples: Record<string, SampleRef>; lastSet: string; lastBatch: Record<string, string>;
}
const empty = (): TimsData => ({ version: 1, fractions: [], sets: [], batches: [], pages: {}, batchFractions: {}, details: {}, outbox: [], ids: {}, samples: {}, lastSet: '', lastBatch: {} });
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const local = () => `local:${crypto.randomUUID()}`;
const refs = (entry: OutboxEntry, id: string) => entry.target === id || entry.source === id || Object.values(entry.body).includes(id);
export interface TimsBadge { text: string; state: 'sent' | 'pending' | 'waiting' | 'failed'; fractions: FractionOut[] }
/** The durable base and the outbox never enter a Project, grain, spot or sync hash.
 * Optimism is a projection: refusing a write removes its effect without an undo patch. */
export class TimsOutbox {
  data = empty();
  error = '';
  offline = false;
  busy = false;
  enabled = true;
  private automatic = true;
  setAutomatic(enabled: boolean) {
    this.automatic = enabled;
    if (!enabled) this.scheduler.cancel();
    else this.wake();
  }
  private refreshing?: Promise<void>;
  private saving: Promise<void> = Promise.resolve();
  private scheduler = new BackgroundSync(() => { if (this.automatic) void this.flush().catch(() => {}); }, Math.random, 250);
  readonly key: string;
  readonly mount: string;
  private storage: MirrorStorage;
  api: TimsTransport;
  private changed: () => void;
  private writable: () => boolean;
  constructor(key: string, mount: string, storage: MirrorStorage, api: TimsTransport, changed: () => void = () => {}, writable: () => boolean = () => true) {
    this.key = key; this.mount = mount; this.storage = storage; this.api = api; this.changed = changed; this.writable = writable;
  }
  async load() {
    const raw = await this.storage.getItem(this.key);
    if (raw) { const data = JSON.parse(raw) as TimsData; if (data.version !== 1) throw new Error('Unsupported TIMS mirror version'); this.data = { ...empty(), ...data }; }
    this.changed();
  }
  dispose() { this.enabled = false; this.scheduler.cancel(); }
  private notify() { this.data = { ...this.data }; this.changed(); }
  private requireEdit() {
    requireWrite();
    if (!this.enabled || !this.writable()) throw new Error('This TIMS view is read-only.');
  }
  private remap(temp: string, real: string) {
    for (const entry of this.data.outbox) {
      if (entry.target === temp) entry.target = real;
      if (entry.source === temp) entry.source = real;
      if (entry.body.batch_id === temp) entry.body.batch_id = real;
    }
    for (const set of Object.keys(this.data.lastBatch)) if (this.data.lastBatch[set] === temp) this.data.lastBatch[set] = real;
    for (const alias of Object.keys(this.data.ids)) if (this.data.ids[alias] === temp) this.data.ids[alias] = real;
    this.data.ids[temp] = real;
  }
  async persist() {
    const encoded = JSON.stringify(this.data);
    const save = this.saving.catch(() => {}).then(() => this.storage.setItem(this.key, encoded));
    this.saving = save;
    try { await save; }
    catch (e) { this.error = 'TIMS changes could not be saved on this computer. Free some space and try again.'; this.notify(); throw e; }
  }
  private async edited() { this.notify(); await this.persist(); this.wake(); }
  resolve(id: string) { return this.data.ids[id] ?? id; }
  private get applicable() {
    const blocked = new Set(this.data.outbox.filter(e => e.state === 'failed').flatMap(e => e.op.endsWith('.create') ? [e.target] : e.op === 'source.add' && e.source ? [e.source] : []));
    return this.data.outbox.filter(e => {
      if (e.state === 'failed' || [...blocked].some(id => refs(e, id))) { if (e.op.endsWith('.create')) blocked.add(e.target); return false; }
      return true;
    });
  }
  private project(entries: OutboxEntry[]) {
    let fractions = clone(this.data.fractions);
    for (const e of entries) {
      if (e.op === 'fraction.create') fractions.push({ ...e.body, id: e.target, sample: e.sample!, status: 'picked', sources: (e.body.sources as SourceInput[]).map((s, i) => ({ ...s, id: e.sourceIds?.[i] ?? `${e.target}:source:${i}` })) } as FractionOut);
      else if (e.op === 'fraction.patch') fractions = fractions.map(f => f.id === e.target ? { ...f, ...e.body } as FractionOut : f);
      else if (e.op === 'fraction.retire') fractions = fractions.map(f => f.id === e.target ? { ...f, retired_at: 'pending' } : f);
      else if (e.op === 'source.add') fractions = fractions.map(f => f.id === e.target ? { ...f, sources: [...f.sources, { ...e.body, id: e.source! } as unknown as SourceOut] } : f);
      else if (e.op === 'source.retire') fractions = fractions.map(f => f.id === e.target ? { ...f, sources: f.sources.filter(s => s.id !== e.source) } : f);
    }
    return fractions;
  }
  get fractions() { return this.project(this.applicable); }
  get batches(): BatchDetail[] {
    return [...this.data.batches, ...this.applicable.filter(e => e.op === 'batch.create').map(e => ({ ...e.body, id: e.target, operator_name: 'You', capsule_map: {} }) as unknown as BatchDetail)];
  }
  occupancy(batch: string, exclude?: string) {
    const map = { ...this.batches.find(b => b.id === batch)?.capsule_map };
    // Clear obsolete positions held by cached fractions, then replay their current view.
    const known = new Set(this.data.fractions.map(f => f.id));
    for (const [n, id] of Object.entries(map)) if (id && (known.has(id) || id === exclude)) map[n] = null;
    for (const f of this.fractions) if (!f.retired_at && f.id !== exclude && f.batch_id === batch && f.capsule_number) map[String(f.capsule_number)] = f.id;
    return map;
  }
  nextCapsule(batch: string, exclude?: string) {
    const set = this.data.sets.find(s => s.id === this.batches.find(b => b.id === batch)?.capsule_set_id);
    return nextFreeCapsule(set?.positions ?? 15, this.occupancy(batch, exclude));
  }
  get badges(): Map<string, TimsBadge> {
    const map = new Map<string, TimsBadge>();
    const fractions = this.fractions;
    const projected = this.data.outbox.some(e => e.state === 'failed') ? this.project(this.data.outbox) : [];
    const ids = new Set(fractions.map(f => f.id));
    // Failed additions remain visible as failed badges while their effect is rolled back.
    const failed = this.data.outbox.filter(e => e.state === 'failed');
    const failedView = failed.length ? this.project(failed) : [];
    for (const f of [...fractions, ...projected.filter(f => !ids.has(f.id) && this.data.outbox.some(e => e.op === 'fraction.create' && e.target === f.id)), ...failedView.filter(f => !ids.has(f.id) && !projected.some(p => p.id === f.id))]) {
      if (f.retired_at) continue;
      const entries = this.data.outbox.filter(e => e.target === f.id || (f.batch_id && e.target === f.batch_id));
      const state = entries.some(e => e.state === 'failed') ? 'failed' : entries.some(e => e.state === 'waiting') ? 'waiting' : entries.length ? 'pending' : 'sent';
      const set = this.data.sets.find(s => s.id === this.batches.find(b => b.id === f.batch_id)?.capsule_set_id);
      const label = f.capsule_number ? set?.capsule_labels[f.capsule_number - 1] ?? f.capsule_label : null;
      const failedSources = failed.filter(e => e.op === 'source.add' && e.target === f.id).map(e => ({ ...e.body, id: e.source! }) as unknown as SourceOut);
      for (const source of [...f.sources, ...failedSources]) {
        if (source.retired_at) continue;
        const text = `${label ? `${label} · ` : ''}${f.name}${source.is_fragment ? '†' : ''}`;
        const previous = map.get(source.atlas_grain_id);
        if (previous?.fractions.some(fraction => fraction.id === f.id)) continue;
        map.set(source.atlas_grain_id, { text: previous ? `${previous.text} / ${text}` : text, state: previous?.state === 'failed' ? 'failed' : state, fractions: [...(previous?.fractions ?? []), failedSources.length ? { ...f, sources: [...f.sources, ...failedSources] } : f] });
      }
    }
    return map;
  }
  private entry(op: Operation, target: string, body: object = {}, extra: Partial<OutboxEntry> = {}) {
    const e: OutboxEntry = { id: crypto.randomUUID(), key: crypto.randomUUID(), op, target, body: clone(body) as Record<string, unknown>, state: 'pending', ...extra };
    if (op === 'fraction.create') e.sourceIds = (e.body.sources as SourceInput[]).map(() => local());
    this.data.outbox.push(e); return e;
  }
  async createBatch(body: BatchInput, mineral?: Mineral) { this.requireEdit(); const id = local(); this.entry('batch.create', id, body, { mineral }); await this.edited(); return id; }
  async createFraction(body: FractionInput, sample: SampleRef) {
    this.requireEdit(); const id = local(); this.entry('fraction.create', id, { ...body, name: body.name.toLowerCase() }, { sample }); await this.edited(); return id;
  }
  async patchFraction(id: string, body: Partial<FractionFields>) {
    this.requireEdit(); id = this.resolve(id);
    if (body.name) body = { ...body, name: body.name.toLowerCase() };
    const create = this.data.outbox.find(e => e.op === 'fraction.create' && e.target === id && !e.attempted);
    if (create) Object.assign(create.body, body);
    else {
      const patch = this.data.outbox.find(e => e.op === 'fraction.patch' && e.target === id && !e.attempted && e.state !== 'failed');
      if (patch) Object.assign(patch.body, body); else this.entry('fraction.patch', id, body);
    }
    await this.edited();
  }
  async addSource(id: string, source: SourceInput) {
    this.requireEdit(); id = this.resolve(id);
    const create = this.data.outbox.find(e => e.op === 'fraction.create' && e.target === id && !e.attempted);
    if (create) { (create.body.sources as SourceInput[]).push(clone(source)); (create.sourceIds ??= []).push(local()); }
    else this.entry('source.add', id, source, { source: local() });
    await this.edited();
  }
  async retireSource(id: string, sourceId: string) {
    this.requireEdit(); id = this.resolve(id); sourceId = this.resolve(sourceId);
    const create = this.data.outbox.find(e => e.op === 'fraction.create' && e.target === id && !e.attempted);
    if (create) {
      const index = create.sourceIds ? create.sourceIds.indexOf(sourceId) : Number(sourceId.split(':source:')[1]);
      if (index < 0 || !Number.isInteger(index)) return;
      (create.body.sources as SourceInput[]).splice(index, 1); create.sourceIds?.splice(index, 1);
      if (!(create.body.sources as SourceInput[]).length) this.drop(create.id);
    } else {
      const add = this.data.outbox.find(e => e.op === 'source.add' && e.source === sourceId && !e.attempted);
      if (add) this.drop(add.id); else this.entry('source.retire', id, {}, { source: sourceId });
    }
    await this.edited();
  }
  async retireFraction(id: string) {
    this.requireEdit(); id = this.resolve(id);
    const create = this.data.outbox.find(e => e.op === 'fraction.create' && e.target === id && !e.attempted);
    if (create) this.drop(create.id); else this.entry('fraction.retire', id);
    await this.edited();
  }
  private drop(entryId: string) {
    const entry = this.data.outbox.find(e => e.id === entryId);
    const removed = new Set(entry?.op.endsWith('.create') ? [entry.target] : []);
    this.data.outbox = this.data.outbox.filter(e => {
      if (e.id === entryId || [...removed].some(id => refs(e, id))) { if (e.op.endsWith('.create')) removed.add(e.target); return false; }
      return true;
    });
  }
  async discard(entryId: string) { this.requireEdit(); this.drop(entryId); await this.edited(); }
  async editFailed(entryId: string, body: object) {
    this.requireEdit(); const e = this.data.outbox.find(e => e.id === entryId && e.state === 'failed');
    if (!e) return;
    if (e.op === 'fraction.create' && 'sources' in body) {
      const nextSources = (body as { sources: SourceInput[] }).sources, oldSources = e.body.sources as SourceInput[];
      e.sourceIds = nextSources.map(s => { const index = oldSources.findIndex(old => old.atlas_grain_id === s.atlas_grain_id && (old.atlas_spot_id ?? null) === (s.atlas_spot_id ?? null)); return index >= 0 ? e.sourceIds?.[index] ?? local() : local(); });
    }
    e.body = { ...e.body, ...clone(body) }; e.state = 'pending'; delete e.error; e.attempted = false; e.key = crypto.randomUUID();
    await this.edited();
  }
  async remember(set: string, batch: string) {
    this.requireEdit(); this.data.lastSet = set; this.data.lastBatch[set] = batch; await this.persist(); this.notify();
  }
  localName(grain: string, sample: SampleRef, mineral: Mineral) {
    sample = this.data.samples[grain] ?? this.fractions.find(f => f.sources.some(s => s.atlas_grain_id === grain))?.sample ?? sample;
    return { name: localSuggestion(this.fractions, sample, mineral), sample, checked: false };
  }
  async suggestion(grain: string, sample: SampleRef, mineral: Mineral) {
    sample = this.localName(grain, sample, mineral).sample;
    const checked = false;
    if (this.enabled) try {
      const suggestion = await this.api.suggest(grain, mineral); sample = suggestion.sample;
      this.data.samples[grain] = sample; await this.persist();
      const pending = localSuggestion(this.fractions, sample, mineral);
      return { name: Number(pending.match(/\d+/)?.[0]) > Number(suggestion.suggestion.match(/\d+/)?.[0]) ? pending : suggestion.suggestion, sample, checked: true };
    } catch { /* Cached sample and pending fractions support working offline. */ }
    return { name: localSuggestion(this.fractions, sample, mineral), sample, checked };
  }
  wake() { if (this.automatic && this.enabled && this.data.outbox.some(e => e.state === 'pending')) this.scheduler.wake(); }
  mountSynced() {
    for (const e of this.data.outbox) if (e.state === 'waiting') { e.state = 'pending'; delete e.error; }
    this.notify(); void this.persist().then(() => this.wake()).catch(() => {});
  }
  async refresh() {
    if (!this.enabled || this.busy) return;
    return this.refreshing ??= this.refreshLists().finally(() => { this.refreshing = undefined; });
  }
  private async refreshLists() {
    try {
      const path = `/v1/fractions?${new URLSearchParams({ mount: this.mount, limit: '500' })}`;
      const old = this.data.fractions;
      await this.api.list<FractionOut>(path, this.data.pages, rows => {
        if (!this.enabled) return;
        // Show the first page at once; retain later cached rows until a complete read.
        const ids = new Set(rows.map(r => r.id));
        this.data.fractions = [...rows, ...old.filter(f => !ids.has(f.id))]; this.notify();
      }).then(rows => { if (this.enabled) this.data.fractions = [...rows, ...old.filter(f => f.retired_at && !rows.some(r => r.id === f.id))]; });
      if (!this.enabled) return;
      this.offline = false; this.error = ''; await this.persist(); this.notify();
    } catch (e) { if (this.enabled) { this.offline = !(e instanceof SyncApiError); this.error = e instanceof Error ? e.message : String(e); this.notify(); } }
  }
  async loadSets() {
    try { this.data.sets = await this.api.list<CapsuleOut>('/v1/tims/capsule-sets?active=true&limit=500', this.data.pages); await this.persist(); this.notify(); }
    catch (e) { this.error = e instanceof Error ? e.message : String(e); this.notify(); }
  }
  async loadBatches(set: string) {
    try {
      const batches = await this.api.list<BatchDetail>(`/v1/tims/batches?${new URLSearchParams({ capsule_set: set, limit: '500' })}`, this.data.pages);
      this.data.batches = [...this.data.batches.filter(b => b.capsule_set_id !== set), ...batches.map(b => ({ ...b, capsule_map: b.capsule_map ?? this.data.details[b.id]?.value.capsule_map ?? {} }))];
      await this.persist(); this.notify();
    } catch (e) { this.error = e instanceof Error ? e.message : String(e); this.notify(); }
  }
  async loadBatch(id: string) {
    if (!id || id.startsWith('local:')) return;
    try {
      const detail = await this.api.detail<BatchDetail>(`/v1/tims/batches/${encodeURIComponent(id)}`, this.data.details[id]);
      this.data.details[id] = detail; this.data.batches = [...this.data.batches.filter(b => b.id !== id), detail.value];
      if (detail.value.capsule_set && !this.data.sets.some(s => s.id === detail.value.capsule_set!.id)) this.data.sets.push(detail.value.capsule_set);
      this.notify();
      this.data.batchFractions[id] = detail.value.fractions ?? await this.api.list<FractionOut>(`/v1/fractions?${new URLSearchParams({ batch: id, limit: '500' })}`, this.data.pages);
      await this.persist(); this.notify();
    } catch (e) { this.error = e instanceof Error ? e.message : String(e); this.notify(); }
  }
  newBatch(body: BatchInput, mineral: Mineral) { return this.createBatch(body, mineral); }
  private async prepareBatch(entry: OutboxEntry) {
    if (entry.op !== 'batch.create' || entry.attempted || !entry.mineral || entry.body.recipe_id) return;
    const set = this.data.sets.find(s => s.id === entry.body.capsule_set_id);
    const recipes = await this.api.list<{ id: string }>(`/v1/tims/recipes?${new URLSearchParams({ lab: set?.lab_id ?? '', mineral: entry.mineral, active: 'true', limit: '500' })}`, this.data.pages);
    if (recipes.length === 1) entry.body.recipe_id = recipes[0].id;
  }
  fractionName(id: string) {
    return this.fractions.find(f => f.id === id)?.name ?? Object.values(this.data.batchFractions).flat().find(f => f.id === id)?.name ?? 'Taken';
  }
  private async send(e: OutboxEntry) {
    const root = `/v1/fractions/${encodeURIComponent(e.target)}`;
    const path = e.op === 'batch.create' ? '/v1/tims/batches' : e.op === 'fraction.create' ? '/v1/fractions' : e.op === 'fraction.retire' ? `${root}/retire` : e.op === 'source.add' ? `${root}/sources` : e.op === 'source.retire' ? `${root}/sources/${encodeURIComponent(e.source!)}/retire` : root;
    return this.api.write<BatchDetail | FractionOut | SourceOut>(path, e.op === 'fraction.patch' ? 'PATCH' : 'POST', e.body, { key: e.key, keyChanged: async key => { e.key = key; await this.persist(); } });
  }
  /** Entries sharing a batch, fraction or grain keep their durable queue order.
   * Unrelated work can pass a refusal or a source waiting for mount sync. */
  private touches(e: OutboxEntry): Set<string> {
    const keys = new Set([`${e.op === 'batch.create' ? 'batch' : 'fraction'}:${e.target}`]);
    if (e.source) keys.add(`source:${e.source}`);
    const base = this.data.fractions.find(f => f.id === e.target);
    for (const batch of [e.body.batch_id, base?.batch_id]) if (batch) keys.add(`batch:${batch}`);
    for (const source of [...(base?.sources ?? []), ...(e.body.sources as SourceInput[] ?? []), ...(e.op === 'source.add' ? [e.body as unknown as SourceInput] : [])]) {
      keys.add(`grain:${source.atlas_grain_id}`);
      if (source.atlas_spot_id) keys.add(`spot:${source.atlas_spot_id}`);
    }
    return keys;
  }
  private nextPending() {
    const earlier = new Set<string>();
    for (const entry of this.data.outbox) {
      const keys = this.touches(entry);
      if (entry.state === 'pending' && ![...keys].some(key => earlier.has(key))) return entry;
      for (const key of keys) earlier.add(key);
    }
  }
  async flush() {
    if (!this.enabled || this.busy || !this.writable() || !this.data.outbox.length) return;
    this.busy = true; this.notify();
    try {
      await this.refreshing;
      for (;;) {
        const e = this.nextPending();
        if (!e || !this.enabled || !this.writable()) break;
        // Save storage errors separately: no network request can precede durability.
        await this.persist();
        try {
          await this.prepareBatch(e);
          if (!this.enabled || !this.writable()) return;
          e.attempted = true; await this.persist();
          const result = await this.send(e);
          // Always persist an acknowledgement to this exact account's mirror, even if hidden.
          if (e.op === 'batch.create') {
            const b = result as BatchDetail; this.data.batches.push({ ...b, capsule_map: b.capsule_map ?? {} });
          } else if (e.op === 'source.add' || e.op === 'source.retire') {
            const source = result as SourceOut;
            this.data.fractions = this.data.fractions.map(f => f.id === e.target ? { ...f, sources: e.op === 'source.add' ? [...f.sources, source] : f.sources.filter(s => s.id !== e.source) } : f);
          } else {
            const fraction = result as FractionOut;
            this.data.fractions = [...this.data.fractions.filter(f => f.id !== fraction.id), fraction];
          }
          this.data.outbox = this.data.outbox.filter(entry => entry.id !== e.id);
          if (e.op === 'fraction.create') {
            const sources = (result as FractionOut).sources;
            (e.body.sources as SourceInput[]).forEach((source, index) => {
              const real = sources.find(s => s.atlas_grain_id === source.atlas_grain_id && (s.atlas_spot_id ?? null) === (source.atlas_spot_id ?? null));
              if (real) { const temp = e.sourceIds?.[index] ?? `${e.target}:source:${index}`; this.remap(temp, real.id); }
            });
          }
          if (e.op.endsWith('.create') || e.op === 'source.add') {
            const temp = e.op === 'source.add' ? e.source! : e.target;
            this.remap(temp, result.id);
          }
          await this.persist(); this.offline = false; this.error = ''; this.notify();
        } catch (error) {
          if (error instanceof ServerReadOnlyError) {
            this.error = error.message; this.notify(); return;
          } else if (error instanceof SyncApiError && ['grain_not_registered', 'spot_not_registered'].includes(error.code ?? '')) {
            e.state = 'waiting'; e.error = 'Waits for the mount to sync';
          } else if (error instanceof SyncApiError && error.status >= 400 && error.status < 500 && !error.transient) {
            e.state = 'failed'; e.error = error.status === 403 ? 'Only lab members can pluck grains for this sample' : error.message;
          } else {
            this.offline = true; this.error = error instanceof Error ? error.message : String(error); if (this.automatic) this.scheduler.retry();
            await this.persist(); this.notify(); return;
          }
          await this.persist(); this.notify();
        }
      }
      this.scheduler.reset();
    } finally { this.busy = false; this.notify(); }
    if (!this.data.outbox.length && this.enabled) await this.refresh();
  }
}
