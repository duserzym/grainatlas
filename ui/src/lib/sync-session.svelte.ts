import type { AppState } from './store.svelte';
import { authToken, publish } from './atlas';
import { SyncApi, changesFromEvents, type Candidate, type ProjectStatus } from './sync-api';
import { ensureDraft, listDrafts, pendingElements, readDraft, writeDraft, type Draft } from './drafts';
import { pixelSignature } from './sync-pixels';
import { applyChanges, changeCounts, chipText, diffElements, emptyElements, groupedGrains, hasChanges, mergeElements, resolveMerge, syncState, uiSpots, validateLineage, type Elements, type Merge, type Resolution } from './sync';

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
export class SyncSession {
  private app: AppState;
  draft = $state.raw<Draft | null>(null);
  status = $state.raw<ProjectStatus | null>(null);
  registered = $state<boolean | undefined>(undefined);
  reachable = $state<boolean | null>(null);
  busy = $state(false);
  dialog = $state<'register' | 'review' | 'pixels' | null>(null);
  candidates = $state.raw<Candidate[]>([]);
  review = $state.raw<{ merge: Merge; theirs: Elements; revision: number; hashes?: ProjectStatus['hashes'] } | null>(null);
  syncedAt = $state<string | null>(null);
  error = $state('');
  private uuid: string | null = null;
  private viewRevision = $state<number | null>(null);
  private pendingSync = false;
  private viewHashes = $state.raw<ProjectStatus['hashes'] | null>(null);
  private get ready() { return !!this.app.project && !this.app.loadingProject && !this.app.storedCopy; }
  private isCurrent(identity: string) { return this.ready && this.identity === identity; }
  constructor(app: AppState) { this.app = app; }
  get elements(): Elements { return { spots: this.app.spots, labels: this.app.labels, grains: Object.entries(this.app.grains).flatMap(([area_id, grains]) => grains.map(g => ({ ...g, area_id, sample: g.sample !== undefined ? g.sample : this.app.project?.areas.find(a => a.id === area_id)?.sample ?? null }))) }; }
  get counts() { if (this.app.readOnly) return { spots: 0, grains: 0, labels: 0 }; return changeCounts(diffElements(this.draft?.base ?? emptyElements(), this.elements)); }
  get pixelsChanged(): boolean {
    if (!this.app.pkg || !Object.keys(this.app.stitched).length || !this.app.project) return false;
    return this.currentPixels() !== this.draft?.pixels_base;
  }
  private currentPixels(): string {
    const p = this.app.fullProject()!;
    p.areas = p.areas.map(a => ({ ...a, params: this.app.params[a.id] ?? a.params, mosaic: this.app.stitched[a.id]?.mosaic ?? a.mosaic }));
    return pixelSignature(p);
  }
  get state() {
    const hashes = this.app.readOnly ? this.viewHashes : this.draft?.base_hashes;
    const serverChanged = hashes && this.status ? (['spots', 'grains', 'labels'] as const).some(k => hashes[k] !== this.status!.hashes[k]) : undefined;
    return syncState({ serverChanged, registered: this.registered, reachable: this.reachable, baseRevision: this.app.readOnly ? this.viewRevision : this.draft?.base_revision ?? null, serverRevision: this.status?.revision ?? null, localChanges: this.counts, pixelsChanged: this.pixelsChanged }); }
  get text(): string {
    return chipText(this.state, this.counts, this.status?.atlas_version ?? null, this.status?.revision ?? null, this.pixelsChanged) + (this.syncedAt && this.state === 'in-sync' ? ` · Synced ${this.syncedAt}` : '');
  }
  get unsynced(): boolean {
    return !!this.app.project && !this.app.readOnly && (!this.draft?.base || hasChanges(this.counts) || this.pixelsChanged);
  }
  private get identity(): string { return JSON.stringify([this.app.project?.uuid, this.app.projectPath?.path ?? null, this.app.remote?.projectId ?? null]); }
  private async api() {
    const server = this.app.remote?.server ?? this.draft?.server ?? this.app.server;
    if (!server) throw new Error('Set the database server in Settings, then Retry.');
    return new SyncApi(server, await authToken(this.app), this.app.remote?.access ?? null);
  }
  private save(patch: Partial<Draft>) {
    if (!this.draft || this.app.loadingProject || this.app.storedCopy) return;
    this.draft = { ...this.draft, ...clone(patch) };
    writeDraft(this.draft);
  }
  track(edited = true) {
    const p = this.app.project;
    if (!p || this.app.readOnly || this.app.loadingProject || this.app.storedCopy) return;
    if (!this.draft || this.draft.uuid !== p.uuid) this.draft = ensureDraft(p, this.app.server);
    const key = this.app.projectPath ? `grainatlas.copy.${p.uuid}|${this.app.projectPath.path}` : `grainatlas.project.${p.package.name}`;
    const changes = this.draft.base ? this.counts : null;
    const pending = !this.draft.base || hasChanges(changes) ? pendingElements(this.elements) : null;
    const now = new Date().toISOString();
    this.save({ changes, pending, pending_at: pending ? now : null, pixels_changed: this.pixelsChanged, updated_at: edited ? now : this.draft.updated_at, source_keys: [...new Set([...(this.draft.source_keys ?? []), key])] });
  }
  /** Every opening path calls this before returning. Later polling never replaces the base. */
  async check() {
    const p = this.app.project;
    const identity = this.identity;
    if (!p || this.busy || this.app.loadingProject || this.app.storedCopy) return;
    if (p.uuid !== this.uuid) {
      this.uuid = p.uuid;
      this.draft = listDrafts(this.app.remote?.server ?? this.app.server).find(d => d.uuid === p.uuid) ?? readDraft(p.uuid) ?? ensureDraft(p, this.app.remote?.server ?? this.app.server);
      writeDraft(this.draft); // ready open adopts even when the subsequent lookup is offline
      this.registered = undefined; this.status = null; this.reachable = null; this.review = null; this.dialog = null; this.syncedAt = null; this.viewRevision = null; this.viewHashes = null; this.candidates = this.draft.link_candidates ?? [];
    }
    this.track(false);
    this.busy = true;
    try {
      const api = await this.api();
      const status = await api.lookup(this.draft?.linked_uuid ?? p.uuid);
      if (!this.isCurrent(identity)) return;
      if (this.app.readOnly && this.viewRevision === null) { this.viewRevision = status?.revision ?? null; this.viewHashes = status?.hashes ?? null; }
      this.status = status; this.registered = !!status; this.reachable = true;
      if (this.app.error === this.error) this.app.error = null;
      this.error = '';
      if (!status) {
        if (!this.draft?.link_offered) {
          const candidates = await api.candidates(p.package.name);
          if (this.identity !== identity || this.app.loadingProject || this.app.storedCopy) return;
          this.save({ link_offered: true, link_candidates: candidates });
          this.candidates = candidates;
        }
        this.track(false);
        return;
      }
      if (this.app.project) this.app.project.laicpmsdb.mount_id = status.mount_id;
      this.save({ server: this.app.remote?.server ?? this.app.server ?? this.draft?.server, mount: status.mount_id, synced: this.draft?.synced ?? true });
      if (!this.draft?.base) {
        // Critical for legacy copies: the server is the base, the local edits remain mine.
        const theirs = await api.elements(status.project_uuid);
        if (!this.isCurrent(identity)) return;
        this.save({ base: { spots: theirs.spots, grains: theirs.grains, labels: theirs.labels }, base_revision: theirs.revision, base_hashes: theirs.hashes });
      }
      if (this.draft?.pixels_base === undefined) {
        const published = await api.publishedProject(status.project_uuid);
        if (!this.isCurrent(identity)) return;
        this.save({ pixels_base: published ? pixelSignature(published) : null });
      }
      this.track(false);
    } catch (e) {
      this.reachable = false;
      this.error = e instanceof Error ? e.message : String(e);
    } finally { this.busy = false; if (this.identity !== identity) void this.check(); }
    if (this.pendingSync && this.reachable && this.state === 'local-changes') { this.pendingSync = false; await this.sync(); }
  }
  dismissLink() { this.candidates = []; this.save({ link_offered: true, link_candidates: [] }); }
  async link(candidate: Candidate) {
    if (!this.app.readOnly) {
      try { await (await this.app.core).setLink(candidate.mount_id, {}); } catch (e) { this.failed(e); return; }
    }
    this.save({ linked_uuid: candidate.project_uuid, mount: candidate.mount_id, link_offered: true, link_candidates: [] });
    this.candidates = [];
    this.dialog = null;
    await this.check();
  }
  viewed(revision: number | null) { this.viewRevision = revision; this.viewHashes = this.status?.revision === revision ? this.status.hashes : null; }
  async editBase() { await this.check(); this.track(); }
  async action() {
    if (this.busy) return;
    if (this.state === 'offline' || this.state === 'checking') { await this.check(); return; }
    if (this.state === 'unregistered') { this.dialog = 'register'; return; }
    if (this.state === 'server-newer') { await this.update(); return; }
    if (this.state === 'both-changed') { await this.prepareReview(); return; }
    if (this.pixelsChanged) { this.dialog = 'pixels'; return; }
    await this.sync();
  }
  async register(mount: string, publishPixels = false) {
    if (!mount) return;
    try { await (await this.app.core).setLink(mount, {}); } catch (e) { this.failed(e); return; }
    this.save({ mount });
    if (this.app.project) this.app.project.laicpmsdb.mount_id = mount;
    this.dialog = null;
    await this.sync(publishPixels);
  }
  private setElements(elements: Elements) {
    this.app.checkpoint();
    this.app.spots = uiSpots(clone(elements.spots), elements.grains, this.app.spots, Object.fromEntries(this.app.project!.areas.map(a => [a.id, a.sample]))); this.app.grains = groupedGrains(clone(elements.grains), this.app.grains); this.app.labels = clone(elements.labels);
    this.app.selection = []; this.app.persist();
  }
  async update() {
    if (this.busy || !this.status) return;
    // Recheck local edits at invocation, so a newly made edit is never overwritten.
    if (hasChanges(this.counts)) { await this.prepareReview(); return; }
    this.busy = true;
    const identity = this.identity;
    try {
      const theirs = await (await this.api()).elements(this.status.project_uuid);
      if (!this.isCurrent(identity)) return;
      if (hasChanges(this.counts)) { this.openReview(theirs, theirs.revision, theirs.hashes); return; }
      this.setElements(theirs);
      if (!this.app.readOnly || !this.draft?.source_keys?.length) this.save({ base: { spots: theirs.spots, grains: theirs.grains, labels: theirs.labels }, base_revision: theirs.revision, base_hashes: theirs.hashes });
      this.status = { ...this.status, revision: theirs.revision, hashes: theirs.hashes };
      this.viewRevision = theirs.revision; this.viewHashes = theirs.hashes;
      this.track(false); this.reachable = true;
    } catch (e) { this.failed(e); } finally { this.busy = false; if (this.identity !== identity) void this.check(); }
  }
  private openReview(theirs: Elements, revision: number, hashes?: ProjectStatus['hashes']) {
    this.review = { merge: mergeElements(this.draft?.base ?? emptyElements(), clone(this.elements), theirs), theirs: clone(theirs), revision, hashes };
    this.dialog = 'review';
  }
  async prepareReview() {
    if (this.busy || !this.status) return;
    this.busy = true;
    const identity = this.identity;
    try {
      const api = await this.api();
      if (this.draft?.base && this.draft.base_revision !== null) {
        const history = await api.changes(this.status.mount_id, this.draft.base_revision);
        if (this.isCurrent(identity)) this.openReview(applyChanges(this.draft.base, changesFromEvents(history.changes, this.status.project_uuid)), history.current_revision);
      } else {
        const theirs = await api.elements(this.status.project_uuid);
        if (this.isCurrent(identity)) this.openReview(theirs, theirs.revision, theirs.hashes);
      }
    }
    catch (e) { this.failed(e); } finally { this.busy = false; if (this.identity !== identity) void this.check(); }
  }
  async apply(choices: Record<string, Resolution>, publishPixels = false) {
    const review = this.review;
    if (!review) return;
    const result = resolveMerge(review.merge, choices);
    this.setElements(result);
    this.save({ base: review.theirs, base_revision: review.revision, base_hashes: review.hashes ?? null });
    this.dialog = null; this.review = null;
    await this.sync(publishPixels);
  }
  private failed(e: unknown) { this.error = e instanceof Error ? e.message : String(e); this.app.error = this.error; this.reachable = false; }
  /** Capture the submitted snapshot: edits made while the request is in flight stay dirty. */
  async sync(publishPixels = false) {
    if (this.busy || !this.ready || !this.app.project || !this.draft?.mount || this.app.readOnly) return;
    this.track(false);
    this.busy = true;
    const identity = this.identity;
    const draft = this.draft;
    const projectUuid = draft.linked_uuid ?? this.app.project.uuid;
    const mine = clone(this.elements);
    try {
      const api = await this.api();
      if (!this.isCurrent(identity)) return;
      const mode = draft.synced ? 'delta' : 'snapshot';
      const diff = diffElements(mode === 'snapshot' ? emptyElements() : draft.base ?? emptyElements(), mine);
      validateLineage(draft.base ?? emptyElements(), mine, diff);
      const reply = await api.sync(draft.mount!, { project_uuid: projectUuid, base_revision: draft.base_revision, mode, ...diff });
      if (!this.isCurrent(identity)) {
        // A different project opened during the POST. Never put the old response in its tab.
        // Leave the old draft conservative for a fresh check; its persisted content is intact.
        return;
      }
      this.reachable = true;
      if (!reply.ok) {
        const theirs = applyChanges(draft.base ?? emptyElements(), changesFromEvents(reply.changes, projectUuid));
        this.openReview(theirs, reply.current_revision);
        return;
      }
      // ids maps client UUIDs to server ids; it never renames local elements/references.
      this.save({ base: mine, base_revision: reply.revision, base_hashes: null, synced: true });
      this.track(false);
      // Snapshot omissions remain on the server. Incorporate them without overwriting edits
      // made since submission; never acknowledge a later revision we did not submit.
      if (mode === 'snapshot') {
        const accepted = await api.elements(projectUuid);
        if (!this.isCurrent(identity)) return;
        if (accepted.revision === reply.revision) {
          const combined = mergeElements(mine, clone(this.elements), accepted);
          if (hasChanges(changeCounts(diffElements(mine, accepted)))) this.setElements(combined.combined);
          this.save({ base: accepted, base_hashes: accepted.hashes });
          this.track(false);
        }
      }
      this.pendingSync = false;
      this.registered = true;
      const status = await api.lookup(projectUuid);
      if (!this.isCurrent(identity)) return;
      this.status = status;
      if (status?.revision === reply.revision) this.save({ base_hashes: status.hashes });
      this.syncedAt = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
      if (this.app.error === this.error) this.app.error = null;
      this.error = '';
      this.track();
      if (publishPixels && this.app.pkg) {
        const before = this.app.lastPublish?.at;
        await publish(this.app);
        if (this.app.lastPublish?.at && this.app.lastPublish.at !== before) {
          // Only acknowledge pixels after a successful publish made by this invocation.
          const published = await api.publishedProject(this.draft.linked_uuid ?? this.app.project.uuid);
          if (!this.isCurrent(identity)) return;
          if (published) this.save({ pixels_base: pixelSignature(published) });
          this.status = await api.lookup(this.draft.linked_uuid ?? this.app.project.uuid);
          this.track();
        }
      }
    } catch (e) { if (this.identity === identity) { this.pendingSync = true; this.failed(e); } } finally { this.busy = false; if (this.identity !== identity) void this.check(); }
  }
  async afterSave() {
    if (!this.ready) return;
    this.track();
    if (this.app.link.syncOnSave && this.registered && this.reachable) {
      if (this.pixelsChanged) this.dialog = 'pixels';
      else await this.sync();
    }
  }
}
