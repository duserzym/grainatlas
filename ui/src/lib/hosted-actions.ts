import type { AtlasMode, AtlasResultCode, CommandResult } from '@lab/bridge';
import { hostedSync, ATLAS_EVENTS, type AtlasCommand } from './hosted-sync';
import { current, tabs, type AppState } from './store.svelte';
import { deleteDraft, draftElements, hasPendingChanges, listDrafts, readDraft, storedDraftProject, writeDraft } from './drafts';
import { emptyElements, groupedGrains, uiSpots } from './sync';
import { editCopy, openFromServer } from './remote';
import type { Project } from './core/types';
import { reportStorageWrite } from './storage-failure';

export function reportHostedStatus(app: AppState, force = false) {
  if (!app.project) return;
  const s = app.sync, mount = s.status?.mount_id ?? s.draft?.mount ?? app.mountId;
  hostedSync.status({ type: ATLAS_EVENTS.status, project: app.project.uuid, name: app.project.name,
    mount, mode: app.readOnly ? 'view' : 'edit', state: s.checking ? 'checking' : s.state,
    revision: mount ? s.status?.revision ?? s.draft?.base_revision ?? null : null,
    atlasVersion: s.status?.atlas_version ?? null, changes: { ...s.counts, pixels: s.pixelsChanged }, syncedAt: s.syncedAt }, force);
}
function result(app: AppState): Partial<CommandResult> {
  return { mount: app.sync.status?.mount_id ?? app.sync.draft?.mount ?? null, revision: app.sync.status?.revision ?? app.sync.draft?.base_revision ?? null };
}
const fail = (code: Exclude<AtlasResultCode, 'needs-confirmation'>, error: string): Partial<CommandResult> => ({ ok: false, code, error: error.slice(0, 300) });
function failure(app: AppState): Partial<CommandResult> {
  if (app.sync.confirmation) return { ok: false, code: 'needs-confirmation', confirm: app.sync.confirmation };
  if (app.sync.validationFailed) return fail('failed', app.sync.failureMessage || app.sync.error);
  if (app.sync.dialog === 'review' || app.sync.state === 'both-changed') return fail('conflict', 'Review differences before leaving Edit.');
  const error = app.sync.error || app.error || 'The action did not finish. Changes kept here.';
  return fail(/401|403/.test(error) ? 'unauthorized' : app.sync.reachable === false ? 'offline' : 'failed', error);
}
export async function setMode(app: AppState, mode: AtlasMode, own = false) {
  await app.sync.settled();
  if (mode === 'edit') {
    if (app.remote?.readOnly) await editCopy(app);
    else app.viewMode = false;
    if (app.readOnly) throw new Error('Could not enter Edit.');
    await app.sync.check();
    if (own) app.sync.showConfirmation();
  } else if (!app.readOnly) {
    if (own && app.sync.unsynced) {
      const decision = await new Promise<'sync' | 'keep-draft' | 'stay'>(answer => { app.leaveEditPrompt = { answer }; });
      app.leaveEditPrompt = null;
      if (decision === 'stay') return;
      const outcome = await leaveEdit(app, decision, false);
      if (outcome.ok === false) { if (outcome.error) app.error = outcome.error; return; }
    } else {
      app.sync.track(false);
      // The shell already asked; preserve its draft before switching to View.
      if (app.sync.unsynced && (!app.sync.draft || !writeDraft(app.sync.draft))) throw new Error('Could not keep the draft on this computer.');
      view(app);
    }
  }
  reportHostedStatus(app);
}
function view(app: AppState) {
  app.sync.viewed(app.sync.status?.revision ?? app.sync.draft?.base_revision ?? null);
  if (app.remote) app.remote = { ...app.remote, readOnly: true };
  else app.viewMode = true;
  app.tool = 'select';
}
export async function leaveEdit(app: AppState, decision: 'sync' | 'keep-draft' | 'discard', shellCommand = true): Promise<Partial<CommandResult>> {
  await app.sync.settled();
  if (app.readOnly) return result(app);
  if (decision === 'sync') {
    await app.sync.sync(false, shellCommand);
    if (app.sync.confirmation || app.sync.validationFailed || app.sync.dialog === 'review' || app.sync.unsynced || app.sync.reachable !== true) return failure(app);
  } else if (decision === 'keep-draft') {
    app.sync.track(false);
    if (!app.sync.draft || !writeDraft(app.sync.draft)) return fail('failed', 'Could not keep the draft on this computer.');
  } else {
    const base = app.sync.draft?.base ?? emptyElements();
    app.discardUndo = structuredClone(JSON.parse(JSON.stringify(app.sync.elements)));
    app.spots = uiSpots(base.spots, base.grains, app.spots);
    app.grains = groupedGrains(base.grains, app.grains); app.labels = structuredClone(base.labels);
    view(app); // suppress queued persistence before deleting tracking
    if (!deleteDraft(app.project!.uuid)) {
      const snapshot = app.discardUndo!;
      if (app.remote) app.remote = { ...app.remote, readOnly: false }; else app.viewMode = false;
      app.spots = uiSpots(snapshot.spots, snapshot.grains, app.spots);
      app.grains = groupedGrains(snapshot.grains, app.grains); app.labels = snapshot.labels;
      return fail('failed', 'Could not remove the draft record.');
    }
    app.sync.forgetDraft();
  }
  view(app);
  reportHostedStatus(app);
  hostedSync.drafts(listDrafts(app.server));
  return result(app);
}
export async function undoDiscard(app: AppState) {
  const snapshot = app.discardUndo;
  if (!snapshot) return;
  app.discardUndo = null;
  await setMode(app, 'edit');
  app.spots = uiSpots(snapshot.spots, snapshot.grains, app.spots);
  app.grains = groupedGrains(snapshot.grains, app.grains); app.labels = snapshot.labels;
  app.persist(); app.sync.track();
}
/** Home actions use pending first; display metadata is optional for a sync. */
async function draftTab(uuid: string): Promise<AppState | null> {
  const open = tabs.list.find(t => t.project?.uuid === uuid);
  if (open) { tabs.select(open.id); await setMode(open, 'edit'); return open; }
  const draft = listDrafts(current().server).find(d => d.uuid === uuid) ?? readDraft(uuid);
  if (!draft) return null;
  const elements = draftElements(draft) ?? draft.base ?? (!draft.mount ? emptyElements() : null);
  if (!elements) throw new Error('Open the project file to recover this draft.');
  const tab = tabs.add(), server = draft.server ?? tab.server;
  if (!server) throw new Error('Set the database server in Settings.');
  tab.serverOverride = server;
  const legacy = storedDraftProject(draft);
  // Opening pixels is optional: an unpublished or offline draft still stays intact.
  await openFromServer(server, draft.linked_uuid ?? uuid, null, null, draft.mount, tab);
  const project: Project = { ...(legacy ?? tab.project ?? { schema_version: 1, created: draft.updated_at, package: { name: draft.name, path: '', reader: '', kind: 'package', meta: {} }, areas: [], settings: tab.settings }), uuid, name: draft.name, laicpmsdb: { mount_id: draft.mount } };
  const core = await tab.core;
  await core.blank(project.package.name);
  const { grainatlas_ui: _ui, ...record } = project;
  await core.loadProject(JSON.stringify(record));
  tab.project = project; tab.error = null;
  if (tab.remote) tab.remote = { ...tab.remote, readOnly: false };
  tab.adopt(project);
  tab.spots = uiSpots(elements.spots, elements.grains, tab.spots);
  tab.grains = groupedGrains(elements.grains, tab.grains); tab.labels = elements.labels;
  reportStorageWrite(tab, 'draft', writeDraft(draft)); tab.sync.draft = draft;
  await tab.sync.check();
  return tab;
}
export async function runHostedCommand(cmd: AtlasCommand): Promise<Partial<CommandResult> | void> {
  if (cmd.type === 'report-status') { reportHostedStatus(current(), true); hostedSync.drafts(listDrafts(current().server), true); return; }
  if (cmd.type === 'atlas-synced') {
    for (const tab of tabs.list) if (tab.mountId === cmd.mount || tab.sync.status?.mount_id === cmd.mount) await tab.sync.check();
    return;
  }
  if (cmd.type === 'discard-draft') {
    const tab = tabs.list.find(t => t.project?.uuid === cmd.project);
    if (tab && !tab.readOnly) {
      const outcome = await leaveEdit(tab, 'discard');
      if (outcome.ok === false) return outcome;
    } else if (!deleteDraft(cmd.project)) return fail('failed', 'Could not remove the draft record.');
    if (tab) tab.sync.forgetDraft();
    hostedSync.drafts(listDrafts(current().server), true);
    return tab ? result(tab) : {};
  }
  let app = tabs.list.find(t => t.project?.uuid === cmd.project);
  if (cmd.type === 'open-project') {
    app ??= !current().project ? current() : tabs.add();
    tabs.select(app.id);
    const server = cmd.server ?? app.server;
    if (!server) return fail('failed', 'Set the database server in Settings.');
    app.serverOverride = server;
    if (app.project?.uuid === cmd.project && (!app.remote || app.remote.server === server)) {
      await setMode(app, 'view'); await app.sync.check();
    } else if (!(await openFromServer(server, cmd.project, null, null, null, app))) return fail(/401|403/.test(app.error ?? '') ? 'unauthorized' : /404/.test(app.error ?? '') ? 'not-found' : 'offline', app.error ?? 'Could not open the project.');
    if (cmd.mode === 'edit') { await setMode(app, 'edit'); app.sync.showConfirmation(); }
    reportHostedStatus(app, true);
    return result(app);
  }
  if (cmd.type === 'sync-draft') {
    if (!app) {
      const draft = listDrafts(current().server).find(d => d.uuid === cmd.project);
      if (draft && !hasPendingChanges(draft)) {
        current().notice = 'No changes to sync';
        return { ok: true, mount: draft.mount, revision: draft.base_revision };
      }
    }
    app = await draftTab(cmd.project) ?? undefined;
    if (!app) return fail('not-found', 'No draft for that project is kept here.');
    if (!app.sync.draft?.mount) {
      const error = app.sync.showLink();
      reportHostedStatus(app, true);
      return fail('failed', error);
    }
    await app.sync.sync(false, true);
    reportHostedStatus(app, true);
    return app.sync.confirmation || app.sync.validationFailed || app.sync.unsynced || app.sync.dialog === 'review' || app.sync.reachable !== true ? failure(app) : result(app);
  }
  if (!app) return fail('not-found', 'That project is not open.');
  tabs.select(app.id);
  if (cmd.type === 'set-mode') { await setMode(app, cmd.mode); if (cmd.mode === 'edit') app.sync.showConfirmation(); return result(app); }
  if (cmd.type === 'leave-edit') return leaveEdit(app, cmd.decision);
}
