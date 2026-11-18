<script lang="ts">
  import { appStorage } from '../lib/app-storage';
  import StartMounts from '../components/StartMounts.svelte';
  import MountPicker from '../components/MountPicker.svelte';
  import { oversightSession } from '../lib/oversight';
  import { SyncApi } from '../lib/sync-api';
  import { authToken } from '../lib/atlas';
  import { DRAFTS_CHANGED, listDrafts, storedDraftProject, ensureDraft, writeDraft, type Draft } from '../lib/drafts';
  import type { MountOut } from '../lib/upbdb';
  import DraftList from '../components/DraftList.svelte';
  import { onMount } from 'svelte';
  import { app, shared, tabs } from '../lib/store.svelte';
  import { fromDrop, fromInput } from '../lib/files';
  import { pendingProjectNotice, loadProjectFile, openFiles, openPaths } from '../lib/actions';
  import { isDesktop } from '../lib/core/core';
  import { RECENT_KEY, readRecent, type RecentEntry } from '../lib/deeplink';
  import type { Project } from '../lib/core/types';
  import { openRecent, recentCounts, recentLocation, relocateRecent, removeRecent, samePackage, storedProject, updateRecentCounts, baseName, type PathStatus, type RecentLocation } from '../lib/recent';

  let over = $state(false);
  let input: HTMLInputElement;
  let imagesInput: HTMLInputElement;
  const IMAGE_EXT = ['jpg', 'jpeg', 'png', 'tif', 'tiff', 'bmp'];
  let recent = $state<RecentEntry[]>([]);
  const desktop = isDesktop();

  /** The row being checked or opened, and the row whose inline message is shown. */
  let busy = $state<string | null>(null);
  let note = $state<{ name: string; kind: 'missing' | 'unreachable' | 'unlocated' | 'unsupported' | 'wrong'; path: string | null } | null>(null);
  /** Rows whose numbers were read from the project file (saved projects). */
  let fresh = $state<Record<string, boolean>>({});

  // The kept copies are megabytes of JSON: parse each once, not on every render.
  const keptCopies = new Map<string, Project | null>();
  function kept(name: string): Project | null {
    if (!keptCopies.has(name)) keptCopies.set(name, storedProject(name, appStorage));
    return keptCopies.get(name) ?? null;
  }

  async function invoke<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }

  // Desktop: the command line and dropped folders are handled for the whole window (App.svelte).
  onMount(() => {
    recent = readRecent();
    refreshCounts();
    void localOverviews();
  });

  /** Use an already loaded local pyramid; opening the start screen never loads source images. */
  async function localOverviews() {
    for (const tab of tabs.list) {
      if (tab.readOnly || !tab.project) continue;
      const uuid = tab.project.uuid;
      const entry = recent.find(r => r.id === uuid);
      if (!entry || entry.thumbnail) continue;
      const [area, stitched] = Object.entries(tab.stitched)[0] ?? [];
      const level = stitched?.mosaic.levels.at(-1);
      if (!area || !level || level.cols !== 1 || level.rows !== 1) continue;
      try {
        const raw = await (await tab.core).pyramidTile(area, level.level, 0, 0);
        if (tab.readOnly || tab.project?.uuid !== uuid) continue;
        const source = document.createElement('canvas'); source.width = raw.w; source.height = raw.h;
        source.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(raw.data), raw.w, raw.h), 0, 0);
        const canvas = document.createElement('canvas');
        const ratio = Math.min(1, 200 / Math.max(raw.w, raw.h));
        canvas.width = Math.max(1, Math.round(raw.w * ratio)); canvas.height = Math.max(1, Math.round(raw.h * ratio));
        canvas.getContext('2d')!.drawImage(source, 0, 0, canvas.width, canvas.height);
        const thumbnail = canvas.toDataURL('image/webp', .7);
        if (thumbnail.length > 100000) continue;
        recent = readRecent().map(r => r.id === uuid ? { ...r, thumbnail } : r);
        appStorage.setItem(RECENT_KEY, JSON.stringify(recent));
      } catch { /* the local pyramid may have been evicted */ }
    }
  }

  /** Packages: counts from the copy this app keeps. Saved projects: their file, read in the background (a few at a time, each bounded). */
  async function refreshCounts() {
    for (const r of recent) {
      const c = recentCounts(r, kept(r.name));
      if (!c.cached && (c.areas !== r.areas || c.spots !== r.spots)) recent = updateRecentCounts(r.name, c, appStorage);
    }
    if (!desktop) return;
    const saved = recent.filter((r) => r.path);
    for (let i = 0; i < saved.length; i += 3) {
      await Promise.all(saved.slice(i, i + 3).map(async (r) => {
        try {
          const c = await invoke<{ areas: number; spots: number }>('project_counts', { path: r.path });
          fresh = { ...fresh, [r.name]: true };
          if (c.areas !== r.areas || c.spots !== r.spots) recent = updateRecentCounts(r.name, c, appStorage);
        } catch { /* unreadable or offline: the cached numbers stay, marked so */ }
      }));
    }
  }

  function shown(r: RecentEntry) {
    const c = recentCounts(r, kept(r.name));
    return { ...c, cached: c.cached && !fresh[r.name] };
  }

  function where(r: RecentEntry): string | null {
    return recentLocation(r, kept(r.name))?.path ?? null;
  }

  async function openAt(location: RecentLocation) {
    await openPaths([location.path]);
  }

  async function choose(r: RecentEntry) {
    if (busy) return;
    busy = r.name;
    note = null;
    const draft = localDrafts.find(d => d.uuid === r.id);
    const copy = draft ? storedDraftProject(draft) : null;
    if (copy && !keptCopies.has(r.name)) keptCopies.set(r.name, copy);
    try {
      const out = await openRecent(r, kept(r.name), {
        canOpenPaths: desktop,
        focusExisting: (name) => {
          const t = tabs.list.find((x) => x.pkg?.name === name);
          if (t) tabs.select(t.id);
          return !!t;
        },
        status: (path) => invoke<PathStatus>('path_status', { path }),
        open: openAt,
      });
      if (out.kind === 'missing' || out.kind === 'unreachable') note = { name: r.name, kind: out.kind, path: out.path };
      else if (out.kind === 'unlocated') note = { name: r.name, kind: 'unlocated', path: null };
      else if (out.kind === 'unsupported') note = { name: r.name, kind: 'unsupported', path: out.path };
    } catch (err) {
      app.error = String(err);
    } finally {
      busy = null;
    }
  }

  async function locate(r: RecentEntry) {
    const project = !!r.path;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const gaproj = project && /\.gaproj$/i.test(r.path!);
    const p = await open(gaproj
      ? { directory: false, multiple: false, title: `Locate ${r.name}`, filters: [{ name: 'GrainAtlas project', extensions: ['gaproj'] }] }
      : { directory: true, multiple: false, title: `Locate ${r.name}` });
    if (typeof p !== 'string') return;
    if (!project && !samePackage(r.name, p)) {
      note = { name: r.name, kind: 'wrong', path: p };
      return;
    }
    recent = relocateRecent(r.name, { kind: project ? 'project' : 'package', path: p }, appStorage);
    keptCopies.delete(r.name);
    note = null;
    const moved = recent.find((x) => x.name === r.name);
    if (moved) await choose(moved);
  }

  function forget(r: RecentEntry) {
    recent = removeRecent(r.name, appStorage);
    note = null;
  }

  async function pick() {
    if (desktop) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const p = await open({ directory: true, multiple: false, title: 'Choose a package or area folder' });
      if (typeof p === 'string') openPaths([p]);
    } else input.click();
  }

  async function pickImages() {
    if (desktop) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const p = await open({ directory: false, multiple: true, title: 'Choose images', filters: [{ name: 'Images', extensions: IMAGE_EXT }] });
      if (Array.isArray(p) && p.length) openPaths(p);
      else if (typeof p === 'string') openPaths([p]);
    } else imagesInput.click();
  }

  async function chosen(el: HTMLInputElement) {
    if (!el.files?.length) return;
    const { name, files } = fromInput(el.files);
    el.value = '';
    await openFiles(name, files);
  }

  async function drop(e: DragEvent) {
    e.preventDefault();
    over = false;
    if (desktop || !e.dataTransfer) return;
    const r = await fromDrop(e.dataTransfer);
    if (r) await openFiles(r.name, r.files);
    else app.error = 'Drop a package folder, an area folder or image files.';
  }


  async function project(e: Event) {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f) await loadProjectFile(f).catch((err) => (app.error = String(err)));
  }
  let mountContent = $state(false);
  let mountOffline = $state(false);
  let signedIn = $state(false);
  let refreshMounts = $state(0);
  let linkDialog = $state<{ recent?: RecentEntry; mount?: MountOut; create?: boolean } | null>(null);
  let linkNote = $state('');
  let linking = $state(false);
  let localDrafts = $state<Draft[]>([]);
  let split: HTMLDivElement;
  let projectInput: HTMLInputElement;
  const hasContent = $derived(recent.length > 0 || mountContent);
  onMount(() => {
    const update = () => localDrafts = listDrafts();
    update(); window.addEventListener(DRAFTS_CHANGED, update);
    return () => window.removeEventListener(DRAFTS_CHANGED, update);
  });
  function unlinked(r: RecentEntry) {
    return !localDrafts.find(d => d.uuid === r.id)?.mount && !kept(r.name)?.upbdb?.mount_id;
  }
  function showLink(r?: RecentEntry, mount?: MountOut, create = false) {
    linkNote = ''; linkDialog = { recent: r, mount, create };
  }
  async function linkMount(id: string, r = linkDialog?.recent) {
    if (oversightSession.readOnly || linking || !app.server) return;
    if (!r) { linkDialog = null; refreshMounts++; return; }
    const project = kept(r.name);
    const uuid = r.id ?? project?.uuid;
    if (!uuid) { linkNote = 'Open this project first to link it.'; return; }
    linking = true; linkNote = '';
    try {
      const server = app.server;
      await new SyncApi(server, await authToken()).link(uuid, id);
      const draft = localDrafts.find(d => d.uuid === uuid) ?? (project ? ensureDraft(project, server) : {
        uuid, name: r.name, server, mount: null, base_revision: null, base: null, updated_at: r.when, changes: null,
      });
      const updated = { ...draft, server, mount: id, link_offered: true, link_candidates: [] };
      writeDraft(updated); localDrafts = [...localDrafts.filter(d => d.uuid !== uuid), updated];
      const open = tabs.list.find(t => t.project?.uuid === uuid);
      if (open) await open.sync.check();
      linkDialog = null; refreshMounts++;
    } catch (e) { linkNote = e instanceof Error ? e.message : String(e); }
    finally { linking = false; }
  }
  function showModal(node: HTMLDialogElement) { node.showModal(); return { destroy: () => node.close() }; }
  function resize(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      app.layout.recentWidth = Math.max(180, Math.min(360, e.clientX - split.getBoundingClientRect().left));
    };
    const stop = () => { handle.removeEventListener('pointermove', move); handle.removeEventListener('pointerup', stop); handle.removeEventListener('pointercancel', stop); app.saveLayout(); };
    handle.addEventListener('pointermove', move); handle.addEventListener('pointerup', stop); handle.addEventListener('pointercancel', stop);
  }
