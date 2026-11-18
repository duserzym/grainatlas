import { staleOpenElements, automaticSpotRemovals, type MissingElements } from './stale-open';
import { isHosted } from '@lab/bridge';
import { BackgroundSync } from './background-sync';
import { bundlePixels, saveBundleTones } from './bundle-tone';
import { deferredWrites, flushWrites } from './deferred-writes';
import { canWrite } from './presentation';
import { hostedSync } from './hosted-sync';
import type { AtlasSyncConfirmation } from '@lab/bridge';
import type { AppState } from './store.svelte';
import { authToken, publish, performPublish } from './atlas';
import { PublishError, waitFor, type PublishJob } from './publish-engine';
import { imagesMayRun } from './image-idle';
import { SyncApi, SyncApiError, SyncNetworkError, lineageFailureMessage, changesFromEvents, problemSummary, requestIdFor, syncWireBody, type Candidate, type ProjectStatus, type SyncBody } from './sync-api';
import { canonicalJson } from './sync-hash';
import { notifyUpbLab } from './upblab-notice';
import { draftElements, ensureDraft, listDrafts, pendingElements, readDraft, writeDraft, type Draft } from './drafts';
import { pixelSignature } from './sync-pixels';
import { draftSyncStatus, plainSyncStatus } from './sync-status';
import { appStorage } from './app-storage';
import { appliedCounts, applyChanges, changeCounts, chipText, diffElements, elementDiffer, emptyElements, groupedGrains, hasChanges, mergeElements, needsFirstSnapshot, resolveMerge, syncState, uiSpots, validateLineage, viewElements, syncChanges, syncIntentFailure, storedElements, projectElements, databaseAdditions, keepAdditions, firstSyncMerge, withKnownLineage, appliedPart, elementLabel, nameList, KINDS, type HeldItem, type Kind, type Elements, type ReadElements, type Merge, type Resolution } from './sync';

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
interface HeldSync { identity: string; mine: Elements; current: ReadElements; body: SyncBody; publishPixels: boolean; counts: AtlasSyncConfirmation }
/** When each server was last found unreachable: a re-check (e.g. after Edit) of a server that
 * just failed does not block spot deletion again. */
const unreachableAt = new Map<string, number>();
/** Tests only: forget remembered failures. */
export function forgetUnreachable() { unreachableAt.clear(); }
const recentlyUnreachable = (server: string | null | undefined) => !!server && unreachableAt.has(server) && Date.now() - unreachableAt.get(server)! < 120_000;

