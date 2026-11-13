<script lang="ts">
  import DraftList from '../components/DraftList.svelte';
  import { onMount } from 'svelte';
  import { app, shared, tabs } from '../lib/store.svelte';
  import { fromDrop, fromInput } from '../lib/files';
  import { pendingProjectNotice, loadProjectFile, openExample, openFiles, openPaths, targetTab } from '../lib/actions';
  import { isDesktop } from '../lib/core/core';
  import { readRecent, type RecentEntry } from '../lib/deeplink';
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
    if (!keptCopies.has(name)) keptCopies.set(name, storedProject(name, localStorage));
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
  });

  /** Packages: counts from the copy this app keeps. Saved projects: their file, read in the background (a few at a time, each bounded). */
  async function refreshCounts() {
    for (const r of recent) {
      const c = recentCounts(r, kept(r.name));
      if (!c.cached && (c.areas !== r.areas || c.spots !== r.spots)) recent = updateRecentCounts(r.name, c, localStorage);
    }
    if (!desktop) return;
    const saved = recent.filter((r) => r.path);
    for (let i = 0; i < saved.length; i += 3) {
      await Promise.all(saved.slice(i, i + 3).map(async (r) => {
        try {
          const c = await invoke<{ areas: number; spots: number }>('project_counts', { path: r.path });
          fresh = { ...fresh, [r.name]: true };
          if (c.areas !== r.areas || c.spots !== r.spots) recent = updateRecentCounts(r.name, c, localStorage);
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
    await openPaths([location.path], targetTab());
  }

  async function choose(r: RecentEntry) {
    if (busy) return;
    busy = r.name;
    note = null;
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

  function rowKey(e: KeyboardEvent, r: RecentEntry) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      void choose(r);
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
    recent = relocateRecent(r.name, { kind: project ? 'project' : 'package', path: p }, localStorage);
    keptCopies.delete(r.name);
    note = null;
    const moved = recent.find((x) => x.name === r.name);
    if (moved) await choose(moved);
  }

  function forget(r: RecentEntry) {
    recent = removeRecent(r.name, localStorage);
    note = null;
  }

  async function pick() {
    if (desktop) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const p = await open({ directory: true, multiple: false, title: 'Choose a package or area folder' });
      if (typeof p === 'string') openPaths([p], targetTab());
    } else input.click();
  }

  async function pickImages() {
    if (desktop) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const p = await open({ directory: false, multiple: true, title: 'Choose images', filters: [{ name: 'Images', extensions: IMAGE_EXT }] });
      if (Array.isArray(p) && p.length) openPaths(p, targetTab());
      else if (typeof p === 'string') openPaths([p], targetTab());
    } else imagesInput.click();
  }

  async function chosen(el: HTMLInputElement) {
    if (!el.files?.length) return;
    const { name, files } = fromInput(el.files);
    el.value = '';
    await openFiles(name, files, targetTab());
  }

  async function drop(e: DragEvent) {
    e.preventDefault();
    over = false;
    if (desktop || !e.dataTransfer) return;
    const r = await fromDrop(e.dataTransfer);
    if (r) await openFiles(r.name, r.files, targetTab());
    else app.error = 'Drop a package folder, an area folder or image files.';
  }


  async function project(e: Event) {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f) await loadProjectFile(f).catch((err) => (app.error = String(err)));
  }
</script>

<section class="open">
  <DraftList />
  {#each shared.pendingProjects as pending (pending.project.package.name)}
    <div class="glass panel linked" role="status">
      <p data-testid="pending-project">{pendingProjectNotice(pending.project)}</p>
      <div class="row"><button class="primary" onclick={pick} data-testid="pending-open-package">Open package…</button></div>
    </div>
  {/each}
  {#if app.deepLink}
    <div class="glass panel linked" role="status" data-testid="deeplink">
      <h2>Linked project</h2>
      {#if app.deepLink.name}
        <p>The link points to project <code>{app.deepLink.project}</code>{#if app.deepLink.spot}, spot <code>{app.deepLink.spot}</code>{/if}, last opened here from the package <strong>{app.deepLink.name}</strong>. Images are not kept in this page: open that package folder again and the {app.deepLink.spot ? 'spot is selected' : 'project comes back'}.</p>
      {:else}
        <p>Project <code>{app.deepLink.project}</code>{#if app.deepLink.spot} (spot <code>{app.deepLink.spot}</code>){/if} is not available on this computer. Open its package folder, then load the project's saved <code>project.json</code> (Project file, below) to continue.</p>
      {/if}
      <div class="row">
        <button class="primary" onclick={pick} data-testid="deeplink-open">Open package…</button>
        <button onclick={() => (app.deepLink = null)}>Dismiss</button>
      </div>
    </div>
  {/if}
  <div
    class="glass drop"
    class:over
    role="button"
    tabindex="0"
    aria-label="Open a package folder, an area folder or images"
    ondragover={(e) => { e.preventDefault(); over = true; }}
    ondragleave={() => (over = false)}
    ondrop={drop}
    onclick={pick}
    onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && pick()}
  >
    <h1>Open images</h1>
    <p class="muted">Drop a folder or image files here, or click to choose a folder. Any of these opens:</p>
    <ul class="kinds">
      <li><b>A package</b>: a Hitachi TM4000 Zigzag folder (the <code>.zag</code> and one subfolder per area), or images with a <code>positions.csv</code> (<code>file,x_um,y_um,pixel_size_nm</code>).</li>
      <li><b>One area folder</b> on its own (<code>001_name/</code> with its <code>.ini</code>, tiles and <code>.txt</code> sidecars): the scale comes from the sidecars.</li>
      <li><b>Loose images</b> with no sidecars: you give the scale once (µm per pixel or field width); each image is an area, or they form one grid.</li>
    </ul>
    <p class="faint">Nothing leaves this computer: the files are read in this page{desktop ? ' (desktop app)' : ''}.</p>
    <input bind:this={input} type="file" webkitdirectory multiple hidden onchange={() => chosen(input)} data-testid="package-input" />
    <input bind:this={imagesInput} type="file" multiple accept=".jpg,.jpeg,.png,.tif,.tiff,.bmp,image/*" hidden onchange={() => chosen(imagesInput)} data-testid="images-input" />
  </div>
  <div class="example">
    <button onclick={pick} data-testid="choose-folder">Choose folder…</button>
    <button onclick={pickImages} data-testid="choose-images">Choose images…</button>
  </div>

  {#if !desktop}<div class="example">
    <button onclick={() => openExample(targetTab())} data-testid="example">Try the example</button>
    <span class="faint">one 3 × 2 TM4000 area (HJ-GC1, 150×, SE) from the test fixture</span>
  </div>{/if}

  <div class="cols">
    <div class="glass panel">
      <h2>Recent projects</h2>
      {#if recent.length}
        <table class="data">
          <thead><tr><th>Package</th><th>Areas</th><th>Spots</th><th>Last opened</th></tr></thead>
          <tbody>
            {#each recent as r (r.name)}
              {@const c = shown(r)}
              {@const path = where(r)}
              <tr
                class="recent"
                class:busy={busy === r.name}
                tabindex="0"
                role="button"
                aria-label={`Open ${r.name}`}
                aria-busy={busy === r.name}
                title={path ?? 'Location not recorded: choose its folder'}
                data-testid="recent-row"
                onclick={() => choose(r)}
                onkeydown={(e) => rowKey(e, r)}
              >
                <td>{r.name}{#if busy === r.name}<span class="faint small"> opening…</span>{/if}</td>
                <td class="num" title={c.cached ? 'Cached when last opened' : undefined}>{c.areas}</td>
                <td class="num" title={c.cached ? 'Cached when last opened' : undefined}>{c.spots}</td>
                <td class="muted">{new Date(r.when).toLocaleString()}</td>
              </tr>
              {#if note?.name === r.name}
                <tr class="note"><td colspan="4">
                  <div role="status" data-testid="recent-note">
                    {#if note.kind === 'missing'}
                      <p><strong>{r.name}</strong> is not at <code>{note.path}</code>.</p>
                    {:else if note.kind === 'unreachable'}
                      <p><code>{note.path}</code> did not answer within 4 s; Google Drive may be offline.</p>
                    {:else if note.kind === 'unlocated'}
                      <p>Location not recorded: choose its folder.</p>
                    {:else if note.kind === 'wrong'}
                      <p>The folder <code>{baseName(note.path ?? '')}</code> is not the package <strong>{r.name}</strong>. Choose the folder named {r.name}.</p>
                    {:else}
                      <p>Opening by path needs the desktop app.{#if note.path} Its folder: <code>{note.path}</code>.{/if}</p>
                    {/if}
                    <div class="row">
                      {#if note.kind === 'unsupported'}
                        <button class="primary" onclick={pick} data-testid="recent-choose">Choose folder…</button>
                      {:else}
                        {#if note.kind === 'unreachable'}<button class="primary" onclick={() => choose(r)} data-testid="recent-retry">Try again</button>{/if}
                        <button class:primary={note.kind !== 'unreachable'} onclick={() => locate(r)} data-testid="recent-locate">Locate…</button>
                        <button onclick={() => forget(r)} data-testid="recent-remove">Remove from list</button>
                      {/if}
                      <button onclick={() => (note = null)}>Dismiss</button>
                    </div>
                  </div>
                </td></tr>
              {/if}
            {/each}
          </tbody>
        </table>
        <p class="faint small">Click a project to open it: grains, spots and parameters come back. Removing one from this list keeps its saved work.</p>
      {:else}
        <p class="muted">None yet.</p>
      {/if}
    </div>
    <div class="glass panel">
      <h2>Project file</h2>
      <p class="muted">After opening the package, load a saved <code>project.json</code> to restore its parameters, grains and spots.</p>
      <input type="file" accept=".json,.gaproj,application/json" onchange={project} disabled={!app.pkg} />
    </div>
  </div>
</section>

<style>
  .open {
    max-width: 960px;
    margin: var(--space-8) auto;
    padding: 0 var(--space-4);
    display: grid;
    gap: var(--space-6);
    overflow: auto;
    max-height: 100%;
  }
  .drop {
    padding: var(--space-8);
    text-align: center;
    cursor: pointer;
    border-style: dashed;
    transition: border-color var(--motion), background var(--motion);
  }
  .drop.over,
  .drop:hover {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .drop p {
    max-width: 44rem;
    margin: var(--space-3) auto 0;
  }
  .kinds {
    max-width: 44rem;
    margin: var(--space-3) auto 0;
    text-align: left;
    display: grid;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.92rem;
  }
  .example {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    justify-content: center;
  }
  .cols {
    display: grid;
    grid-template-columns: 2fr 1fr;
    gap: var(--space-4);
  }
  .small {
    font-size: 0.8rem;
  }
  .linked {
    border-color: var(--accent);
  }
  .linked p {
    margin: var(--space-2) 0 var(--space-3);
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  tr.recent {
    cursor: pointer;
  }
  tr.recent:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  tr.recent:focus-visible td {
    background: var(--accent-soft);
  }
  tr.recent.busy {
    cursor: progress;
  }
  tr.note td {
    background: var(--accent-soft);
  }
  tr.note p {
    margin: 0 0 var(--space-2);
    overflow-wrap: anywhere;
  }
</style>
