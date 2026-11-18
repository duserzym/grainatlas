<script lang="ts">
  import { oversightSession } from '../lib/oversight';
  import { existingProject } from '../lib/open-target';
  import { draftSyncStatus, plainSyncStatus } from '../lib/sync-status';
  import { appStorage } from '../lib/app-storage';
  import { storageFullMessage } from '../lib/storage-failure';
  import { onMount } from 'svelte';
  import { app, tabs } from '../lib/store.svelte';
  import { DRAFTS_CHANGED, draftElements, hasPendingChanges, listDrafts, pendingElements, storedDraftProject, writeDraft, deleteDraft, deleteDraftCopies, type Draft } from '../lib/drafts';
  import { changeCounts, describeChanges, diffElements, emptyElements, groupedGrains, hasChanges, uiSpots } from '../lib/sync';
  import type { Project } from '../lib/core/types';
  import { SyncApi } from '../lib/sync-api';
  import { authToken } from '../lib/atlas';
  import { openFromServer } from '../lib/remote';
  let drafts = $state<Draft[]>([]);
  let busy = $state<string | null>(null);
  let note = $state('');
  let offline = $state<Record<string, boolean>>({});

  onMount(() => {
    const refresh = () => { drafts = listDrafts(app.server).filter(d => hasPendingChanges(d)); };
    refresh(); window.addEventListener(DRAFTS_CHANGED, refresh);
    return () => window.removeEventListener(DRAFTS_CHANGED, refresh);
  });
  function statusFor(draft: Draft) {
    const owner = tabs.list.find(tab => tab.project?.uuid === draft.uuid && tab.sync.draft?.uuid === draft.uuid);
    return (busy !== draft.uuid ? owner?.sync.text : null) ?? plainSyncStatus(draft, draftSyncStatus(draft), { busy: busy === draft.uuid, reachable: offline[draft.uuid] ? false : undefined });
  }
  async function check(draft: Draft) {
    busy = draft.uuid; note = ''; offline[draft.uuid] = false;
    try {
      const elements = draftElements(draft) ?? draft.base;
      if (!elements) throw new Error('The pending changes are unavailable. Open the project file to check.');
      const server = draft.server ?? app.server;
      if (!server) throw new Error('Set the database server in Settings, then Check.');
      const api = new SyncApi(server, await authToken());
      const status = await api.lookup(draft.linked_uuid ?? draft.uuid);
      // Check is read-only for legacy project content. Name matches are offered on opening.
      const theirs = status ? await api.elements(status.project_uuid) : null;
      const base = draft.base ?? (theirs ? { spots: theirs.spots, grains: theirs.grains, labels: theirs.labels } : emptyElements());
      const changes = changeCounts(diffElements(base, elements));
      const pending = !draft.base || !status || hasChanges(changes) ? pendingElements(elements) : null;
      const updated = { ...draft, checked_at: new Date().toISOString(), server_revision: status?.revision, server_can_edit: status?.can_edit, server, mount: status?.mount_id ?? draft.mount, base: draft.base, base_revision: draft.base_revision ?? theirs?.revision ?? null, base_hashes: draft.base_hashes ?? null, synced: draft.synced ?? false, changes, pending, pending_at: pending ? draft.pending_at ?? draft.updated_at : null };
      const owner = tabs.list.find(tab => tab.project?.uuid === draft.uuid);
      const kept = owner ? appStorage.track(owner, 'draft', () => writeDraft(updated)) : await appStorage.commit(() => writeDraft(updated));
      drafts = drafts.map(d => d.uuid === draft.uuid ? updated : d).filter(d => hasPendingChanges(d));
      if (!kept) note = storageFullMessage(draft.name);
      else if (!hasChanges(updated.changes)) note = `${draft.name}: no element changes waiting to sync.`;
    } catch (e) { offline[draft.uuid] = true; note = `${draft.name}: ${e instanceof Error ? e.message : e}. Changes kept here.`; }
    finally { busy = null; }
  }
  async function sync(draft: Draft) {
    busy = draft.uuid;
    const server = draft.server ?? app.server;
    if (!server) { note = 'Set the database server in Settings, then Check.'; busy = null; return; }
    const existing = existingProject(tabs.list, draft.uuid);
    if (existing) { tabs.select(existing.id); await existing.sync.action(); busy = null; return; }
    const tab = tabs.add();
    try {
      const elements = draftElements(draft) ?? draft.base;
      if (!elements) throw new Error('The pending changes are unavailable. Open the project file to sync.');
      const legacy = draft.pending === undefined ? storedDraftProject(draft) : null;
      const api = new SyncApi(server, await authToken(tab));
      const published = await api.publishedProject(draft.linked_uuid ?? draft.uuid);
      if (published) {
        if (!(await openFromServer(server, draft.linked_uuid ?? draft.uuid, null, null, draft.mount, tab))) return;
        if (tab.remote) tab.remote.readOnly = false;
      }
      // Pending records need no project copy. A bundle supplies display metadata when
      // available; otherwise an empty session can register/sync/review the elements.
      const project: Project = { ...(legacy ?? tab.project ?? {
        schema_version: 1, created: draft.pending_at ?? draft.updated_at,
        package: { path: '', reader: '', name: draft.name, kind: 'package', meta: {} },
        areas: [], settings: tab.settings,
      }), uuid: draft.uuid, name: draft.name, upbdb: { mount_id: draft.mount } };
      const core = await tab.core;
      await core.blank(project.package.name);
      const { grainatlas_ui: _ui, ...record } = project;
      await core.loadProject(JSON.stringify(record));
      tab.project = project;
      tab.adopt(project);
      tab.spots = uiSpots(elements.spots, elements.grains, tab.spots, Object.fromEntries(project.areas.map(a => [a.id, a.sample])));
      tab.grains = groupedGrains(elements.grains, tab.grains);
      tab.labels = elements.labels;
      tab.sync.opened();
      await tab.sync.check();
      await tab.sync.action();
    } catch (e) { tab.error = e instanceof Error ? e.message : String(e); }
    finally { busy = null; }
  }

  function discard(draft: Draft) {
    const changes = draft.changes ? describeChanges(draft.changes) || 'no known element changes' : 'changes that have not been checked';
    if (!confirm(`Discard sync tracking for ${draft.name} (${changes})? The pending sync changes, base snapshot and sync history here will be lost. The stored project copy stays unless you separately delete it.`)) return;
    deleteDraft(draft.uuid);
    drafts = drafts.filter(d => d.uuid !== draft.uuid);
    if (confirm(`Also permanently delete the stored project copies for ${draft.name}? Their local edits will be lost. Files on disk and database content stay as they are.`)) deleteDraftCopies(draft);
  }
</script>

{#if drafts.length || note}
  <section class="glass panel" data-testid="draft-list">
    <h2>Work waiting to sync</h2>
    {#each drafts as draft (draft.uuid)}
      <div class="draft" data-testid="draft-row">
        <div><strong>{draft.name}</strong> · <span class="faint small" data-testid="draft-status">{statusFor(draft)}</span><p class="faint small">{new Date(draft.updated_at).toLocaleString()}</p></div>
        <button disabled={busy !== null || (oversightSession.readOnly && draft.changes !== null)} onclick={() => draft.changes === null ? check(draft) : sync(draft)}>{draft.changes === null ? 'Check' : 'Sync'}</button>
        <button disabled={busy !== null} onclick={() => discard(draft)}>Discard</button>
      </div>
    {/each}
    {#if note}<p role="status">{note}</p>{/if}
  </section>
{/if}

<style>
  .draft { display:flex; flex-wrap:wrap; align-items:center; gap:var(--space-2); border-top:1px solid var(--hairline); padding-block:var(--space-2); }
  .draft div { flex:1 1 220px; min-width:0; overflow-wrap:anywhere; }
  .draft p { margin:var(--space-1) 0; }
</style>
