<script lang="ts">
  import { bundlePixels } from './lib/bundle-tone';
  import { loadPublishedPixels } from './lib/remote';
  import { screenEnabled } from './lib/screens';
  import { grainNumber } from './lib/grain-lineage';
  import { onMount, tick, untrack } from 'svelte';
  import { authToken } from './lib/atlas';
  import { oversightSession } from './lib/oversight';
  import { oversight, onOversight } from '@lab/bridge';
  import { presentation } from './lib/presentation';
  import { appStorage } from './lib/app-storage';
  import VersionChip from './components/VersionChip.svelte';
  import { startScheduler } from './lib/updater.svelte';
  import { app, current, tabs, type AppState, type Screen } from './lib/store.svelte';
  import { isHosted, notify, onCommand, onOpenFiles, SHELL_EVENTS } from '@lab/bridge';
  import { findAllGrains, liveChanged, liveIdle, liveKey, openLoose, openOffered, openPathsInTabs, saveProject, saveProjectFile, startDeepLink, stitchAll } from './lib/actions';
  import { isDesktop } from './lib/core/core';
  import TabBar from './components/TabBar.svelte';
  import Open from './screens/Open.svelte';
  import Area from './screens/Area.svelte';
  import Canvas from './screens/Canvas.svelte';
  import Layout from './screens/Layout.svelte';
  import Settings from './screens/Settings.svelte';
  import Busy from './components/Busy.svelte';
  import PixelsLoading from './components/PixelsLoading.svelte';
  import ScaleDialog from './components/ScaleDialog.svelte';
  import SyncChip from './components/SyncChip.svelte';
  import SyncDetails from './components/SyncDetails.svelte';
  import LeaveEditDialog from './components/LeaveEditDialog.svelte';
  import QuitDialog from './components/QuitDialog.svelte';
  import CheckDialog from './components/CheckDialog.svelte';
  import RenameDialog from './components/RenameDialog.svelte';
  import ReplaceDialog from './components/ReplaceDialog.svelte';
  import StoredCopyDialog from './components/StoredCopyDialog.svelte';
  import ThemeSwitch from './components/ThemeSwitch.svelte';
  import CheckUpdatesButton from './components/CheckUpdatesButton.svelte';
  import { handleShellCommand } from './lib/shellcmd';
  import { hostedSync } from './lib/hosted-sync';
  import { reportHostedStatus } from './lib/hosted-actions';
  import { DRAFTS_CHANGED, listDrafts } from './lib/drafts';

  const isViewer = $derived(presentation.current === 'viewer');
  onMount(() => presentation.beforeViewer(async () => {
    await tick(); // let queued editor persistence run before closing the gate
    for (const tab of tabs.list) {
      while (tab.busy) await new Promise(resolve => setTimeout(resolve, 20));
      for (const area of Object.keys(tab.live)) await liveIdle(area, tab);
      await tab.sync.settled();
    }
    await appStorage.flushed();
  }));

  const screens: { id: Screen; label: string; key: string; needs?: 'pkg' | 'mosaic' }[] = [
    { id: 'open', label: 'Open', key: '1' },
    { id: 'area', label: 'Area', key: '2', needs: 'pkg' },
    { id: 'canvas', label: 'Canvas', key: '3', needs: 'mosaic' },
    { id: 'layout', label: 'Layout', key: '4', needs: 'mosaic' },
    { id: 'settings', label: 'Settings', key: '5' },
  ];

  $effect(() => { if (isHosted()) reportHostedStatus(app); });
  $effect(() => { if (app.project?.uuid) appStorage.reportAvailability(app); });

  const menuCollapsed = $derived(app.screen === 'canvas' && app.layout.menuCollapsed);
  function toggleMenu() {
    app.layout.menuCollapsed = !app.layout.menuCollapsed;
    app.saveLayout();
  }

  const desktop = isDesktop();
  let syncDetailsOpen = $state(false);
  let overflowOpen = $state(false);
  /** A storage warning the user hid: it then waits behind the header's Storage full button. */
  let hiddenStorageWarning = $state<string | null>(null);
  let narrowHeader = $state(false);
  let overflowButton = $state<HTMLButtonElement>();
  let overflowPanel = $state<HTMLDivElement>();
  const projectName = $derived(app.project?.name ?? app.remote?.atlas.name ?? app.pkg?.name ?? 'GrainAtlas');
  const saveTitle = $derived(app.sync.canSyncOnSave
    ? 'Save project and sync to the database (Cmd/Ctrl+S). Turn off “Sync to the database when saving” in Settings.'
    : `Save project (Cmd/Ctrl+S): ${app.projectPath?.path ?? (desktop ? `${projectName}.gaproj next to the package` : 'download the project file')}`);
  onMount(() => {
    const query = matchMedia('(max-width: 1280px)');
    const update = () => narrowHeader = query.matches;
    update(); query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  });
  $effect(() => { tabs.activeId; app.screen; menuCollapsed; syncDetailsOpen = false; overflowOpen = false; });
  $effect(() => {
    const tab = current(), notice = tab.notice;
    if (!notice || tab.noticeAction) return;
    const timer = setTimeout(() => { if (tab.notice === notice && !tab.noticeAction) tab.notice = null; }, 6000);
    return () => clearTimeout(timer);
  });
  function publishNoticeBottom(notices: HTMLDivElement) {
    const main = notices.closest('main')!;
    function update() {
      main.style.setProperty('--notice-bottom', `${notices.offsetTop + notices.offsetHeight}px`);
    }
    const resize = new ResizeObserver(update);
    resize.observe(notices);
    update();
    return { destroy() {
      resize.disconnect();
      main.style.removeProperty('--notice-bottom');
    } };
  }
  function closeHeaderOutside(event: PointerEvent) {
    if (!(event.target instanceof Element)) return;
    if (!overflowButton?.contains(event.target) && !overflowPanel?.contains(event.target)) overflowOpen = false;
  }
  // The details drawer is in flow: closing it moves everything below it up. Close after the
  // click has reached its target (not on pointerdown), so the click lands where it was aimed.
  function closeDetailsOutside(event: MouseEvent) {
    if (event.target instanceof Element && !event.target.closest('[data-testid="sync-chip"], [data-testid="sync-details"]')) syncDetailsOpen = false;
  }
  function headerKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (!syncDetailsOpen && !overflowOpen)) return;
    event.preventDefault(); event.stopPropagation();
    if (overflowOpen) overflowButton?.focus();
    else document.querySelector<HTMLButtonElement>('[data-testid="sync-chip"]')?.focus();
    syncDetailsOpen = false; overflowOpen = false;
  }

  const hasMosaic = $derived(Object.keys(app.stitched).length > 0);
  function enabled(s: (typeof screens)[number]) {
    return screenEnabled(s.id, { viewer: isViewer, readOnly: app.readOnly, remote: !!app.remote, pkg: !!app.pkg, mosaic: hasMosaic });
  }

  $effect(() => {
    if (app.remote && !app.readOnly && !app.bundle && (app.screen === 'area' || app.screen === 'layout')) {
      const state = current();
      untrack(() => { if (!state.pixelsLoad) void loadPublishedPixels(state); });
    }
    if (!app.remote || app.readOnly) {
      const state = current();
      untrack(() => { state.cancelPixelsLoad?.(); state.pixelsLoad = null; });
    }
    if (app.remote && app.readOnly && (app.screen === 'area' || app.screen === 'layout')) app.screen = 'canvas';
  });

  // Hash routing (#/canvas) so the build works from file://, any static host and Tauri.
  function fromHash() {
    if (isViewer) return;
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
    if (app.readOnly || (app.remote && !bundlePixels(app))) return;
    const liveKeys = app.liveKeys;
    for (const id of Object.keys(liveKeys)) if (!app.stitched[id]) delete liveKeys[id];
    for (const id of Object.keys(app.stitched)) {
      // First sight of a stitched area: compare with what the core used (a change made while
      // the stitch ran must not be lost).
      if (liveKeys[id] === undefined) {
        const used = app.project?.areas.find((a) => a.id === id)?.params;
        liveKeys[id] = app.bundle?.[id] ? liveKey(app.params[id], app.tone[id], app.grains[id]) : used ? liveKey(used, used.tone, app.grains[id]) : '';
      }
      const key = liveKey(app.params[id], app.tone[id], app.grains[id]);
      if (key !== liveKeys[id]) {
        liveKeys[id] = key;
        liveChanged(id);
      }
    }
  });

  const bundleToneKeys = new WeakMap<AppState, string>();
  $effect(() => {
    if (!bundlePixels(app)) return;
    const state = current();
    const key = JSON.stringify(app.tone);
    const before = bundleToneKeys.get(state);
    bundleToneKeys.set(state, key);
    if (before !== undefined && key !== before) untrack(() => state.persist());
  });

  // Tell UPb Lab which spot is selected (a no-op when not hosted).
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
    // The protocol's grain is "<area>/<number>" (with a split suffix, e.g. 12a), unique after splits.
    const g = spot.grain_id ? app.grains[spot.area_id]?.find((q) => q.id === spot.grain_id) : null;
    const grain = g ? `${spot.area_id}/${grainNumber(g)}` : spot.grain_number == null ? null : `${spot.area_id}/${spot.grain_number}`;
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

  /** Tab keys and the menu are the desktop app's: a browser and UPb Lab keep Cmd+T/W/1-9. */
  const tabKeys = desktop && !isHosted();
  let tabBar = $state<ReturnType<typeof TabBar>>();
  /** A folder dragged over the desktop window (it opens in a new tab). */
  let dropping = $state(false);

  let profile = $state<string | null>(null);

  // The window title names the active tab (desktop), and save dialogs start in its folder.
  $effect(() => {
    const title = `${app.title}${app.dirty && !app.readOnly ? ' •' : ''} — grainatlas`;
    if (!desktop) {
      document.title = tabs.list.length > 1 || app.pkg || app.remote ? title : 'grainatlas';
      return;
    }
    const id = tabs.activeId;
    const windowTitle = title + (profile ? ` (${profile})` : '');
    void import('./lib/core/tauri').then((m) => m.setSaveTab(id));
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow().setTitle(windowTitle)).catch(() => {});
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
    const s = await invoke<{ paths: string[]; stitch: boolean; segment: boolean; um_per_px: number | null; field_um: number | null; profile: string | null }>('startup');
    profile = s.profile;
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

  let quitState = $state<'idle' | 'prompt' | 'saving' | 'exiting'>('idle');
  const unsavedTabs = $derived(tabs.list.filter((tab) => (tab.dirty || tab.sync.unsynced) && !tab.readOnly));

  async function finishQuit() {
    quitState = 'exiting';
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('finish_quit');
    } catch (e) {
      quitState = 'idle';
      app.error = `Could not quit: ${e instanceof Error ? e.message : e}`;
    }
  }

  function requestQuit() {
    if (quitState !== 'idle') return;
    if (unsavedTabs.length) quitState = 'prompt';
    else void finishQuit();
  }

  async function answerQuit(answer: 'save' | 'discard' | 'cancel') {
    if (quitState !== 'prompt') return;
    if (answer === 'cancel') {
      quitState = 'idle';
      return;
    }
    if (answer === 'discard') {
      await finishQuit();
      return;
    }

    quitState = 'saving';
    const previous = tabs.activeId;
    try {
      for (const tab of [...unsavedTabs]) {
        if (!tab.dirty && !tab.sync.unsynced) continue;
        // Save can show the project's checks dialog before writing.
        tabs.select(tab.id);
        await saveProject(tab);
        if (tab.dirty || tab.sync.unsynced) {
          if (!tab.error) tab.notice = tab.dirty ? 'Project was not saved. GrainAtlas is still open.' : 'Changes are still not in the database. GrainAtlas is still open.';
          return;
        }
      }
      if (unsavedTabs.length) {
        quitState = 'prompt';
        return;
      }
      await finishQuit();
    } catch (e) {
      app.error = `Could not save: ${e instanceof Error ? e.message : e}`;
    } finally {
      if (quitState === 'saving') quitState = 'idle';
      else if (quitState === 'prompt') tabs.select(previous);
    }
  }

  onMount(() => {
    const unwatch = watchTheme();
    const undo: (() => void)[] = [];
    undo.push(startScheduler());
    const authWatch = setInterval(() => void authToken().catch(() => {}), 5000);
    undo.push(() => clearInterval(authWatch));
    if (isHosted()) {
      let received = false;
      undo.push(onOversight(value => { received = true; oversightSession.set(value); }));
      void oversight().then(value => { if (!received) oversightSession.set(value); }).catch(() => {});
    }
    if (desktop) {
      if (!isHosted()) {
        void (async () => {
          const { listen } = await import('@tauri-apps/api/event');
          const { invoke } = await import('@tauri-apps/api/core');
          undo.push(await listen('grainatlas://quit-requested', requestQuit));
          await invoke('enable_quit_guard');
        })().catch((e) => {
          app.error = `Could not enable unsaved changes warnings: ${e instanceof Error ? e.message : e}`;
        });
      }
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
      // Drain once now and again once the listener is in place: an OS open that lands between the
      // two (at launch, macOS sends the paths as an Apple event, never in argv) stays queued in the shell.
      void import('@tauri-apps/api/event').then(({ listen }) => listen('grainatlas://opened', () => void takeOpened()).then((f) => { undo.push(f); void takeOpened(); }));
      void takeOpened();
      void (async () => {
        const { listen } = await import('@tauri-apps/api/event');
        const { invoke } = await import('@tauri-apps/api/core');
        const { openStandaloneLaunch } = await import('./lib/standalone-launch');
        let queue = Promise.resolve();
        undo.push(await listen<import('./lib/standalone-launch').StandaloneLaunch>('grainatlas://launch', e => { queue = queue.then(() => openStandaloneLaunch(e.payload)); }));
        await invoke('atlas_launch_ready');
      })();
      if (tabKeys)
        void import('@tauri-apps/api/event').then(({ listen }) =>
          listen<string>('grainatlas://menu', (e) => { if (e.payload === 'new-tab' || e.payload === 'close-tab') tabCommand(e.payload); else if (e.payload === 'export-grains' && !current().busy) { current().screen = 'canvas'; current().grainExportPrompt = true; } else if (e.payload === 'export-mosaic' && !current().busy) { current().screen = 'canvas'; current().mosaicExportPrompt = true; } }).then((f) => undo.push(f)),
        );
    }
    // Files dropped on UPb Lab's window or opened through the OS (rule 4): the
    // bridge takes them at once (before the shell's 3 s fallback) and hands them
    // over as {path, file}; they open like a drop on the Open screen.
    const stopFiles = isHosted() ? onOpenFiles((set) => openOffered(set)) : () => {};
    const reportDrafts = () => { if (isHosted()) hostedSync.drafts(listDrafts(app.server)); };
    reportDrafts();
    window.addEventListener(DRAFTS_CHANGED, reportDrafts);
    undo.push(() => window.removeEventListener(DRAFTS_CHANGED, reportDrafts));
    const stopCommands = isHosted() ? onCommand(handleShellCommand) : () => {};
    fromHash();
    void startDeepLink();
    window.addEventListener('hashchange', fromHash);
    // Ctrl/Cmd 1-4, Ctrl/Cmd , and Ctrl/Cmd R belong to UPb Lab's menu when
    // hosted (bridge README): grainatlas binds Alt 1-5 and Ctrl/Cmd S only.
    const key = (e: KeyboardEvent) => {
      if (isViewer) return;
      if (quitState !== 'idle' || app.checkPrompt || app.renamePrompt) return;
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

  onMount(() => {
    const retry = () => { for (const tab of tabs.list) if (tab.project && !tab.sync.wakeAutomatic()) void tab.sync.check(); };
    const focused = () => { if (!isHosted()) retry(); };
    const timer = window.setInterval(() => { for (const tab of tabs.list) if (tab.project && !tab.sync.backgroundPending) void tab.sync.check(); }, 30_000);
    window.addEventListener('online', retry);
    window.addEventListener('focus', focused);
    const closing = (event: BeforeUnloadEvent) => {
      if (tabs.list.some(tab => !tab.readOnly && (tab.dirty || tab.sync.unsynced))) { event.preventDefault(); event.returnValue = ''; }
    };
    window.addEventListener('beforeunload', closing);
    return () => { clearInterval(timer); window.removeEventListener('online', retry); window.removeEventListener('focus', focused); window.removeEventListener('beforeunload', closing); };
  });
</script>

<svelte:window onpointerdown={closeHeaderOutside} onclick={closeDetailsOutside} onkeydowncapture={headerKey} />

{#if !isViewer && app.leaveEditPrompt}<LeaveEditDialog onchoose={app.leaveEditPrompt.answer} />{/if}

<div class="ga-root app" class:viewer={isViewer} data-theme={app.theme}>
  {#if !isViewer}<div class="project-tabs" hidden={menuCollapsed}><TabBar bind:this={tabBar} shortcuts={tabKeys} /></div>{/if}
  {#key tabs.activeId}
  {#if !isViewer}<div class="glass header-region">
  <header class="topbar" class:compact={menuCollapsed} class:has-menu-toggle={app.screen === 'canvas'}>
    {#if menuCollapsed}
      <span class="compact-name" data-testid="project-name" title={projectName}>{projectName}</span>
    {:else}
    <div class="brand">
      <!-- The app icon (a light tile by design, the same in both themes). -->
      <img class="mark" src="{import.meta.env.BASE_URL}favicon.svg" width="22" height="22" alt="" />
      <span class="brand-label">grainatlas</span>
      {#if app.project}<span class="muted pkgname" data-testid="project-name" title={projectName}>{projectName}</span>{/if}
    </div>
    <nav aria-label="Screens">
      {#each screens as s (s.id)}
        <button class="tab" aria-pressed={app.screen === s.id} disabled={!enabled(s)} onclick={() => (app.screen = s.id)} title="{s.label} (Alt+{s.key})">{s.label}</button>
      {/each}
    </nav>
    <SyncChip bind:detailsOpen={syncDetailsOpen} />
    <div class="right">
      <CheckUpdatesButton />
      {#if !narrowHeader}<ThemeSwitch />{/if}
      {#if app.storageWarning && hiddenStorageWarning === app.storageWarning}
        <button class="danger" onclick={() => (hiddenStorageWarning = null)} title={app.storageWarning} data-testid="storage-full">Storage full</button>
      {/if}
      {#if app.project && !app.readOnly}
        <button onclick={() => saveProject()} title={saveTitle} data-testid="save-project">{app.sync.canSyncOnSave ? 'Save and sync' : 'Save project'}</button>
      {/if}
      <button class="icon overflow-toggle" bind:this={overflowButton} data-testid="header-overflow" aria-label="More project actions" aria-expanded={overflowOpen} aria-controls={overflowOpen ? 'header-overflow-panel' : undefined} onclick={() => overflowOpen = !overflowOpen}>…</button>
    </div>
    {/if}
    {#if menuCollapsed}<SyncChip bind:detailsOpen={syncDetailsOpen} />{/if}
    {#if app.screen === 'canvas'}
      <button class="menu-toggle" aria-expanded={!menuCollapsed} onclick={toggleMenu}
        aria-label={menuCollapsed ? 'Expand GrainAtlas menu' : 'Collapse GrainAtlas menu'}
        title={menuCollapsed ? 'Expand the menu and project tabs' : 'Collapse the menu and project tabs'}>
        <svg viewBox="0 0 14 14" width="14" height="14" aria-hidden="true">
          <path d={menuCollapsed ? 'm3 5 4 4 4-4' : 'm3 9 4-4 4 4'} fill="none"
            stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
    {/if}
  </header>
  {#if overflowOpen && !menuCollapsed}
    <div class="panel header-overflow" id="header-overflow-panel" bind:this={overflowPanel} data-testid="header-overflow-panel">
      {#if app.project && !app.readOnly}<button onclick={() => { overflowOpen = false; void saveProjectFile(); }} title="Save a project file (.gaproj): parameters, placements, grains and spots, no pixels">Save as…</button>{/if}
      {#if narrowHeader}<ThemeSwitch />{/if}
      {#if app.remote}<span class="faint small" data-testid="remote-name" title="{app.remote.server}/atlas/projects/{app.remote.projectId}/">{new URL(app.remote.server).host} · {app.remote.readOnly ? 'read-only' : 'local copy'}</span>{/if}
    </div>
  {/if}
  {#if syncDetailsOpen && app.project}<SyncDetails />{/if}
  </div>{/if}

  <main class:layout-screen={app.screen === 'layout'} class:has-oversight={oversightSession.readOnly || !!oversightSession.owner(app.server, app.project?.uuid)}>
    {#if oversightSession.state?.viewAs}
      <div class="glass panel session-banner" role="status" data-testid="view-as-banner">Viewing as {oversightSession.state.viewAs.name} (read-only)</div>
    {:else if oversightSession.owner(app.server, app.project?.uuid)}
      <div class="glass panel session-banner" role="status" data-testid="on-behalf-banner">Editing on behalf of {oversightSession.owner(app.server, app.project?.uuid)}</div>
    {/if}
    {#if !isViewer && (app.notice || app.error || (app.storageWarning && hiddenStorageWarning !== app.storageWarning))}
      <div class="notices" use:publishNoticeBottom>
        {#if app.storageWarning}
          {#if hiddenStorageWarning !== app.storageWarning}
            <div class="glass toast" role="alert" data-testid="storage-warning"><span>{app.storageWarning}</span><button onclick={() => saveProject(app)}>Save project</button><button onclick={() => (hiddenStorageWarning = app.storageWarning)} data-testid="storage-warning-hide">Hide</button></div>
          {/if}
        {/if}
        {#if app.notice}
          <div class="glass toast ok" role="status">
            <span>{app.notice}</span>
            {#if app.noticeAction}<button onclick={() => { const action = app.noticeAction; app.notice = null; action?.run(); }}>{app.noticeAction.label}</button>{/if}
            <button class="icon" onclick={() => app.notice = null} aria-label="Dismiss">✕</button>
          </div>
        {/if}
        {#if app.error}
          <div class="glass toast" role="alert"><span>{app.error}</span><button class="icon" onclick={() => app.error = null} aria-label="Dismiss">✕</button></div>
        {/if}
      </div>
    {/if}
    <div class="screen" class:pixels-loading={!!app.remote && !app.readOnly && !!app.pixelsLoad && (app.screen === 'area' || app.screen === 'layout')}>
    {#if app.remote && !app.readOnly && app.pixelsLoad && (app.screen === 'area' || app.screen === 'layout')}<PixelsLoading />{/if}
    {#if isViewer}
      {#if hasMosaic}<Canvas />{:else}<p class="faint small">{app.error || app.notice || 'Waiting for a project.'}</p>{/if}
    {:else if app.screen === 'open'}
      <Open />
    {:else if app.screen === 'area'}
      <Area />
    {:else if app.screen === 'canvas'}
      <Canvas />
    {:else if app.screen === 'layout' && (!app.remote || app.bundle)}
      <Layout />
    {:else if app.screen === 'settings'}
      <Settings />
    {/if}
    </div>
  </main>

  {#if !isViewer}
  {#if quitState === 'prompt'}
    <QuitDialog titles={unsavedTabs.map((tab) => tab.title)} answer={answerQuit} />
  {/if}
  {#if app.checkPrompt}<CheckDialog />{/if}
  {#if app.renamePrompt}<RenameDialog />{/if}
  {#if app.offer}<ReplaceDialog />{/if}
  {#if app.storedCopy?.ready}<StoredCopyDialog />{/if}
  {#if app.pendingLoose}<ScaleDialog />{/if}
  {/if}
  {#if app.busy}<Busy task={app.busy} />{/if}
  {/key}
  {#if !isViewer}<VersionChip />{/if}
  {#if dropping}
    <div class="dropzone" aria-hidden="true"><span class="glass">Drop to open{app.empty ? '' : ' in a new tab'}</span></div>
  {/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto auto 1fr;
    height: 100vh;
    overflow: hidden;
    container-type:inline-size;
  }
  .app.viewer { grid-template-rows: 0 0 minmax(0, 1fr); }
  .header-region { grid-row:2; min-width:0; margin:0 var(--space-2) var(--space-2); z-index:5; }
  .header-overflow { display:flex; align-items:center; justify-content:flex-end; gap:var(--space-3); border-top:1px solid var(--hairline); }
  .header-overflow > span { min-width:0; overflow-wrap:anywhere; font-size:0.8rem; }
  .topbar {
    position: relative;
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    min-width: 0;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    z-index: 5;
  }
  .project-tabs {min-width:0;}
  .topbar.has-menu-toggle:not(.compact) {padding-inline-end: 3rem;}
  .topbar.compact {
    margin-bottom: 0;
    padding: 2px var(--space-2);
    flex-wrap: nowrap;
    font-size: 0.75rem;
    gap: var(--space-2);
  }
  button.menu-toggle {
    position: absolute;
    inset-inline-end: var(--space-2);
    top: 50%;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    flex: none;
    width: 28px;
    height: 28px;
    padding: 0;
  }
  .compact .menu-toggle {
    position: static;
    transform: none;
    margin-inline-start: auto;
    width: 24px;
    height: 20px;
  }
  .compact-name {overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:28ch;flex:none;}
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 650;
    min-width:0;
    flex:0 0 auto;
  }
  .mark {
    display: block;
    flex: none;
  }
  .pkgname {
    font-weight: 400;
    max-width:28ch;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap;
  }
  nav {
    display: flex;
    flex:none;
    gap: var(--space-1);
    margin-left: 0;
  }
  .tab {
    border-color: transparent;
    background: transparent;
  }
  .right {
    margin-left: auto;
    display: flex;
    gap: var(--space-2);
    flex-wrap:nowrap;
    flex:none;
    min-width:0;
  }
  .topbar :global(button:not(.menu-toggle)) { height:2rem; white-space:nowrap; }
  .topbar :global(.seg) { height:2rem; padding:0; align-items:center; }
  .topbar :global(.seg button) { height:calc(2rem - 2px); }
  @container (max-width: 1100px) {
    .brand-label, .mark { display:none; }
    .topbar { gap:var(--space-1); padding-inline:var(--space-2); }
    .tab { padding-inline:var(--space-1); font-size:0.8rem; }
    .right > button { padding-inline:var(--space-2); font-size:0.8rem; }
    nav { gap:0; }
  }
  @container (max-width: 1000px) {
    .pkgname, .compact-name { max-width:22ch; }
  }
  main {
    position:relative;
    grid-row:3;
    display:grid;
    grid-template-rows:minmax(0,1fr);
    min-height:0;
    overflow:hidden;
  }
  main.has-oversight { grid-template-rows:auto minmax(0, 1fr); }
  main.layout-screen { display:flex; flex-direction:column; }
  .layout-screen .screen { flex:1; }
  .layout-screen .session-banner { flex:none; }
  .layout-screen .notices {
    position:relative;
    flex:none;
    inset:auto;
    width:auto;
    max-width:none;
    max-height:25vh;
    margin:0 var(--space-2) var(--space-2);
  }
  .session-banner { padding:var(--space-2) var(--space-4); margin:0 var(--space-2) var(--space-2); border-color:var(--accent); }
  .screen { position:relative; min-height:0; min-width:0; overflow:hidden; }
  .screen.pixels-loading { display:grid; grid-template-rows:auto minmax(0,1fr); }
  .notices {
    position:absolute;
    z-index:30;
    top:calc(var(--canvas-toolbar-bottom, 0px) + var(--space-2));
    left:calc(var(--canvas-notice-left, 0px) + var(--space-2));
    right:calc(var(--canvas-notice-right, 0px) + var(--space-2));
    width:calc(100% - var(--canvas-notice-left, 0px) - var(--canvas-notice-right, 0px) - 2 * var(--space-2));
    max-width:640px;
    margin-inline:auto;
    display:grid;
    gap:var(--space-1);
    max-height:min(25vh, calc(100% - var(--canvas-toolbar-bottom, 0px) - var(--canvas-notice-bottom, 0px) - 2 * var(--space-2)));
    overflow:auto;
    pointer-events:none;
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
    pointer-events:auto;
    display:flex;
    gap:var(--space-4);
    align-items:center;
    padding:var(--space-2) var(--space-4);
    color:var(--danger);
    min-width:0;
  }
  .toast button { flex-shrink: 0; }
  .toast span { min-width: 0; overflow-wrap: anywhere; }
  .toast.ok {
    color: var(--ok);
  }
</style>
