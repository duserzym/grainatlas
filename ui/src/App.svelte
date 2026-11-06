<script lang="ts">
  import { onMount } from 'svelte';
  import { app, current, tabs, type Screen } from './lib/store.svelte';
  import { isHosted, notify, onCommand, onOpenFiles, SHELL_EVENTS } from '@lab/bridge';
  import { findAllGrains, liveChanged, liveKey, openLoose, openOffered, openPathsInTabs, saveProject, saveProjectFile, startDeepLink, stitchAll } from './lib/actions';
  import { isDesktop } from './lib/core/core';
  import TabBar from './components/TabBar.svelte';
  import Open from './screens/Open.svelte';
  import Area from './screens/Area.svelte';
  import Canvas from './screens/Canvas.svelte';
  import Layout from './screens/Layout.svelte';
  import Settings from './screens/Settings.svelte';
  import Busy from './components/Busy.svelte';
  import ScaleDialog from './components/ScaleDialog.svelte';
  import CheckDialog from './components/CheckDialog.svelte';
  import RenameDialog from './components/RenameDialog.svelte';
  import ReplaceDialog from './components/ReplaceDialog.svelte';
  import ThemeSwitch from './components/ThemeSwitch.svelte';
  import { handleShellCommand } from './lib/shellcmd';
  import { editCopy } from './lib/remote';

  const screens: { id: Screen; label: string; key: string; needs?: 'pkg' | 'mosaic' }[] = [
    { id: 'open', label: 'Open', key: '1' },
    { id: 'area', label: 'Area', key: '2', needs: 'pkg' },
    { id: 'canvas', label: 'Canvas', key: '3', needs: 'mosaic' },
    { id: 'layout', label: 'Layout', key: '4', needs: 'mosaic' },
    { id: 'settings', label: 'Settings', key: '5' },
  ];

  const hasMosaic = $derived(Object.keys(app.stitched).length > 0);
  function enabled(s: (typeof screens)[number]) {
    if (s.needs === 'pkg') return !!app.pkg;
    // A bundle from a server has tiles, not a mosaic in the core: the Canvas only.
    if (s.id === 'layout' && app.remote) return false;
    if (s.needs === 'mosaic') return hasMosaic;
    return true;
  }

  // Hash routing (#/canvas) so the build works from file://, any static host and Tauri.
  function fromHash() {
    const h = location.hash.replace(/^#\/?/, '') as Screen;
    const s = screens.find((x) => x.id === h);
    if (s && enabled(s)) app.screen = s.id;
  }
  $effect(() => {
    const want = `#/${app.screen}`;
    if (location.hash !== want) history.replaceState(null, '', want);
  });

  // Any change of a stitched area's correction, blending or tone parameters (from
  // any screen, Auto and Reset included) re-renders its mosaic live.
  // (Per tab: two projects can have areas of the same id.)
  $effect(() => {
    // Pixels from a server's bundle are final: nothing to re-render.
    if (app.remote) return;
    const liveKeys = app.liveKeys;
    for (const id of Object.keys(liveKeys)) if (!app.stitched[id]) delete liveKeys[id];
    for (const id of Object.keys(app.stitched)) {
      // First sight of a stitched area: compare with what the core used (a change made while
      // the stitch ran must not be lost).
      if (liveKeys[id] === undefined) {
        const used = app.project?.areas.find((a) => a.id === id)?.params;
        liveKeys[id] = used ? liveKey(used, used.tone, app.grains[id]) : '';
      }
      const key = liveKey(app.params[id], app.tone[id], app.grains[id]);
      if (key !== liveKeys[id]) {
        liveKeys[id] = key;
        liveChanged(id);
      }
    }
  });

  // Tell UPb Studio which spot is selected (a no-op when not hosted).
  let lastSelected = '';
  $effect(() => {
    if (!isHosted()) return;
    const id = app.selection.length === 1 ? app.selection[0] : null;
    const project = app.project?.uuid;
    const spot = id ? app.spots.find((s) => s.id === id) : null;
    if (!project || !spot) {
      lastSelected = '';
      return;
    }
    const via = app.shellSelection?.project === project && app.shellSelection.spot === spot.id ? 'shell' : 'user';
    const grain = spot.grain_number == null ? null : `${spot.area_id}/${spot.grain_number}`;
    const key = JSON.stringify([project, spot.id, spot.name, grain, spot.area_id, via]);
    if (key === lastSelected) return;
    lastSelected = key;
    void notify({ type: SHELL_EVENTS.spotSelected, project, spot: spot.id, name: spot.name, grain, area: spot.area_id, via });
  });

  // Theme: the system setting, and the shell's data-theme on <html> when hosted.
  function watchTheme() {
    const mq = matchMedia('(prefers-color-scheme: dark)');
    const sys = (e: MediaQueryListEvent) => (app.systemDark = e.matches);
    mq.addEventListener('change', sys);
    app.hosted = isHosted();
    let mo: MutationObserver | null = null;
    if (app.hosted) {
      const read = () => {
        const t = document.documentElement.getAttribute('data-theme');
        app.hostTheme = t === 'light' || t === 'dark' ? t : null;
      };
      read();
      mo = new MutationObserver(read);
      mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    }
    return () => {
      mq.removeEventListener('change', sys);
      mo?.disconnect();
    };
  }

  const desktop = isDesktop();
  /** Tab keys and the menu are the desktop app's: a browser and UPb Studio keep Cmd+T/W/1-9. */
  const tabKeys = desktop && !isHosted();
  let tabBar = $state<ReturnType<typeof TabBar>>();
  /** A folder dragged over the desktop window (it opens in a new tab). */
  let dropping = $state(false);

  // The window title names the active tab (desktop), and save dialogs start in its folder.
  $effect(() => {
    const title = `${app.title}${app.dirty && !app.readOnly ? ' •' : ''} — grainatlas`;
    if (!desktop) {
      document.title = tabs.list.length > 1 || app.pkg || app.remote ? title : 'grainatlas';
      return;
    }
    const id = tabs.activeId;
    void import('./lib/core/tauri').then((m) => m.setSaveTab(id));
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow().setTitle(title)).catch(() => {});
  });

  // Cmd+T / Cmd+W arrive as a keydown and, from the menu bar, as a menu event: act once.
  let lastTabCmd = { what: '', at: 0 };
  function tabCommand(what: 'new-tab' | 'close-tab') {
    const now = performance.now();
    if (lastTabCmd.what === what && now - lastTabCmd.at < 400) return;
    lastTabCmd = { what, at: now };
    if (what === 'new-tab') tabs.add();
    else tabBar?.requestClose(current());
  }

  /** Desktop start-up: `grainatlas A.grainatlas B.grainatlas … [--stitch] [--segment]` opens one tab per folder. */
  async function startup() {
    const { invoke } = await import('@tauri-apps/api/core');
    const s = await invoke<{ paths: string[]; stitch: boolean; segment: boolean; um_per_px: number | null; field_um: number | null }>('startup');
    if (!s.paths.length) return;
    const opened = await openPathsInTabs(s.paths);
    if (opened[0]) tabs.select(opened[0].id);
    for (const t of opened) {
      // Loose images with a scale on the command line open without the dialog.
      const pending = t.pendingLoose;
      if (pending && (s.um_per_px || s.field_um)) {
        await openLoose({ um_per_px: s.um_per_px ?? 0, ...(s.field_um ? { field_width_um: s.field_um } : {}), files: pending.images.map((i) => i.file) }, t);
      }
    }
    for (const t of opened) {
      if (!t.pkg) continue;
      // Areas the project renders from its saved placements are not stitched again.
      if (s.stitch) await stitchAll(t, true);
      if (s.segment) {
        await findAllGrains(t);
        t.screen = 'canvas';
      }
    }
  }

  onMount(() => {
    const unwatch = watchTheme();
    const undo: (() => void)[] = [];
    if (desktop) {
      void startup();
      // Dropped folders and files arrive as paths: a new tab unless this one is empty.
      void import('@tauri-apps/api/webview').then(({ getCurrentWebview }) =>
        getCurrentWebview()
          .onDragDropEvent((e) => {
            if (e.payload.type === 'over' || e.payload.type === 'enter') dropping = true;
            else if (e.payload.type === 'leave') dropping = false;
            else if (e.payload.type === 'drop') {
              dropping = false;
              if (e.payload.paths.length) void openPathsInTabs(e.payload.paths);
            }
          })
          .then((f) => undo.push(f)),
      );
      // Projects the OS opens (Finder double-click, Open With, the Dock icon): a tab each.
      const takeOpened = async () => {
        const { invoke } = await import('@tauri-apps/api/core');
        const paths = await invoke<string[]>('take_opened');
        if (paths.length) {
          const opened = await openPathsInTabs(paths);
          if (opened[0]) tabs.select(opened[0].id);
        }
      };
      void import('@tauri-apps/api/event').then(({ listen }) => listen('grainatlas://opened', () => void takeOpened()).then((f) => undo.push(f)));
      void takeOpened();
      if (tabKeys)
        void import('@tauri-apps/api/event').then(({ listen }) =>
          listen<string>('grainatlas://menu', (e) => (e.payload === 'new-tab' || e.payload === 'close-tab') && tabCommand(e.payload)).then((f) => undo.push(f)),
        );
    }
    // Files dropped on UPb Studio's window or opened through the OS (rule 4): the
    // bridge takes them at once (before the shell's 3 s fallback) and hands them
    // over as {path, file}; they open like a drop on the Open screen.
    const stopFiles = isHosted() ? onOpenFiles((set) => openOffered(set)) : () => {};
    const stopCommands = isHosted() ? onCommand(handleShellCommand) : () => {};
    fromHash();
    void startDeepLink();
    window.addEventListener('hashchange', fromHash);
    // Ctrl/Cmd 1-4, Ctrl/Cmd , and Ctrl/Cmd R belong to UPb Studio's menu when
    // hosted (bridge README): grainatlas binds Alt 1-5 and Ctrl/Cmd S only.
    const key = (e: KeyboardEvent) => {
      if (app.checkPrompt || app.renamePrompt) return;
      const mod = e.metaKey || e.ctrlKey;
      if (tabKeys && mod && !e.altKey) {
        const k = e.key.toLowerCase();
        if (k === 't' && !e.shiftKey) {
          e.preventDefault();
          tabCommand('new-tab');
          return;
        }
        if (k === 'w' && !e.shiftKey) {
          e.preventDefault();
          tabCommand('close-tab');
          return;
        }
        if (/^[1-9]$/.test(e.key)) {
          e.preventDefault();
          tabs.selectIndex(Number(e.key) - 1);
          return;
        }
      }
      // Ctrl+Tab / Ctrl+Shift+Tab: next / previous tab (desktop).
      if (tabKeys && e.ctrlKey && e.key === 'Tab') {
        e.preventDefault();
        const i = tabs.list.findIndex((x) => x.id === tabs.activeId);
        const n = tabs.list.length;
        tabs.select(tabs.list[(i + (e.shiftKey ? n - 1 : 1)) % n].id);
        return;
      }
      const t = e.target as HTMLElement;
      if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT') return;
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault();
        void saveProject();
        return;
      }
      if (e.altKey && /^[1-5]$/.test(e.key)) {
        const s = screens[Number(e.key) - 1];
        if (enabled(s)) app.screen = s.id;
      }
    };
    window.addEventListener('keydown', key);
    return () => {
      for (const f of undo) f();
      unwatch();
      stopFiles();
      stopCommands();
      window.removeEventListener('hashchange', fromHash);
      window.removeEventListener('keydown', key);
    };
  });