export class SyncSession {
  private app: AppState;
  draft = $state.raw<Draft | null>(null);
  status = $state.raw<ProjectStatus | null>(null);
  registered = $state<boolean | undefined>(undefined);
  reachable = $state<boolean | null>(null);
  busy = $state(false);
  checking = $state(false);
  private suppressDraft = false;
  staleOpen = $state.raw<MissingElements | null>(null);
  staleBulk = $state(false);
  private editedSinceOpen = false;
  private deletionCheckDeadline = 0;
  /** Also called after adopting pending elements on the Open tab. */
  opened() {
    this.automatic.cancel(); this.uuid = null; this.editedSinceOpen = false;
    this.staleBulk = false;
    const p = this.app.project;
    const draft = p ? listDrafts(this.app.remote?.server ?? this.app.server).find(d => d.uuid === p.uuid) ?? readDraft(p.uuid) : null;
    this.staleOpen = this.app.readOnly ? null : staleOpenElements(draft?.base, this.elements);
    this.dialog = null;
  }
  showStaleOpen() { if (this.staleOpen) this.dialog = 'stale'; }
  bringBack() {
    if (!canWrite() || this.app.readOnly || this.busy || !this.staleOpen || !this.draft?.base) return;
    const ids = this.staleOpen;
    const base = clone(this.draft.base);
    const additions = storedElements(base);
    additions.spots = base.spots.filter(s => ids.spots.includes(s.id));
    additions.grains = additions.grains.filter(g => ids.grains.includes(g.id));
    additions.labels = additions.labels.filter(l => ids.labels.includes(l.id));
    this.save({ pending_request: null });
    this.keepDatabase(additions, true);
    this.editedSinceOpen = true;
    this.staleOpen = null; this.staleBulk = false; this.dialog = null;
    this.track(false); this.refreshAutomatic();
  }
  async removeStale() {
    if (!canWrite() || this.app.readOnly || this.busy || !this.staleOpen) return;
    this.editedSinceOpen = true;
    this.staleOpen = null; this.staleBulk = false; this.dialog = null;
    this.save({ pending_request: null });
    await this.sync();
  }
  private holdAutomaticDeletes(deleted: string[]) {
    if (!this.automaticRunning) return false;
    const spots = automaticSpotRemovals(this.draft?.base, deleted);
    if (!spots.length) return false;
    this.staleOpen = { spots, grains: [], labels: [] }; this.staleBulk = true;
    this.dialog = 'stale'; this.pendingSync = false; this.automatic.cancel();
    return true;
  }
  dialog = $state<'register' | 'review' | 'pixels' | 'bulk' | 'stale' | 'replace-images' | null>(null);
  confirmation = $state.raw<AtlasSyncConfirmation | null>(null);
  private heldSync: HeldSync | null = null;
  showConfirmation() { if (this.heldSync && this.heldSync.identity === this.identity && !this.app.readOnly) this.dialog = 'bulk'; }
  cancelConfirmation() { this.dialog = null; }
  keepBulk() {
    const held = this.heldSync;
    if (!canWrite() || this.app.readOnly || this.busy || !held || held.identity !== this.identity) return;
    const grainIds = new Set([...held.body.grains.retire, ...held.body.grains.delete,
      ...held.body.grains.upsert.flatMap(g => [...(g.split_from ? [g.split_from] : []), ...(g.merged_from ?? [])])]);
    const spotIds = new Set(held.body.spots.delete);
    const database = storedElements(held.current);
    const restored = { grains: database.grains.filter(g => grainIds.has(g.id)), spots: held.current.spots.filter(s => spotIds.has(s.id)), labels: [] };
    // Keeping a parent also cancels the lineage instruction that would retire it.
    const local = { ...clone(this.elements), spots: this.elements.spots.filter(s => !spotIds.has(s.id)),
      grains: this.elements.grains.filter(g => !grainIds.has(g.id)).map(g => ({ ...g,
        ...(g.split_from && grainIds.has(g.split_from) ? { split_from: undefined } : {}),
        ...(g.merged_from ? { merged_from: g.merged_from.filter(id => !grainIds.has(id)) } : {}),
      })) };
    this.setElements(keepAdditions(local, restored));
    this.save({ database_added: keepAdditions(this.databaseAdded, restored) });
    this.confirmation = null; this.heldSync = null; this.dialog = null;
    this.track(false); this.refreshAutomatic();
  }
  async confirmBulk() {
    if (!canWrite()) return;
    const held = this.heldSync;
    if (!held || held.identity !== this.identity) return;
    this.dialog = null;
    await this.sync(held.publishPixels, false, held);
  }
  candidates = $state.raw<Candidate[]>([]);
  review = $state.raw<{ merge: Merge; theirs: ReadElements; revision: number; hashes?: ProjectStatus['hashes'] } | null>(null);
  syncedAt = $state<string | null>(null);
  error = $state('');
  failureMessage = $state('');
  validationFailed = $state(false);
  notice = $state('');
  private intentFailure = $state('');
  omissions = $state.raw<Record<Kind, string[]>>({ spots: [], grains: [], labels: [] });
  /** Items the last partial sync held back; they stay local changes until a sync applies them. */
  private heldBack = $state.raw<HeldItem[]>([]);
  /** Held items that are still local changes here (one reverted or deleted since drops out). */
  get held(): HeldItem[] {
    if (!this.heldBack.length) return [];
    const c = this.localChanges;
    const ids = new Set(KINDS.flatMap(k => [...c[k].upsert.map(e => e.id), ...c[k].delete, ...(k === 'grains' ? c.grains.retire : [])]));
    return this.heldBack.filter(h => ids.has(h.id));
  }
  /** Raw status and body of the last failure, for Details only. */
  errorDetails = $state('');
  /** A readable name for an element id, from this copy, the base or the database. */
  label(kind: Kind, id: string): string { return elementLabel(kind, id, this.elements, this.draft?.base, this.serverElements); }
  private cachedLastSeen = $derived.by<ReadElements>(() => Object.fromEntries(KINDS.map(k => [k, [...(this.draft?.base ?? this.draft?.base_seen ?? emptyElements())[k], ...(this.draft?.database_added ?? emptyElements())[k]]])) as unknown as ReadElements);
  private get lastSeen(): ReadElements { return this.cachedLastSeen; }
  private additions<K extends Kind>(kind: K, mine: Elements[K]): Elements[K] {
    const narrow = (rows: ReadElements[K]) => ({ ...this.emptyBase, [kind]: rows }) as ReadElements;
    return databaseAdditions(narrow(this.serverElements?.[kind] ?? []), narrow(this.lastSeen[kind]), { ...this.emptyBase, [kind]: mine }, this.status?.project_uuid ?? this.app.project?.uuid ?? '')[kind];
  }
  private moreSpots = $derived.by(() => this.additions('spots', this.app.spots));
  private moreGrains = $derived.by(() => this.additions('grains', this.cachedGrains));
  private moreLabels = $derived.by(() => this.additions('labels', this.app.labels));
  private cachedDatabaseMore = $derived.by(() => ({ spots: this.moreSpots, grains: this.moreGrains, labels: this.moreLabels }));
  get databaseMore(): Elements { return this.cachedDatabaseMore; }
  private cachedDatabaseUnresolved = $derived.by(() => {
    if (!KINDS.some(k => this.omissions[k].length)) return this.databaseMore;
    const own = storedElements(projectElements(this.serverElements ?? emptyElements(), this.status?.project_uuid ?? this.app.project?.uuid ?? ''));
    return keepAdditions(this.databaseMore, Object.fromEntries(KINDS.map(k => [k, own[k].filter(e => this.omissions[k].includes(e.id))])) as unknown as Elements);
  });
  get databaseUnresolved(): Elements { return this.cachedDatabaseUnresolved; }
  private cachedDatabaseContext = $derived.by(() => {
    return Object.fromEntries(KINDS.map(k => [k, this.omissions[k].filter(id => !this.databaseUnresolved[k].some(e => e.id === id))])) as Record<Kind, string[]>;
  });
  get databaseContext(): Record<Kind, string[]> { return this.cachedDatabaseContext; }
  get databaseAdded(): Elements { return this.draft?.database_added ?? emptyElements(); }
  private keepDatabase(additions: Elements, cancelRetirements = false) {
    if (!KINDS.some(k => additions[k].length)) return;
    const local = clone(this.elements);
    if (cancelRetirements) {
      const parents = new Set(additions.grains.map(g => g.id));
      local.grains = local.grains.map(g => ({ ...g,
        ...(g.split_from && parents.has(g.split_from) ? { split_from: undefined } : {}),
        ...(g.merged_from ? { merged_from: g.merged_from.filter(id => !parents.has(id)) } : {}),
      }));
    }
    this.setElements(keepAdditions(local, additions));
    this.save({ database_added: keepAdditions(this.databaseAdded, additions) });
    this.track(false);
  }
  keepDatabaseKind(kind: Kind) { if (this.app.readOnly || this.busy) return; this.keepDatabase({ ...emptyElements(), [kind]: this.databaseUnresolved[kind] }); }
  private setStatus(status: ProjectStatus | null) {
    const checkedRevision = status?.revision;
    if (status) {
      // A visible revision must have its measured-spot list already in memory.
      if (this.elementsRevision === null) return;
      const acknowledged = this.draft?.synced && this.draft.mount === status.mount_id ? this.draft.base_revision ?? 0 : 0;
      status = { ...status, revision: Math.min(this.elementsRevision, Math.max(status.revision, acknowledged, this.status?.mount_id === status.mount_id ? this.status.revision : 0)) };
    }
    this.status = status;
    if (checkedRevision != null) this.save({ checked_at: new Date().toISOString(), server_revision: checkedRevision, server_can_edit: status?.can_edit });
  }
  private snapshotNeeded = $state(false);
  private checkedElements: { identity: string; projectUuid: string; fetchedAt: number; elements: ReadElements } | null = null;
  private serverElements = $state.raw<ReadElements | null>(null);
  private serverSpots = $state.raw<Elements['spots']>([]);
  get syncDisabledReason(): string { return this.draft?.link_conflict ? 'This project is linked to another mount' : this.status?.can_edit === false ? 'Read-only access: you do not have permission to sync this mount.' : ''; }
  /** The database was found unreachable by this session or, within two minutes, by a re-check. */
  get knownUnreachable(): boolean { return this.reachable === false || recentlyUnreachable(this.app.remote?.server ?? this.draft?.server ?? this.app.server); }
  refuseSpotDeletion(ids: string[]): boolean {
    if ([...this.serverSpots, ...(this.draft?.base?.spots ?? []), ...this.elements.spots].some(s => ids.includes(s.id) && s.acquisition_ids?.length)) {
      this.app.notice = 'This spot has measurements in the database; it cannot be deleted there';
      return true;
    }
    // Until the first measured-spot list is in, nothing shown can vouch for these spots. Later
    // checks keep the previous list and the revision it belongs to, so deletes stay allowed.
    // Once a check has found the database unreachable, a retry in flight does not block deletes:
    // measured spots stay protected through the acquisition links kept in the draft base.
    if (ids.length && this.checking && this.reachable !== false && !recentlyUnreachable(this.app.remote?.server ?? this.draft?.server ?? this.app.server) && Date.now() < this.deletionCheckDeadline && this.elementsRevision === null && this.registered !== false) {
      this.app.notice = 'Checking which spots are measured…';
      return true;
    }
    return false;
  }
  private uuid: string | null = null;
  private viewBase = $state.raw<ReadElements | null>(null);
  private viewRevision = $state<number | null>(null);
  private pendingSync = false;
  private automaticRunning = false;
  private disposed = false;
  private automatic = new BackgroundSync(() => { void this.runAutomatic(); });
  private get automaticEnabled() {
    return !this.disposed && canWrite() && !isHosted() && !this.app.hosted && this.app.link.syncAutomatically !== false
      && this.ready && !this.app.readOnly && !!this.draft?.mount
      && (this.registered === true && this.status?.can_edit === true || !!this.draft?.server_can_edit && !!this.draft?.pending_request)
      && !this.staleOpen && !this.imageDecision && !this.confirmation && !this.review && !this.dialog && !this.held.length && !this.intentFailure
      && !this.draft?.link_conflict && !(this.error && this.reachable !== false);
  }
  get backgroundPending() { return this.automaticEnabled && (!!this.draft?.pending_request || !!this.cachedSyncStatus?.elements || this.pendingSync); }
  refreshAutomatic() {
    this.refreshImages();
    if (!this.backgroundPending) { this.automatic.cancel(); return; }
    if (!this.busy) {
      if (this.reachable === false) this.automatic.retry();
      else this.automatic.ensure();
    }
  }
  wakeAutomatic() {
    if (this.imagePending) { this.imageTimer.wake(); return true; }
    if (this.backgroundPending) { if (!this.busy) this.automatic.wake(); return true; }
    return false;
  }
  dispose() { this.disposed = true; this.automatic.cancel(); this.imageTimer.cancel(); }
  private async runAutomatic() {
    if (!this.backgroundPending || this.busy) return;
    await this.runQuietSync();
  }
  private async runQuietSync() {
    if (this.busy) return;
    this.automaticRunning = true;
    try { await this.sync(); }
    finally { this.automaticRunning = false; }
  }
  imageBusy = $state(false);
  imageProgress = $state.raw<{ done: number; total: number } | null>(null);
  imageError = $state('');
  private imageOffline = $state(false);
  private imageRetryAfter = 0;
  private lastImageEdit = Date.now();
  private imageEdits = 0;
  private imageRun: Promise<void> | null = null;
  private imageTimer = new BackgroundSync(() => { void this.publishImages(undefined, true); }, Math.random, 10_000);
  get imageDecision() { return this.draft?.publish_job?.decision === 'ask'; }
  private get imagePending() {
    return this.automaticEnabled && this.reachable === true && !this.knownUnreachable && !this.backgroundPending && !this.busy && !this.imageBusy
      && (this.pixelsChanged || !!this.draft?.publish_job?.result) && this.pixelKey !== null && !this.app.pixelsLoad && !this.imageDecision
      && !(this.draft?.publish_job?.decision === 'keep' && this.draft.publish_job.pixels_key === this.pixelKey);
  }
  private imageGates(foreground: boolean) {
    return {
      elementsIdle: foreground || !this.busy && !this.backgroundPending,
      attention: !!this.staleOpen || !!this.confirmation || !!this.review || !!this.dialog || !!this.app.checkPrompt || !!this.app.renamePrompt
        || !!this.app.leaveEditPrompt || !!this.app.storedCopy || !!this.app.offer || !!this.app.pendingLoose
        || typeof document !== 'undefined' && !!document.querySelector('dialog[open]'),
      visible: typeof document === 'undefined' || document.visibilityState !== 'hidden',
      loading: !!this.app.pixelsLoad || !!this.app.busy || this.app.rendering > 0 || Object.values(this.app.liveBusy).some(Boolean),
    };
  }
  /** Called at the edit itself, before the deferred write, so timers cannot race a paint. */
  imagesEdited() {
    this.lastImageEdit = Date.now(); this.imageEdits++;
    if (!this.imageBusy) this.imageTimer.cancel();
  }
  private refreshImages() {
    if (!this.imagePending) { this.imageTimer.cancel(); return; }
    if (this.imageOffline) this.imageTimer.retry(this.imageRetryAfter);
    else this.imageTimer.ensure();
  }
  private imageCheckpoint = '';
  savePublishJob(job: PublishJob | null) { this.save({ publish_job: job }); }
  async persistPublishJob() {
    const job = this.draft?.publish_job;
    if (!job) throw new DOMException('Image publishing stopped', 'AbortError');
    const key = JSON.stringify([job.server, job.mount, job.account, job.atlas_sha256, job.upload_id, job.start_key, job.commit_key, job.zip_key, job.replace_current, job.completed_at]);
    if (key === this.imageCheckpoint) return;
    flushWrites();
    if (!this.draft || !await appStorage.trackDurable(this.app, 'draft', () => writeDraft(this.draft!))) {
      throw new PublishError(422, 'storage', 'The image upload could not be saved on this computer. Free some space, then try again.');
    }
    this.imageCheckpoint = key;
  }
  showImageDecision() { if (this.imageDecision) this.dialog = 'replace-images'; }
  keepImages() {
    const job = this.draft?.publish_job;
    if (!job) return;
    this.savePublishJob({ ...job, decision: 'keep', pixels_key: this.pixelKey });
    this.imageOffline = false; this.imageError = ''; this.imageTimer.reset();
    this.dialog = null;
    this.refreshAutomatic();
  }
  async replaceImages() {
    const job = this.draft?.publish_job;
    if (!job || !canWrite() || this.app.readOnly) return;
    this.savePublishJob({ ...job, decision: undefined, replace_current: true, upload_id: undefined,
      start_key: crypto.randomUUID(), commit_key: crypto.randomUUID(), zip_key: crypto.randomUUID() });
    this.dialog = null;
    await this.publishImages();
  }
  async imagesPublished(projectUuid: string, valid: () => boolean = () => true, gate: () => Promise<void> = async () => {}) {
    const identity = this.identity;
    await gate();
    const api = await this.api();
    // Keep the job until the published base has been read: a failed refresh resumes/replays commit.
    const published = await api.publishedProject(projectUuid);
    if (!this.isCurrent(identity) || !valid()) return;
    if (published) this.save({ pixels_base: pixelSignature(published) });
    await gate();
    const status = await api.lookup(projectUuid);
    if (!this.isCurrent(identity) || !valid()) return;
    this.setStatus(status);
    this.savePublishJob(null);
    this.track(false);
    if (this.status) this.syncedNotice(this.status.revision, { spots: 0, grains: 0, labels: 0 }, true);
  }
  async publishImages(projectUuid = this.draft?.linked_uuid, background = false): Promise<void> {
    if (this.imageRun) return this.imageRun;
    if (this.staleOpen) { if (!background) this.showStaleOpen(); return; }
    if (background && !this.imagePending || !canWrite() || this.app.readOnly || !this.ready) return;
    if (background && !imagesMayRun(this.lastImageEdit, Date.now(), this.imageGates(false))) { this.imageTimer.ensure(); return; }
    const identity = this.identity;
    const scope = () => JSON.stringify([this.identity, this.app.project?.upbdb.mount_id, this.app.remote?.server ?? this.draft?.server ?? this.app.server, this.app.link.token, this.app.sessionToken]);
    const startedScope = scope();
    const startedEdits = this.imageEdits;
    const sameProject = () => !this.disposed && this.isCurrent(identity) && scope() === startedScope;
    this.imageBusy = true; this.imageProgress = null; this.imageError = ''; this.imageTimer.cancel();
    const gate = async (idle = true) => {
      for (;;) {
        if (!sameProject() || !canWrite() || this.app.readOnly) throw new DOMException('Image publishing stopped', 'AbortError');
        // Recheck between every request and retry, including a resumed job.
        if (this.staleOpen || background && (this.reachable !== true || this.knownUnreachable)) throw new DOMException('Image publishing paused', 'AbortError');
        if (!idle || imagesMayRun(background || this.imageEdits !== startedEdits ? this.lastImageEdit : -Infinity, Date.now(), this.imageGates(!background))) return;
        await waitFor(500);
      }
    };
    this.imageRun = (async () => {
      try {
        await performPublish(this.app, projectUuid, gate, sameProject);
        if (sameProject()) { this.imageOffline = false; this.imageTimer.reset(); }
      } catch (error) {
        if (!sameProject() || error instanceof DOMException && error.name === 'AbortError') return;
        const e = error as { code?: string; status?: number; retryAfter?: number };
        if (e.code === 'replace_confirm') this.showImageDecision();
        else {
          this.imageError = error instanceof Error ? error.message : String(error);
          // Image failures stay quiet and retry; only replacement requires an attention decision.
          this.imageOffline = error instanceof PublishError ? error.transient : !e.status || e.status >= 500 || e.status === 429;
          this.imageRetryAfter = e.retryAfter ?? 0;
          if (background && this.imageOffline) {
            this.reachable = false;
            const server = this.app.remote?.server ?? this.draft?.server ?? this.app.server;
            if (server) unreachableAt.set(server, Date.now());
          }
        }
      } finally {
        this.imageBusy = false; this.imageRun = null;
        if (sameProject()) this.refreshAutomatic();
      }
    })();
    return this.imageRun;
  }
  private idleWaiters: (() => void)[] = [];
  async settled() { flushWrites(); while (this.busy) await new Promise<void>(resolve => this.idleWaiters.push(resolve)); }
  private finished() {
    this.busy = false;
    for (const resolve of this.idleWaiters.splice(0)) resolve();
    if (this.reachable === true && !this.pendingSync) this.automatic.reset();
    this.refreshAutomatic();
  }
  private serverHashes = $state.raw<ProjectStatus['hashes'] | null>(null);
  private elementsRevision = $state<number | null>(null);
  private viewHashes = $state.raw<ProjectStatus['hashes'] | null>(null);
  private get ready() { return !!this.app.project && !this.app.loadingProject && !this.app.storedCopy; }
  private isCurrent(identity: string) { return this.ready && this.identity === identity; }
  constructor(app: AppState) { this.app = app; }
  private cachedGrains = $derived.by<Elements['grains']>(() => Object.entries(this.app.grains).flatMap(([area_id, grains]) => grains.map(g => ({ ...g, area_id, sample: g.sample !== undefined ? g.sample : this.app.project?.areas.find(a => a.id === area_id)?.sample ?? null }))));
  private cachedElements = $derived.by<Elements>(() => ({ spots: this.app.spots, labels: this.app.labels, grains: this.cachedGrains }));
  get elements(): Elements { return this.cachedElements; }
  get firstSync(): boolean {
    const server = this.serverElements ?? this.draft?.base;
    // Decided from the server's state (at open/check and after a sync), never from later local edits:
    // a split's new children are ordinary local changes, not a reason for a first snapshot.
    return !!server && this.snapshotNeeded;
  }
  get baseRevision() { return this.app.readOnly ? this.viewRevision : this.draft?.base_revision ?? null; }
  private localBase = $derived.by(() => this.firstSync ? this.emptyBase : (this.app.readOnly || this.app.draftHere != null ? this.serverElements : this.draft?.base ?? this.draft?.base_seen ?? this.serverElements) ?? this.viewBase ?? this.emptyBase);
  private emptyBase = emptyElements();
  private diffKind = elementDiffer();
  // Lineage depends only on grains and database identities, never on spot edits.
  private knownGrainKey = $derived.by(() => [...this.knownGrains()].sort().join('|'));
  private localGrains = $derived.by(() => withKnownLineage({ ...this.emptyBase, grains: this.cachedGrains }, new Set(this.knownGrainKey.split('|'))).grains);
  private spotChanges = $derived.by(() => this.diffKind('spots', this.localBase.spots, this.app.spots));
  private grainChanges = $derived.by(() => this.diffKind('grains', this.localBase.grains, this.localGrains));
  private labelChanges = $derived.by(() => this.diffKind('labels', this.localBase.labels, this.app.labels));
  private cachedLocalChanges = $derived.by(() => ({ spots: this.spotChanges, grains: this.grainChanges, labels: this.labelChanges }));
  get localChanges() { return this.cachedLocalChanges; }
  private cachedDatabaseChanges = $derived.by(() => {
    const base = this.draft?.base ?? this.viewBase;
    return base && this.serverElements ? diffElements(base, this.serverElements) : null;
  });
  get databaseChanges() { return this.cachedDatabaseChanges; }
  private cachedCounts = $derived.by(() => changeCounts(this.localChanges));
  get counts() { return this.cachedCounts; }
  get canSyncOnSave(): boolean {
    return this.ready && !this.app.readOnly && this.app.link.syncOnSave && this.registered === true && this.reachable === true && this.status?.can_edit === true;
  }
  private pixelKey = $derived.by(() => {
    if ((!this.app.pkg && !bundlePixels(this.app)) || !Object.keys(this.app.stitched).length || !this.app.project) return null;
    // Snapshot only pixel inputs. In particular, never visit grainatlas_ui geometry.
    const adopted = $state.snapshot({ grainatlas_ui: {
      tone: (this.app.project.grainatlas_ui as { tone?: unknown } | undefined)?.tone,
      bundle_tone: (this.app.project.grainatlas_ui as { bundle_tone?: unknown } | undefined)?.bundle_tone,
    } }) as unknown as import('./core/types').Project;
    const p = $state.snapshot({ areas: this.app.project.areas, grainatlas_ui: { tone: this.app.tone } }) as unknown as import('./core/types').Project;
    saveBundleTones(p, $state.snapshot(this.app.bundle), $state.snapshot(this.app.params), $state.snapshot(this.app.tone), adopted);
    if (!this.app.bundle) p.areas = p.areas.map(a => ({ ...a, params: this.app.params[a.id] ?? a.params, mosaic: this.app.stitched[a.id]?.mosaic ?? a.mosaic }));
    return pixelSignature(p);
  });
  private storedSyncStatus = $state.raw({ pixels: false, unlinked: false });
  private cachedSyncStatus = $derived.by(() => this.draft ? draftSyncStatus(this.draft, undefined, { changes: this.counts, pixelsKey: this.pixelKey, stored: this.storedSyncStatus }) : null);
  get pixelsChanged(): boolean { return this.cachedSyncStatus?.pixels ?? false; }
  get state() {
    if (this.intentFailure) return 'local-changes';
    if (!this.app.readOnly && KINDS.some(k => this.databaseMore[k].length)) return 'both-changed';
    const hashes = this.app.readOnly ? this.viewHashes : this.draft?.base_hashes;
    const serverChanged = hashes && this.serverHashes ? (['spots', 'grains', 'labels'] as const).some(k => hashes[k] !== this.serverHashes![k]) : undefined;
    return syncState({ firstSync: this.firstSync, serverChanged, registered: this.registered, reachable: this.reachable, baseRevision: this.app.readOnly ? this.viewRevision : this.draft?.base_revision ?? null, serverRevision: this.elementsRevision ?? this.status?.revision ?? null, localChanges: this.cachedSyncStatus?.elements ?? hasChanges(this.counts), pixelsChanged: this.pixelsChanged }); }
  get detailText(): string {
    if (this.staleOpen) return 'Needs your attention';
    if (this.imageDecision) return 'Choose whether to replace the current images';
    if (this.imageBusy) return this.imageProgress ? `Syncing images… ${this.imageProgress.done} of ${this.imageProgress.total} files` : 'Syncing images…';
    if (this.imageError) return this.imageError;
    if (this.confirmation) return 'Needs confirmation';
    if (this.draft?.link_conflict) return 'This project is linked to another mount';
    if (this.intentFailure) return `Local changes · ${this.intentFailure}`;
    if (!this.app.readOnly && KINDS.some(k => this.databaseMore[k].length)) return 'Database has more';
    if (this.error && this.reachable) return `Sync failed · ${this.errorHeadline}`;
    if (this.status?.can_edit === false) return 'Read-only access';
    if (this.held.length && !this.app.readOnly) return `${this.held.length} ${this.held.length === 1 ? 'item' : 'items'} held back`;
    return chipText(this.state, this.counts, this.status?.atlas_version ?? null, this.status?.revision ?? null, this.pixelsChanged, this.firstSync); // sync warnings show in the notice bar and the details drawer, not the pill
  }
  get text() {
    const text = plainSyncStatus(this.draft ? { ...this.draft, held_back: this.held } : null, { pending: !!this.cachedSyncStatus?.pending }, {
      busy: this.busy, reachable: this.reachable, error: !!this.error,
      attention: !!this.staleOpen || !!this.confirmation || !!this.review || !!this.intentFailure || !!this.held.length,
      serverRevision: this.draft?.server_revision ?? this.status?.revision, checked: this.reachable === true && !!this.status,
    });
    if (this.imageDecision || text === 'Needs your attention') return 'Needs your attention';
    if (this.imageBusy && !this.busy) return 'Syncing images…';
    if (this.imageOffline && !this.busy) return 'Offline, will sync later';
    return text;
  }
  get registeredSpotConflict() {
    return this.review?.merge.differences.some(d => d.collision && d.kind === 'spots' &&
      (!!d.base || [d.mine, d.theirs].some(s => s && 'acquisition_ids' in s && s.acquisition_ids?.length))) ?? false;
  }
  get unsynced(): boolean {
    return !!this.app.project && !this.app.readOnly && (!!this.intentFailure || KINDS.some(k => this.databaseMore[k].length) || !this.draft?.synced || !this.draft?.base || !!this.cachedSyncStatus?.pending);
  }
  private get identity(): string { return JSON.stringify([this.app.project?.uuid, this.app.projectPath?.path ?? null, this.app.remote?.projectId ?? null]); }
  private async api() {
    const server = this.app.remote?.server ?? this.draft?.server ?? this.app.server;
    if (!server) throw new Error('Set the database server in Settings, then Retry.');
    return new SyncApi(server, await authToken(this.app), this.app.remote?.access ?? null);
  }
  private save(patch: Partial<Draft>) {
    if (!this.draft || this.app.loadingProject || this.app.storedCopy) return;
    // Pending is assembled from immutable per-kind collections. Keep its reference;
    // other patches (server responses and request metadata) retain copy isolation.
    const copied = clone('pending' in patch ? { ...patch, pending: null } : patch);
    if ('pending' in patch) copied.pending = patch.pending;
    this.draft = { ...this.draft, ...copied };
    if ('pixels_base' in patch || 'mount' in patch || 'link_candidates' in patch) {
      const status = draftSyncStatus(this.draft, undefined, { changes: this.counts, pixelsKey: this.pixelKey });
      this.storedSyncStatus = { pixels: status.pixels, unlinked: status.unlinked };
    }
    if (canWrite() && !this.suppressDraft) {
      const draft = this.draft;
      deferredWrites.enqueue(this, () => {
        if (!this.suppressDraft && this.draft === draft) appStorage.track(this.app, 'draft', () => writeDraft(draft));
      });
    }
  }
  private pendingSpots = $derived.by(() => pendingElements({ ...this.emptyBase, spots: this.app.spots }).spots);
  private pendingGrains = $derived.by(() => pendingElements({ ...this.emptyBase, grains: this.cachedGrains }).grains);
  private pendingLabels = $derived.by(() => pendingElements({ ...this.emptyBase, labels: this.app.labels }).labels);
  private pendingElements = $derived.by(() => ({ spots: this.pendingSpots, grains: this.pendingGrains, labels: this.pendingLabels }));
  track(edited = true) {
    const p = this.app.project;
    if (!p || this.app.readOnly || this.app.loadingProject || this.app.storedCopy) return;
    if (!this.draft || this.draft.uuid !== p.uuid) this.draft = ensureDraft(p, this.app.server);
    if (edited) this.editedSinceOpen = true;
    const key = this.app.projectPath ? `grainatlas.copy.${p.uuid}|${this.app.projectPath.path}` : `grainatlas.project.${p.package.name}`;
    const changes = this.draft.base ? this.counts : null;
    const pending = !!this.intentFailure || !this.draft.base || hasChanges(changes) ? this.pendingElements : null;
    const now = new Date().toISOString();
    // Storage readers flush queued writes. Resolve legacy copies here, outside
    // rendered getters, to keep status reads from serialising a project per edit.
    const status = draftSyncStatus(this.draft, undefined, { changes: this.counts, pixelsKey: this.pixelKey });
    this.storedSyncStatus = { pixels: status.pixels, unlinked: status.unlinked };
    this.save({ changes, pending, pending_at: pending ? now : null, ...(this.pixelKey !== null ? { pixels_key: this.pixelKey } : {}), pixels_changed: status.pixels, updated_at: edited ? now : this.draft.updated_at, source_keys: [...new Set([...(this.draft.source_keys ?? []), key])] });
    if (edited) this.imagesEdited();
    if (edited && this.backgroundPending && !this.busy) this.automatic.edit();
  }
  /** Every opening path calls this before returning. Later polling never replaces the base. */
  async check(): Promise<void> {
    flushWrites();
    if (this.busy) { await this.settled(); return this.check(); }
    const p = this.app.project;
    const identity = this.identity;
    if (!p || this.busy || this.app.loadingProject || this.app.storedCopy) return;
    if (p.uuid !== this.uuid) {
      this.automatic.reset(); this.imageTimer.reset(); this.imageError = ''; this.imageOffline = false; this.pendingSync = false;
      this.uuid = p.uuid; this.suppressDraft = false; this.confirmation = null; this.heldSync = null;
      const previous = listDrafts(this.app.remote?.server ?? this.app.server).find(d => d.uuid === p.uuid) ?? readDraft(p.uuid);
      const fresh = !previous;
      this.draft = previous ?? ensureDraft(p, this.app.remote?.server ?? this.app.server);
      const stored = draftSyncStatus(this.draft);
      this.storedSyncStatus = { pixels: stored.pixels, unlinked: stored.unlinked };
      if (fresh && !this.app.readOnly) this.save({ base_seen: clone(this.elements) });
      appStorage.track(this.app, 'draft', () => writeDraft(this.draft!)); // ready open adopts even when the subsequent lookup is offline
      this.registered = undefined; this.status = null; this.reachable = null; this.review = null; this.dialog = null; this.syncedAt = this.draft.synced_at ?? null; this.viewRevision = null; this.viewBase = null; this.viewHashes = null; this.serverHashes = null; this.elementsRevision = null; this.serverSpots = []; this.serverElements = null; this.snapshotNeeded = false; this.notice = ''; this.intentFailure = this.draft.incomplete?.reason ?? ''; this.omissions = this.draft.incomplete?.omissions ?? { spots: [], grains: [], labels: [] }; this.candidates = this.draft.link_candidates ?? []; this.heldBack = this.draft.held_back ?? [];
    }
    // A view may enter Edit without opening the file again. Only inspect it
    // before the first local edit or explicit choice in this session.
    if (!this.staleOpen && !this.app.readOnly) this.staleOpen = staleOpenElements(this.draft?.base, this.elements, this.editedSinceOpen);
    this.checkedElements = null;
    this.track(false);
    this.busy = true; this.checking = true; this.deletionCheckDeadline = Date.now() + 8000;
    try {
      const api = await this.api();
      const status = await api.lookup(this.draft?.linked_uuid ?? p.uuid);
      if (!this.isCurrent(identity)) return;
      if (!status) this.setStatus(null);
      this.registered = !!status; this.reachable = true; unreachableAt.delete(this.app.remote?.server ?? this.draft?.server ?? this.app.server ?? '');
      if (this.app.error === this.errorHeadline) this.app.error = null;
      if (this.draft?.incomplete) { this.failureMessage = this.draft.incomplete.reason; this.validationFailed = true; }
      if (!this.intentFailure) { this.error = ''; this.failureMessage = ''; this.validationFailed = false; }
      if (!status) {
        if (!this.draft?.link_offered) {
          const candidates = await api.candidates(p.package.name, p.name);
          if (this.identity !== identity || this.app.loadingProject || this.app.storedCopy) return;
          this.save({ link_offered: true, link_candidates: candidates });
          this.candidates = candidates;
        }
        this.track(false);
        return;
      }
      if (this.app.project) this.app.project.upbdb.mount_id = status.mount_id;
      this.save({ server: this.app.remote?.server ?? this.app.server ?? this.draft?.server, mount: status.mount_id, checked_at: new Date().toISOString(), server_revision: status.revision, server_can_edit: status.can_edit, link_conflict: false, synced: this.draft?.synced ?? false });
      const theirs = await api.elements(status.project_uuid);
      if (!this.isCurrent(identity)) return;
      this.checkedElements = { identity, projectUuid: status.project_uuid, fetchedAt: Date.now(), elements: theirs };
      if (!this.serverElements) this.snapshotNeeded = needsFirstSnapshot(theirs, this.elements);
      this.serverElements = theirs; this.serverSpots = theirs.spots; this.serverHashes = theirs.hashes; this.elementsRevision = theirs.revision;
      this.setStatus({ ...status, revision: theirs.revision });
      if (this.app.readOnly && this.viewRevision === null) {
        const pending = this.draft ? draftElements(this.draft) : null;
        this.displayElements(pending ?? viewElements(this.elements, theirs));
        this.app.draftHere = pending ? this.draft?.base_revision ?? 'unknown' : null;
        this.viewRevision = theirs.revision; this.viewBase = theirs; this.viewHashes = theirs.hashes;
      }
      if (!this.draft?.base) {
        // Lookup is not an acknowledgement: never turn unseen server rows into deletions.
        this.save({ base_revision: this.draft?.base_revision ?? theirs.revision });
        if (!this.app.readOnly && this.draft?.base_seen) {
          const shown = Object.fromEntries(KINDS.map(k => [k, theirs[k].filter(e => e.id === null || this.draft!.base_seen![k].some(local => local.id === e.id))])) as unknown as ReadElements;
          this.save({ base: shown, base_hashes: theirs.hashes });
        }
      }
      if (this.draft?.pixels_base === undefined) {
        const published = await api.publishedProject(status.project_uuid);
        if (!this.isCurrent(identity)) return;
        this.save({ pixels_base: published ? pixelSignature(published) : null });
      }
      this.track(false);
    } catch (e) {
      this.reachable = e instanceof SyncApiError && e.status < 500;
      const failedServer = this.app.remote?.server ?? this.draft?.server ?? this.app.server;
      if (failedServer && !this.reachable) unreachableAt.set(failedServer, Date.now());
      this.error = e instanceof Error ? e.message : String(e);
      this.errorDetails = e instanceof SyncApiError ? e.raw : '';
    } finally { this.finished(); this.checking = false; if (this.identity !== identity) void this.check(); }
    this.refreshAutomatic();
    if (!this.staleOpen && !this.automaticEnabled && this.pendingSync && this.reachable && this.state === 'local-changes') { this.pendingSync = false; await this.runQuietSync(); }
  }
  dismissLink() { this.candidates = []; this.save({ link_offered: true, link_candidates: [] }); }
  showLink(): string {
    const mount = this.candidates.length === 1 ? ` to mount ${this.candidates[0].mount_label}` : '';
    this.error = `${this.app.project?.name || this.draft?.name || 'This project'} is not linked to a database mount yet. Link it${mount} in Images to sync.`;
    if (!this.syncDisabledReason) this.dialog = 'register';
    return this.error;
  }
  async link(candidate: Candidate) {
    if (!canWrite()) return;
    if (this.busy || !this.app.project || this.app.readOnly || !candidate.can_edit) return;
    const identity = this.identity;
    this.busy = true;
    try {
      // Link this project's UUID to the chosen mount. Candidate UUIDs are hints only.
      await (await this.api()).link(this.app.project.uuid, candidate.mount_id);
      if (!this.isCurrent(identity)) return;
      await (await this.app.core).setLink(candidate.mount_id, {});
      if (!this.isCurrent(identity)) return;
      this.app.project.upbdb.mount_id = candidate.mount_id;
      this.save({ mount: candidate.mount_id, link_offered: true, link_candidates: [] });
      this.candidates = []; this.dialog = null;
    } catch (e) {
      if (this.isCurrent(identity)) {
        if (e instanceof SyncApiError && e.status === 409) { this.dismissLink(); this.save({ link_conflict: true }); this.dialog = null; }
        this.failed(e);
      }
      return;
    } finally { this.finished(); }
    await this.check();
  }
  viewed(revision: number | null) {
    this.viewRevision = revision;
    this.viewHashes = this.elementsRevision === revision ? this.serverHashes : null;
    if (this.elementsRevision === revision && this.serverElements) this.viewBase = this.serverElements;
    else if (this.draft?.base_revision === revision && this.draft.base) this.viewBase = this.draft.base;
  }
  openedView() { this.checkedElements = null; this.viewRevision = null; this.viewBase = null; this.viewHashes = null; this.serverElements = null; }
  forgetDraft() {
    this.confirmation = null; this.heldSync = null; this.dialog = null;
    this.suppressDraft = true;
    if (this.draft) this.draft = { ...this.draft, pending: null, pending_at: null, changes: { spots: 0, grains: 0, labels: 0 }, pixels_changed: false };
  }
  async editBase() {
    flushWrites();
    if (!canWrite()) return;
    this.suppressDraft = false;
    const revision = this.viewRevision, hashes = this.viewHashes, viewed = this.viewBase;
    const hadDraft = !!this.draft && (this.draft.pending != null || (this.draft.pending === undefined && this.draft.changes === null) || draftSyncStatus(this.draft).pending);
    await this.check();
    // A clean View opened at a newer revision becomes the next edit base. Keep a real
    // pending/legacy draft's older base; edits made during lookup remain above viewed.
    if (!hadDraft && viewed) {
      this.save({ base_seen: viewed });
      if (revision !== null) this.save({ base: viewed, base_revision: revision, base_hashes: hashes });
    }
    this.track();
  }
  /** `changed` = elements this action wrote (bridge v4); pixels = a new bundle was published in it. */
  private syncedNotice(revision: number, counts = this.counts, pixels = false) {
    const project = this.app.project?.uuid, mount = this.status?.mount_id ?? this.draft?.mount;
    if (project && mount) hostedSync.synced({ project, mount, revision, atlasVersion: this.status?.atlas_version ?? null, changed: { ...counts, pixels }, at: new Date().toISOString() });
  }
  async action() {
    flushWrites();
    if (!canWrite()) return;
    if (this.busy || this.syncDisabledReason) return;
    if (this.staleOpen) { this.showStaleOpen(); return; }
    if (this.imageDecision) { this.showImageDecision(); return; }
    if (this.confirmation) { this.showConfirmation(); return; }
    if (this.state === 'offline' || this.state === 'checking') { await this.check(); return; }
    if (this.state === 'unregistered') { this.dialog = 'register'; return; }
    if (this.state === 'server-newer') { await this.update(); return; }
    if (this.state === 'both-changed' && !KINDS.some(k => this.databaseMore[k].length)) { await this.prepareReview(); return; }
    if (this.pixelsChanged) { this.dialog = 'pixels'; return; }
    await this.sync();
  }
  async register(mount: string, publishPixels = false) {
    flushWrites();
    if (!canWrite()) return;
    if (!mount || this.syncDisabledReason) return;
    try { await (await this.app.core).setLink(mount, {}); } catch (e) { this.failed(e); return; }
    this.save({ mount });
    if (this.app.project) this.app.project.upbdb.mount_id = mount;
    this.dialog = null;
    await this.sync(publishPixels);
  }
  private displayElements(elements: ReadElements) {
    this.app.spots = uiSpots(clone(elements.spots), elements.grains, this.app.spots, Object.fromEntries(this.app.project!.areas.map(a => [a.id, a.sample]))); this.app.grains = groupedGrains(clone(elements.grains), this.app.grains); this.app.labels = clone(elements.labels.filter(l => !l.retired));
  }
  private setElements(elements: ReadElements) {
    this.app.checkpoint(); this.displayElements(elements);
    this.app.selection = []; this.app.persist();
  }
  async update() {
    flushWrites();
    if (this.busy || !this.status) return;
    // Recheck local edits at invocation, so a newly made edit is never overwritten.
    if (hasChanges(this.counts)) { await this.prepareReview(); return; }
    this.busy = true;
    const identity = this.identity;
    try {
      const theirs = await (await this.api()).elements(this.status.project_uuid);
      if (!this.isCurrent(identity)) return;
      if (hasChanges(this.counts)) { this.openReview(theirs, theirs.revision, theirs.hashes); return; }
      const changed = changeCounts(diffElements(this.syncElements(this.elements, theirs), theirs));
      this.serverElements = theirs; this.serverSpots = theirs.spots;
      this.setElements(this.mergeLocal(this.draft?.base ?? emptyElements(), clone(this.elements), theirs).combined);
      if (!this.app.readOnly || !this.draft?.source_keys?.length) this.save({ base: { spots: theirs.spots, grains: theirs.grains, labels: theirs.labels }, base_revision: theirs.revision, base_hashes: theirs.hashes });
      this.serverHashes = theirs.hashes; this.elementsRevision = theirs.revision;
      this.setStatus(await (await this.api()).lookup(this.status.project_uuid));
      this.viewRevision = theirs.revision; this.viewBase = theirs; this.viewHashes = theirs.hashes;
      this.track(false); this.reachable = true; this.syncedNotice(theirs.revision, changed);
    } catch (e) { this.failed(e); } finally { this.finished(); if (this.identity !== identity) void this.check(); }
  }
  private openReview(theirs: ReadElements, revision: number, hashes?: ProjectStatus['hashes'], base = this.draft?.base ?? emptyElements(), conflicts: { kind: Kind; id: string }[] = []) {
    this.confirmation = null; this.heldSync = null;
    const local = clone(this.elements);
    const merge = this.mergeLocal(base, local, theirs);
    // Ordinary review equality has different rounding/default rules. Every field
    // conflict found before a first snapshot must still require a choice here.
    for (const { kind, id } of conflicts) {
      const difference = merge.differences.find(d => d.kind === kind && d.id === id);
      if (difference) difference.collision = true;
      else merge.differences.push({ kind, id, base: storedElements(base)[kind].find(e => e.id === id), mine: local[kind].find(e => e.id === id),
        theirs: storedElements(theirs)[kind].find(e => e.id === id), collision: true, bothAllowed: kind === 'labels' });
    }
    if (this.automaticRunning && !merge.differences.some(d => d.collision)) {
      this.setElements(merge.combined);
      this.save({ base: clone(theirs), base_revision: revision, base_hashes: hashes ?? null });
      this.track(false);
      return;
    }
    for (const d of merge.differences) if (d.kind === 'spots' && d.base) d.bothAllowed = false;
    this.review = { merge, theirs: clone(theirs), revision, hashes };
    this.dialog = 'review';
  }
  async prepareReview() {
    flushWrites();
    if (this.busy || !this.status) return;
    this.busy = true;
    const identity = this.identity;
    try {
      const api = await this.api();
      if (this.draft?.base && this.draft.base_revision !== null) {
        const history = await api.changes(this.status.mount_id, this.draft.base_revision);
        if (this.isCurrent(identity)) this.openReview(applyChanges(this.draft.base, changesFromEvents(history.changes, this.status.project_uuid)), history.revision);
      } else {
        const theirs = await api.elements(this.status.project_uuid);
        if (this.isCurrent(identity)) this.openReview(theirs, theirs.revision, theirs.hashes);
      }
    }
    catch (e) { this.failed(e); } finally { this.finished(); if (this.identity !== identity) void this.check(); }
  }
  async apply(choices: Record<string, Resolution>, publishPixels = false) {
    flushWrites();
    if (!canWrite()) return;
    const review = this.review;
    if (!review) return;
    const result = resolveMerge(review.merge, choices);
    this.setElements(result);
    this.save({ base: review.theirs, base_revision: review.revision, base_hashes: review.hashes ?? null });
    this.dialog = null; this.review = null;
    await this.sync(publishPixels, false, undefined, review.theirs);
  }
  get errorHeadline(): string { const parts = this.error.split(' · '); return lineageFailureMessage(parts) ?? (parts.length > 1 ? problemSummary(parts) : this.error); }
  private knownGrains(server: ReadElements | null = this.serverElements, base: ReadElements | null = this.draft?.base ?? null): Set<string> {
    // Include retired rows: the server still decides whether their lineage is valid.
    return new Set([...(base?.grains ?? []), ...(this.draft?.base?.grains ?? []), ...(server?.grains ?? []), ...(this.viewBase?.grains ?? []), ...this.lastSeen.grains].flatMap(g => g.id === null ? [] : [g.id]));
  }
  private syncElements(elements: Elements, server: ReadElements | null = this.serverElements): Elements {
    return withKnownLineage(elements, this.knownGrains(server));
  }
  private mergeLocal(base: ReadElements, local: Elements, server: ReadElements): Merge {
    const known = this.knownGrains(server, base);
    const merge = mergeElements(base, withKnownLineage(local, known), server);
    // Independent fields on the same row are safe to combine too. Deletion and
    // same-field conflicts still require the existing explicit resolution.
    const fields = firstSyncMerge(base, local, server, this.draft?.linked_uuid ?? this.app.project?.uuid ?? '');
    for (const d of merge.differences) {
      if (!d.collision || !d.base || !d.mine || !d.theirs || fields.conflicts.some(c => c.kind === d.kind && c.id === d.id)) continue;
      const combined = fields.elements[d.kind].find(e => e.id === d.id);
      if (combined) {
        (merge.combined[d.kind] as import('./sync').Element[]) = merge.combined[d.kind].map(e => e.id === d.id ? combined : e);
        d.collision = false;
      }
    }
    // Compare database fields, then carry local-only history back into the project.
    const originals = new Map(local.grains.map(g => [g.id, g]));
    const restore = (grain: Elements['grains'][number]) => {
      const original = originals.get(grain.id);
      if (original?.split_from && !known.has(original.split_from)) grain.split_from = original.split_from;
      const unknown = original?.merged_from?.filter(id => !known.has(id)) ?? [];
      if (unknown.length) grain.merged_from = [...new Set([...(grain.merged_from ?? []), ...unknown])];
    };
    merge.combined.grains.forEach(restore);
    for (const difference of merge.differences) if (difference.kind === 'grains' && difference.mine) restore(difference.mine as Elements['grains'][number]);
    return merge;
  }
  private failed(e: unknown) {
    this.confirmation = null; this.heldSync = null;
    if (this.dialog === 'bulk') this.dialog = null;
    this.error = e instanceof Error ? e.message : String(e);
    this.errorDetails = e instanceof SyncApiError ? e.raw || e.messages.join('\n') : '';
    this.failureMessage = e instanceof SyncApiError ? lineageFailureMessage(e.messages) ?? e.messages[0] : this.error;
    this.validationFailed = e instanceof SyncApiError && e.status === 422;
    if (!this.automaticRunning) this.app.error = this.errorHeadline;
    this.reachable = e instanceof SyncApiError ? !e.transient : false;
  }
  /** Capture the submitted snapshot: edits made while the request is in flight stay dirty. */
  async sync(publishPixels = false, shellCommand = false, approved?: HeldSync, reviewedBase?: ReadElements) {
    flushWrites();
    if (!canWrite()) return;
    if (this.busy) await this.settled();
    if (this.busy || !this.ready || !this.app.project || this.app.readOnly || this.syncDisabledReason) return;
    if (this.staleOpen) { if (!this.automaticRunning && !shellCommand) this.showStaleOpen(); return; }
    if (this.holdAutomaticDeletes(this.draft?.pending_request?.body?.spots.delete ?? this.localChanges.spots.delete)) return;
    if (!this.draft?.mount) { this.showLink(); return; }
    // Old drafts or undo can bypass the canvas guard. Restore protected spots
    // locally too, rather than acknowledging a silent hide as a deletion.
    const missing = [...this.serverSpots, ...(this.draft.base?.spots ?? [])].filter(s => s.acquisition_ids?.length && !this.app.spots.some(local => local.id === s.id));
    if (missing.length) {
      const unique = [...new Map(missing.map(s => [s.id, s])).values()];
      this.app.spots = uiSpots([...this.app.spots, ...unique], this.elements.grains, this.app.spots);
      this.app.notice = 'This spot has measurements in the database; it cannot be deleted there';
    }
    this.automatic.cancel();
    this.track(false);
    this.busy = true;
    const identity = this.identity;
    const draft = this.draft;
    const projectUuid = draft.linked_uuid ?? this.app.project.uuid;
    const checked = this.checkedElements;
    this.checkedElements = null; // A check result can only seed the next sync, never a retry or approval.
    const recovery = !approved && draft.pending_request?.body && draft.pending_request.mine && draft.pending_request.current ? draft.pending_request : null;
    let mine = recovery?.mine ?? approved?.mine ?? clone(this.elements);
    let retryMissing: Record<Kind, string[]> | null = null;
    try {
      const api = await this.api();
      if (!this.isCurrent(identity)) return;
      for (let attempt = 0; attempt < 2; attempt++) {
        try {
          const reuseChecked = !approved && attempt === 0 && checked?.identity === identity && checked.projectUuid === projectUuid && Date.now() - checked.fetchedAt < 2000;
          const current = recovery?.current ?? (this.registered ? (reuseChecked ? checked.elements : await api.elements(projectUuid)) : this.serverElements ?? draft.base ?? emptyElements());
          if (!this.isCurrent(identity)) return;
          this.serverElements = current;
          const additions = databaseAdditions(current, this.lastSeen, mine, projectUuid);
          if (!recovery && KINDS.some(k => additions[k].length)) {
            this.keepDatabase(additions); mine = keepAdditions(mine, additions);
            if (approved) { this.confirmation = null; this.heldSync = null; approved = undefined; }
          }
          const mode = approved?.body.mode ?? (this.firstSync || !draft.synced ? 'snapshot' : 'delta');
          // check() can seed base before any sync is acknowledged. That database
          // context is not the local baseline: use what was shown at first open.
          if (!recovery && !approved && mode === 'snapshot' && (!draft.synced || !draft.base || !KINDS.some(k => draft.base![k].length))
            && KINDS.some(k => projectElements(current, projectUuid)[k].length)) {
            const base = reviewedBase ?? draft.base_seen ?? mine;
            const merged = firstSyncMerge(base, mine, current, projectUuid);
            if (merged.conflicts.length) {
              this.openReview(current, 'revision' in current ? Number(current.revision) : draft.base_revision ?? 0, 'hashes' in current ? current.hashes as ProjectStatus['hashes'] : undefined, base, merged.conflicts);
              return;
            }
            if (KINDS.some(k => JSON.stringify(merged.elements[k]) !== JSON.stringify(mine[k]))) {
              // Preserve edits made during the fetch, just as after a sync reply.
              this.setElements(this.mergeLocal(mine, clone(this.elements), merged.elements).combined);
              mine = merged.elements;
              this.track(false);
            }
          }
          // Use the existing three-way merge for newer remote rows. Registered
          // spot conflicts always need a person; other true collisions retain review.
          if (!recovery && !approved && mode === 'delta' && draft.base && 'revision' in current && Number(current.revision) > (draft.base_revision ?? 0)) {
            const merge = this.mergeLocal(draft.base, mine, current);
            if (merge.differences.some(d => d.collision)) {
              this.openReview(current, Number(current.revision), 'hashes' in current ? current.hashes as ProjectStatus['hashes'] : undefined);
              return;
            }
            this.setElements(this.mergeLocal(mine, clone(this.elements), merge.combined).combined);
            mine = merge.combined;
          }
          const syncMine = this.syncElements(mine, current);
          const diff = approved?.body ?? syncChanges(mode, draft.base ?? emptyElements(), syncMine, current, projectUuid, this.lastSeen);
          if (retryMissing) for (const k of KINDS) {
            const removed = retryMissing[k].filter(id => this.lastSeen[k].some(e => e.id === id) && !mine[k].some(e => e.id === id) && (k !== 'spots' || !current.spots.some(s => s.id === id && s.acquisition_ids?.length)));
            if (k === 'grains') diff.grains.retire = [...new Set([...diff.grains.retire, ...removed])];
            else diff[k].delete = [...new Set([...diff[k].delete, ...removed])];
          }
          try { if (!recovery) validateLineage(draft.base ?? emptyElements(), syncMine, diff); }
          catch (e) { throw new SyncApiError(422, [e instanceof Error ? e.message : String(e)]); }
          const sinceSeen = diffElements(draft.base ?? draft.base_seen ?? emptyElements(), current);
          const onlyAdditions = KINDS.every(k => !sinceSeen[k].delete.length && (k !== 'grains' || !sinceSeen.grains.retire.length) && sinceSeen[k].upsert.every(e => !(draft.base ?? draft.base_seen ?? emptyElements())[k].some(s => s.id === e.id)));
          const body: SyncBody = recovery?.body ?? approved?.body ?? { project_uuid: projectUuid, base_revision: 'revision' in current && (mode === 'delta' && !approved || onlyAdditions && KINDS.some(k => sinceSeen[k].upsert.length)) ? Number(current.revision) : draft.base_revision ?? 0, mode, ...diff,
            spots: { ...diff.spots, upsert: diff.spots.upsert.map(s => mode === 'snapshot' ? { ...s, sample: mine.spots.find(local => local.id === s.id)?.sample } : s) } };
          if (this.holdAutomaticDeletes(body.spots.delete)) return;
          const parents = new Set([...body.grains.retire, ...body.grains.delete, ...body.grains.upsert.flatMap(g => [...(g.split_from ? [g.split_from] : []), ...(g.merged_from ?? [])])]);
          const counts: AtlasSyncConfirmation = {
            retireGrains: current.grains.filter(g => !g.retired && g.id && parents.has(g.id)).length,
            grainsTotal: this.status?.counts.grains ?? current.grains.filter(g => !g.retired).length,
            deleteSpots: current.spots.filter(s => body.spots.delete.includes(s.id)).length,
            spotsTotal: this.status?.counts.spots ?? current.spots.length,
          };
          let bulk = counts.retireGrains > Math.max(3, counts.grainsTotal * .1) || counts.deleteSpots > Math.max(3, counts.spotsTotal * .1);
          if (!recovery && (bulk || approved)) {
            let preview;
            try { preview = await api.sync(draft.mount!, { ...body, dry_run: true }); }
            catch (e) {
              if (!(e instanceof SyncApiError) || !e.bulkConfirmation) throw e;
              bulk = true; Object.assign(counts, e.confirmation);
            }
            if (!this.isCurrent(identity)) return;
            if (preview && !preview.ok) { this.openReview(applyChanges(draft.base ?? emptyElements(), changesFromEvents(preview.changes, projectUuid)), preview.current_revision); return; }
          }
          const hold = () => {
            this.heldSync = { identity, mine: syncMine, current, body, publishPixels, counts }; this.confirmation = counts;
            this.dialog = shellCommand ? null : 'bulk'; this.pendingSync = false;
            this.error = ''; this.failureMessage = ''; this.validationFailed = false;
          };
          if (!recovery && bulk && (!approved || JSON.stringify(approved.counts) !== JSON.stringify(counts))) {
            hold(); return;
          }
          this.confirmation = null; this.heldSync = null;
          let reply;
          const sent: SyncBody = recovery?.body ?? { ...body, ...(approved ? { confirm_bulk: true } : {}) };
          // One id per attempt; a retry of the same request after a lost reply reuses it (contract 2026-11, section 3).
          let request: import('./sync-api').PendingRequest = { ...requestIdFor(draft.pending_request, canonicalJson(syncWireBody(sent))), body: sent, mine: syncMine, current };
          const persistRequest = async () => {
            if (!this.isCurrent(identity)) throw new SyncApiError(409, ['The open project changed. Your previous changes are kept.']);
            this.save({ pending_request: request }); flushWrites();
            if (!this.draft || !await appStorage.trackDurable(this.app, 'draft', () => writeDraft(this.draft!))) throw new SyncApiError(422, ['Your changes could not be saved on this computer. Free some space, then try again.']);
          };
          await persistRequest();
          if (!this.isCurrent(identity)) return;
          const send = () => api.sync(draft.mount!, sent, request.id, async id => {
            request = { ...request, wire_id: request.wire_id ?? request.id, id };
            await persistRequest();
          }, request.wire_id ?? request.id);
          try {
            try { reply = await send(); }
            catch (e) {
              if (!(e instanceof SyncApiError) || e.code !== 'request_id_reused') throw e;
              console.warn('Legacy sync request id was already used; retrying once with a fresh id.');
              request = { ...request, ...requestIdFor(null, request.key), wire_id: undefined };
              await persistRequest();
              reply = await send();
            }
          } catch (e) {
            if (this.isCurrent(identity) && !(e instanceof SyncNetworkError) && !(e instanceof SyncApiError && e.transient)) this.save({ pending_request: null });
            if (!(e instanceof SyncApiError) || !e.bulkConfirmation) throw e;
            if (this.isCurrent(identity)) { Object.assign(counts, e.confirmation); hold(); }
            return;
          }
          if (!this.isCurrent(identity)) {
            // A different project opened during the POST. Never put the old response in its tab.
            // Leave the old draft conservative for a fresh check; its persisted content is intact.
            return;
          }
          this.reachable = true;
          if (!reply.ok) {
            this.save({ pending_request: null });
            const theirs = applyChanges(draft.base ?? emptyElements(), changesFromEvents(reply.changes, projectUuid));
            this.openReview(theirs, reply.current_revision);
            return;
          }
          this.omissions = reply.not_in_snapshot;
          // A partial reply applied everything except the held items: acknowledge only that part.
          const held = reply.held_back ?? [];
          const part = held.length ? appliedPart(body, syncMine, current, held, projectUuid) : { changes: body, mine: syncMine };
          const appliedMine = part.mine;
          let reason = syncIntentFailure(part.changes, appliedMine, current, projectUuid, reply);
          if (reason) { this.intentFailure = reason; throw new SyncApiError(422, [reason]); }
          // Refetch before acknowledging anything: a successful HTTP response may
          // have retained omissions or ignored submitted changes.
          const accepted = await api.elements(projectUuid);
          if (!this.isCurrent(identity)) return;
          const ownOmissions = KINDS.flatMap(k => reply.not_in_snapshot[k].filter(id => [...current[k], ...accepted[k]].some(e => e.id === id && 'project_uuid' in e && e.project_uuid === projectUuid && e.id !== null)).map(id => `${k.slice(0, -1)} ${elementLabel(k, id, mine, current, accepted)}`));
          if (reply.complete === false && ownOmissions.length) {
            this.serverElements = accepted;
            this.keepDatabase(databaseAdditions(accepted, this.lastSeen, this.elements, projectUuid));
            const incomplete = `The database did not sync this project's copy; left out ${nameList(ownOmissions)}`;
            this.intentFailure = incomplete;
            this.save({ incomplete: { reason: incomplete, omissions: reply.not_in_snapshot } });
            throw new SyncApiError(422, [incomplete]);
          }
          reason = syncIntentFailure(part.changes, appliedMine, current, projectUuid, reply, accepted);
          if (reason) { this.intentFailure = reason; throw new SyncApiError(422, [reason]); }
          this.intentFailure = '';
          this.serverElements = accepted; this.serverSpots = accepted.spots; this.serverHashes = accepted.hashes; this.elementsRevision = accepted.revision;
          // Set the response revision before saving or notifying reactive consumers.
          if (!this.status) this.setStatus(await api.lookup(projectUuid));
          if (!this.isCurrent(identity)) return;
          if (this.status) this.setStatus({ ...this.status, revision: reply.revision });
          this.registered = true;
          if (this.app.error === this.errorHeadline) this.app.error = null;
          this.error = ''; this.failureMessage = ''; this.validationFailed = false;
          this.heldBack = held;
          this.save({ pending_request: null, checked_at: new Date().toISOString(), server_revision: accepted.revision, incomplete: null, held_back: held, base: appliedMine, base_revision: reply.revision, base_hashes: null, synced: true, synced_at: new Date().toISOString() });
          if (accepted.revision === reply.revision) {
            // Edits made in flight remain above the submitted snapshot.
            const local = clone(this.elements);
            const acknowledged = this.mergeLocal(appliedMine, local, accepted).combined;
            for (const grain of acknowledged.grains) {
              const previous = local.grains.find(g => g.id === grain.id);
              if (previous) grain.sample = previous.sample;
            }
            this.displayElements(acknowledged);
            this.snapshotNeeded = needsFirstSnapshot(accepted, appliedMine); // held new items are not a reason for a snapshot
            this.save({ base: accepted, base_hashes: accepted.hashes, database_added: Object.fromEntries(KINDS.map(k => [k, this.databaseAdded[k].filter(e => accepted[k].some(a => a.id === e.id && !('retired' in a && a.retired)))])) as unknown as Elements });
          }
          const wrote = appliedCounts(reply.applied);
          this.syncedNotice(reply.revision, wrote);
          if (KINDS.some(k => wrote[k] > 0)) notifyUpbLab(draft.mount!, projectUuid, reply.revision, this.app.remote?.server ?? draft.server ?? this.app.server);
          this.track(false);
          if (this.app.draftHere != null) this.app.draftHere = this.unsynced ? reply.revision : null;
          const notices = reply.warnings.filter(warning => {
            if (/kept.*null|null.*kept/i.test(warning)) { console.warn('Sync sent null fields that the database kept:', warning); return false; }
            return true;
          });
          for (const kind of ['spots', 'grains', 'labels'] as const) {
            if (reply.not_in_snapshot[kind].length) notices.push(`${reply.not_in_snapshot[kind].length} database ${reply.not_in_snapshot[kind].length === 1 ? kind.slice(0, -1) : kind} kept by the database (not part of this project's copy): ${nameList(reply.not_in_snapshot[kind].map(id => elementLabel(kind, id, accepted, current)))}`);
          }
          if (reply.rebased_over) notices.push(`Rebased over ${reply.rebased_over} mount revisions`);
          if (held.length) notices.unshift(`${held.length} ${held.length === 1 ? 'item was' : 'items were'} held back: ${nameList(held.map(h => h.label || h.id.slice(0, 8)))}. Open the sync details to fix them, then sync again.`);
          this.notice = notices.join(' · ');
          if (this.notice) this.app.notice = this.notice;
          this.pendingSync = false;
          this.registered = true;
          const status = await api.lookup(projectUuid);
          if (!this.isCurrent(identity)) return;
          this.setStatus(status);
          this.syncedAt = this.draft?.synced_at ?? null;
          if (this.app.error === this.errorHeadline) this.app.error = null;
          this.error = ''; this.failureMessage = ''; this.validationFailed = false;
          this.track();
          if (this.app.draftHere != null && !this.unsynced) this.app.draftHere = null;
          if (publishPixels && (this.app.pkg || bundlePixels(this.app))) {
            await publish(this.app);
          }
          break;
        } catch (e) {
          if (!(e instanceof SyncApiError) || e.code !== 'snapshot_incomplete' || attempt === 1) throw e;
          retryMissing = e.missing;
          const latest = await api.elements(projectUuid);
          if (!this.isCurrent(identity)) return;
          this.serverElements = latest;
          // Rebuild from the same recorded intent. New rows are pulled; seen removals
          // become delete lists. The next pass still applies the bulk guard.
          const additions = databaseAdditions(latest, this.lastSeen, mine, projectUuid);
          this.keepDatabase(additions);
          mine = keepAdditions(mine, additions);
          approved = undefined;
        }
      }
    } catch (e) { if (this.identity === identity) { this.pendingSync = !(e instanceof SyncApiError) || e.transient; this.failed(e); this.track(false); } } finally { this.finished(); if (this.identity !== identity) void this.check(); }
  }
  async afterSave() {
    flushWrites();
    if (!canWrite()) return;
    if (!this.ready) return;
    this.track();
    if (this.app.checkLabelsError?.startsWith('Database checks skipped')) this.app.notice = [this.app.notice, this.app.checkLabelsError].filter(Boolean).join(' ');
    if (!this.staleOpen && this.canSyncOnSave) {
      if (this.pixelsChanged) this.dialog = 'pixels';
      else await this.runQuietSync();
    }
  }
}
