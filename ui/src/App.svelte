<script lang="ts">
  import { onMount } from 'svelte';
  import { app, type Screen } from './lib/store.svelte';
  import { isHosted, notify } from '@lab/bridge';
  import { saveProject, startDeepLink } from './lib/actions';
  import Open from './screens/Open.svelte';
  import Area from './screens/Area.svelte';
  import Canvas from './screens/Canvas.svelte';
  import Layout from './screens/Layout.svelte';
  import Settings from './screens/Settings.svelte';
  import Busy from './components/Busy.svelte';
  import ScaleDialog from './components/ScaleDialog.svelte';
  import ThemeSwitch from './components/ThemeSwitch.svelte';

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
      <svg viewBox="0 0 32 32" width="22" height="22" aria-hidden="true"><path d="M10 8l5-2 4 3-1 6-5 3-4-3z" fill="currentColor" /><path d="M19 17l4-1 3 4-2 5-4 0-2-4z" fill="currentColor" opacity="0.6" /><circle cx="13" cy="11" r="2" fill="none" stroke="var(--accent)" stroke-width="1.4" /></svg>
      <span>grainatlas</span>
      {#if app.pkg}<span class="muted pkgname">{app.pkg.name}</span>{/if}
    </div>
    <nav aria-label="Screens">
      {#each screens as s (s.id)}
        <button class="tab" aria-pressed={app.screen === s.id} disabled={!enabled(s)} onclick={() => (app.screen = s.id)} title="{s.label} (Alt+{s.key})">{s.label}</button>
      {/each}
    </nav>
    <div class="right">
      <ThemeSwitch />
      {#if app.project}
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