</script>

<section class="open">
  <div class="glass panel action-bar" class:over ondragover={e => { e.preventDefault(); over = true; }} ondragleave={() => over = false} ondrop={drop} role="region" aria-label="Open files">
    <button onclick={pick} data-testid="choose-folder">Open package…</button>
    <button onclick={() => projectInput.click()}>Open project…</button>
    <button disabled={!signedIn || mountOffline || oversightSession.readOnly} onclick={() => showLink(undefined, undefined, true)}>New mount…</button>
    <button onclick={pickImages} data-testid="choose-images">Choose images…</button>
    {#if hasContent}<span class="faint small">Drop a folder or images here</span>{/if}
  </div>
  <input bind:this={input} type="file" webkitdirectory multiple hidden onchange={() => chosen(input)} data-testid="package-input" />
  <input bind:this={imagesInput} type="file" multiple accept=".jpg,.jpeg,.png,.tif,.tiff,.bmp,image/*" hidden onchange={() => chosen(imagesInput)} data-testid="images-input" />
  <input bind:this={projectInput} type="file" accept=".json,.gaproj,application/json" hidden onchange={project} data-testid="project-input" />
  {#if !hasContent}
    <div class="glass drop" class:over role="button" tabindex="0" aria-label="Open a package folder, an area folder or images"
      ondragover={e => { e.preventDefault(); over = true; }} ondragleave={() => over = false} ondrop={drop} onclick={pick} onkeydown={e => (e.key === 'Enter' || e.key === ' ') && pick()}>
      <h1>Open images</h1>
      <p class="muted">Drop a package folder, an area folder or image files here.</p>
      <p class="faint small">The files stay on this computer. For loose images, give the scale once.</p>
    </div>
  {/if}
  {#each shared.pendingProjects as pending (pending.project.package.name)}
    <div class="glass panel" role="status"><p data-testid="pending-project">{pendingProjectNotice(pending.project)}</p><button onclick={pick} data-testid="pending-open-package">Open package…</button></div>
  {/each}
  {#if app.deepLink}
    <div class="glass panel" role="status" data-testid="deeplink"><h2>Linked project</h2>
      <p>Project <code>{app.deepLink.project}</code>{#if app.deepLink.spot}, spot <code>{app.deepLink.spot}</code>{/if}. Open {app.deepLink.name ?? 'its package folder'} to continue.</p>
      <button onclick={pick} data-testid="deeplink-open">Open package…</button><button onclick={() => app.deepLink = null}>Dismiss</button>
    </div>
  {/if}
  <div class="split" bind:this={split} style={`--recent-width: ${app.layout.recentWidth}px`}>
    <aside class="glass panel recent-list">
      <h2>Recent projects</h2>
      {#each recent as r (r.name)}
        {@const c = shown(r)}
        <div class="recent-item" data-testid="recent-row">
          <button class="recent-title" disabled={!!busy} onclick={() => choose(r)} title={where(r) ?? 'Location not recorded'}>{r.name}{busy === r.name ? ' opening…' : ''}</button>
          <p class="faint small">{c.areas} areas · {c.spots} spots{c.cached ? ' · cached' : ''}</p>
          <p class="faint small">{new Date(r.when).toLocaleString()}</p>
          <p class="small">On this computer</p>
          {#if unlinked(r)}<button class="small" disabled={!signedIn || mountOffline || oversightSession.readOnly} onclick={() => showLink(r)}>Link to a mount</button>{/if}
          {#if note?.name === r.name}
            <div class="note" role="status" data-testid="recent-note">
              {#if note.kind === 'missing'}<p>{r.name} is not at <code>{note.path}</code>.</p>
              {:else if note.kind === 'unreachable'}<p><code>{note.path}</code> did not answer within 4 s; storage may be offline.</p>
              {:else if note.kind === 'unlocated'}<p>Location not recorded: choose its folder.</p>
              {:else if note.kind === 'wrong'}<p>The folder {baseName(note.path ?? '')} is not the package {r.name}.</p>
              {:else}<p>Opening by path needs the desktop app.</p>{/if}
              <div class="row">
                {#if note.kind === 'unsupported'}<button onclick={pick} data-testid="recent-choose">Choose folder…</button>
                {:else}
                  {#if note.kind === 'unreachable'}<button onclick={() => choose(r)} data-testid="recent-retry">Try again</button>{/if}
                  <button onclick={() => locate(r)} data-testid="recent-locate">Locate…</button>
                  <button onclick={() => forget(r)} data-testid="recent-remove">Remove from list</button>
                {/if}
                <button onclick={() => note = null}>Dismiss</button>
              </div>
            </div>
          {/if}
        </div>
      {:else}<p class="muted">None yet.</p>{/each}
      <details><summary>Sync drafts</summary><DraftList /></details>
    </aside>
    <div class="splitter" role="slider" tabindex="0" aria-label="Recent projects width" aria-orientation="vertical" aria-valuenow={app.layout.recentWidth} aria-valuemin="180" aria-valuemax="360" onpointerdown={resize}
      onkeydown={e => { if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') { e.preventDefault(); app.layout.recentWidth = Math.max(180, Math.min(360, app.layout.recentWidth + (e.key === 'ArrowLeft' ? -10 : 10))); app.saveLayout(); } }}></div>
    <StartMounts {recent} onopen={choose} onlink={m => showLink(undefined, m)} oncontent={n => { if (n > 0) mountContent = true; }} onauth={value => { signedIn = value; if (!value) mountContent = false; }} onoffline={value => mountOffline = value} refresh={refreshMounts} />
  </div>
  {#if linkDialog}
    <dialog use:showModal class="glass panel mount-dialog" aria-label={linkDialog.create ? 'New mount' : 'Link a local project'} oncancel={e => { e.preventDefault(); if (!linking) linkDialog = null; }}>
        <h2>{linkDialog.create ? 'New mount' : linkDialog.mount ? `Link to ${linkDialog.mount.label}` : `Link ${linkDialog.recent?.name}`}</h2>
        {#if linkDialog.mount}
          {#each recent.filter(unlinked) as r}<button disabled={linking || oversightSession.readOnly} onclick={() => linkMount(linkDialog!.mount!.id, r)}>{r.name}</button>{:else}<p>No unlinked local projects. Open a local project first.</p>{/each}
        {:else}
          <MountPicker value={null} onchange={id => void linkMount(id)} disabled={linking || mountOffline} startNew={linkDialog.create} />
        {/if}
        {#if linkNote}<p role="status">{linkNote}</p>{/if}
        <button disabled={linking} onclick={() => linkDialog = null}>Close</button>
    </dialog>
  {/if}
</section>

<style>
  .open { width: 100%; height: 100%; padding: var(--space-3); display: flex; flex-direction: column; gap: var(--space-3); overflow: auto; min-height: 0; }
  .action-bar, .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); }
  .action-bar { padding: var(--space-2) var(--space-3); }
  .drop { padding: var(--space-6); text-align: center; cursor: pointer; border-style: dashed; }
  .over { border-color: var(--accent); background: var(--accent-soft); }
  .drop p { margin: var(--space-2) 0 0; }
  .split { display: grid; grid-template-columns: minmax(180px, var(--recent-width)) 8px minmax(0, 1fr); gap: var(--space-2); align-items: start; min-width: 0; }
  .splitter { align-self: stretch; min-height: 160px; cursor: col-resize; touch-action: none; border-left: 2px solid var(--hairline); }
  .splitter:focus-visible { outline: 2px solid var(--accent); }
  .recent-list { min-width: 0; padding: var(--space-3); }
  h2 { margin: 0 0 var(--space-2); }
  .recent-item { padding: var(--space-2) 0; border-bottom: 1px solid var(--hairline); overflow-wrap: anywhere; }
  .recent-item p { margin: var(--space-1) 0; }
  .recent-title { width: 100%; text-align: left; overflow-wrap: anywhere; font-weight: 600; }
  .note { font-size: .8rem; padding-top: var(--space-2); }
  .mount-dialog::backdrop { background: color-mix(in srgb, var(--bg) 55%, transparent); }
  .mount-dialog { margin: auto; width: min(500px, 90vw); max-height: 80vh; overflow: auto; display: grid; gap: var(--space-3); }
  @media (max-width: 850px) { .split { grid-template-columns: minmax(0, 1fr); } .splitter { display: none; } }
</style>