</script>

<div class="ga-root app" data-theme={app.theme}>
  <TabBar bind:this={tabBar} shortcuts={tabKeys} />
  {#key tabs.activeId}
  <header class="glass topbar">
    <div class="brand">
      <!-- The app icon (a light tile by design, the same in both themes). -->
      <img class="mark" src="{import.meta.env.BASE_URL}favicon.svg" width="22" height="22" alt="" />
      <span>grainatlas</span>
      {#if app.pkg}<span class="muted pkgname">{app.pkg.name}</span>{/if}
      {#if app.remote}
        <span class="muted pkgname" data-testid="remote-name" title="{app.remote.server}/atlas/projects/{app.remote.projectId}/">{app.remote.atlas.name} · {new URL(app.remote.server).host}{app.remote.readOnly ? ' · read-only' : ' · local copy'}</span>
      {/if}
    </div>
    <nav aria-label="Screens">
      {#each screens as s (s.id)}
        <button class="tab" aria-pressed={app.screen === s.id} disabled={!enabled(s)} onclick={() => (app.screen = s.id)} title="{s.label} (Alt+{s.key})">{s.label}</button>
      {/each}
    </nav>
    <div class="right">
      <ThemeSwitch />
      {#if app.readOnly}
        <button class="primary" onclick={editCopy} title="Make this published project a local, editable one (same project id)" data-testid="edit-copy">Edit a copy</button>
      {:else if app.project}
        <button onclick={() => saveProject()} title="Save project (Cmd/Ctrl+S): {app.projectPath ? app.projectPath.path : desktop ? `${app.project.name}.gaproj next to the package` : 'download the project file'}" data-testid="save-project">Save project</button>
        <button onclick={() => saveProjectFile()} title="Save a project file (.gaproj): parameters, placements, grains and spots, no pixels; it reopens with the package's images (double-click it)">Save as…</button>
      {/if}
    </div>
  </header>

  <main>
    {#if app.screen === 'open'}
      <Open />
    {:else if app.screen === 'area'}
      <Area />
    {:else if app.screen === 'canvas'}
      <Canvas />
    {:else if app.screen === 'layout'}
      <Layout />
    {:else}
      <Settings />
    {/if}
  </main>

  {#if app.checkPrompt}<CheckDialog />{/if}
  {#if app.renamePrompt}<RenameDialog />{/if}
  {#if app.offer}<ReplaceDialog />{/if}
  {#if app.pendingLoose}<ScaleDialog />{/if}
  {#if app.busy}<Busy />{/if}
  {#if app.notice}
    <div class="glass toast ok" role="status">
      <span>{app.notice}</span>
      <button class="icon" onclick={() => (app.notice = null)} aria-label="Dismiss">✕</button>
    </div>
  {/if}
  {#if app.error}
    <div class="glass toast" role="alert">
      <span>{app.error}</span>
      <button class="icon" onclick={() => (app.error = null)} aria-label="Dismiss">✕</button>
    </div>
  {/if}
  {/key}
  {#if dropping}
    <div class="dropzone" aria-hidden="true"><span class="glass">Drop to open{app.empty ? '' : ' in a new tab'}</span></div>
  {/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto auto 1fr;
    height: 100vh;
    overflow: hidden;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin: 0 var(--space-2) var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    z-index: 5;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 650;
  }
  .mark {
    display: block;
    flex: none;
  }
  .pkgname {
    font-weight: 400;
    margin-left: var(--space-2);
  }
  nav {
    display: flex;
    gap: var(--space-1);
    margin-left: var(--space-4);
  }
  .tab {
    border-color: transparent;
    background: transparent;
  }
  .right {
    margin-left: auto;
    display: flex;
    gap: var(--space-2);
  }
  main {
    position: relative;
    min-height: 0;
    overflow: hidden;
  }
  .dropzone {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    border: 3px dashed var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    z-index: 70;
    pointer-events: none;
  }
  .dropzone span {
    padding: var(--space-3) var(--space-4);
    font-weight: 600;
  }
  .toast {
    position: fixed;
    bottom: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: var(--space-4);
    align-items: center;
    padding: var(--space-2) var(--space-4);
    color: var(--danger);
    z-index: 50;
    max-width: 80vw;
  }
  .toast.ok {
    color: var(--ok);
  }
</style>
