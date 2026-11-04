<script lang="ts">
  import { onMount } from 'svelte';
  import { app, type Screen } from './lib/store.svelte';
  import { isHosted, notify } from '@lab/bridge';
  import { liveChanged, liveKey, saveProject, startDeepLink } from './lib/actions';
  import Open from './screens/Open.svelte';
  import Area from './screens/Area.svelte';
  import Canvas from './screens/Canvas.svelte';
  import Layout from './screens/Layout.svelte';
  import Settings from './screens/Settings.svelte';
  import Busy from './components/Busy.svelte';
  import ScaleDialog from './components/ScaleDialog.svelte';
  import ThemeSwitch from './components/ThemeSwitch.svelte';
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
  const liveKeys: Record<string, string> = {};
  $effect(() => {
    // Pixels from a server's bundle are final: nothing to re-render.
    if (app.remote) return;
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
    const id = app.selection.length === 1 ? app.selection[0] : null;
    const project = app.project?.uuid;
    const spot = id ? app.spots.find((s) => s.id === id) : null;
    if (!project || !spot) {
      lastSelected = '';
      return;
    }
    const key = `${project}/${spot.id}`;
    if (key === lastSelected) return;
    lastSelected = key;
    void notify({ type: 'grainatlas.spot.selected', project, spot: spot.id, name: spot.name });
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

  onMount(() => {
    const unwatch = watchTheme();
    fromHash();
    void startDeepLink();
    window.addEventListener('hashchange', fromHash);
    // Ctrl/Cmd 1-4, Ctrl/Cmd , and Ctrl/Cmd R belong to UPb Studio's menu when
    // hosted (bridge README): grainatlas binds Alt 1-5 and Ctrl/Cmd S only.
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT') return;
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault();
        saveProject();
        return;
      }
      if (e.altKey && /^[1-5]$/.test(e.key)) {
        const s = screens[Number(e.key) - 1];
        if (enabled(s)) app.screen = s.id;
      }
    };
    window.addEventListener('keydown', key);
    return () => {
      unwatch();
      window.removeEventListener('hashchange', fromHash);
      window.removeEventListener('keydown', key);
    };
  });
</script>

<div class="ga-root app" data-theme={app.theme}>
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
        <button onclick={saveProject} title="Save project (Cmd/Ctrl+S)">Save project</button>
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
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
    overflow: hidden;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin: var(--space-2);
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
