<script lang="ts">
  // The window's open projects, as browser-style tabs: click to switch, × or middle-click to
  // close (asks first when the tab has unsaved changes), drag to reorder, + for a new tab.
  import { tabs, type AppState } from '../lib/store.svelte';
  import { saveProject } from '../lib/actions';
  import { flushWrites } from '../lib/deferred-writes';

  let { shortcuts = false }: { shortcuts?: boolean } = $props();

  /** The tab waiting for an answer in the close dialog. */
  let asking = $state<AppState | null>(null);
  let strip: HTMLDivElement;
  let drag = $state<{ id: number; x0: number; moved: boolean; over: number } | null>(null);

  /** Closes a tab; with unsaved changes the dialog asks first. */
  export function requestClose(t: AppState) {
    flushWrites();
    if ((t.dirty || t.sync.unsynced) && !t.readOnly) {
      tabs.select(t.id);
      asking = t;
      return;
    }
    tabs.remove(t.id);
  }

  async function answer(a: 'save' | 'discard' | 'cancel') {
    const t = asking;
    asking = null;
    if (!t || a === 'cancel') return;
    if (a === 'save') {
      await saveProject(t);
      if (t.dirty || t.sync.unsynced) {
        if (!t.dirty) t.notice = 'Saved here. Changes are still not in the database; sync or close with the local draft kept.';
        return;
      }
    }
    tabs.remove(t.id);
  }

  function down(e: PointerEvent, t: AppState) {
    if (e.button === 1) {
      e.preventDefault();
      requestClose(t);
      return;
    }
    if (e.button !== 0 || (e.target as HTMLElement).closest('.close')) return;
    tabs.select(t.id);
    drag = { id: t.id, x0: e.clientX, moved: false, over: tabs.list.indexOf(t) };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function move(e: PointerEvent) {
    if (!drag) return;
    if (!drag.moved && Math.abs(e.clientX - drag.x0) < 5) return;
    drag.moved = true;
    const els = Array.from(strip.querySelectorAll<HTMLElement>('[data-tab]'));
    let over = els.length - 1;
    for (let i = 0; i < els.length; i++) {
      const r = els[i].getBoundingClientRect();
      if (e.clientX < r.left + r.width / 2) {
        over = i;
        break;
      }
    }
    drag.over = over;
  }

  function up() {
    if (drag?.moved) tabs.move(drag.id, drag.over);
    drag = null;
  }

  function key(e: KeyboardEvent, t: AppState) {
    const i = tabs.list.indexOf(t);
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      const j = i + (e.key === 'ArrowRight' ? 1 : -1);
      if (e.shiftKey) tabs.move(t.id, j);
      else if (tabs.list[j]) tabs.select(tabs.list[j].id);
      strip.querySelector<HTMLElement>(`[data-tab="${tabs.activeId}"]`)?.focus();
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      requestClose(t);
    }
  }

  const mod = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform) ? '⌘' : 'Ctrl+';
</script>

<div class="tabbar" role="tablist" aria-label="Open projects" bind:this={strip} data-testid="tabbar">
  {#each tabs.list as t, i (t.id)}
    <div
      class="tab"
      class:active={t.id === tabs.activeId}
      class:dragging={drag?.moved && drag.id === t.id}
      class:drop-before={drag?.moved && drag.id !== t.id && drag.over === i && tabs.list.findIndex((x) => x.id === drag?.id) > i}
      class:drop-after={drag?.moved && drag.id !== t.id && drag.over === i && tabs.list.findIndex((x) => x.id === drag?.id) < i}
      role="tab"
      tabindex={t.id === tabs.activeId ? 0 : -1}
      aria-selected={t.id === tabs.activeId}
      data-tab={t.id}
      data-testid="tab"
      title="{t.title}{t.dirty ? ' (unsaved changes)' : ''}{shortcuts && i < 9 ? ` · ${mod}${i + 1}` : ''}"
      onpointerdown={(e) => down(e, t)}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={() => (drag = null)}
      onauxclick={(e) => e.button === 1 && e.preventDefault()}
      onkeydown={(e) => key(e, t)}
    >
      {#if t.busy || t.rendering > 0}
        <span class="spinner" aria-label="working"></span>
      {/if}
      <span class="name">{t.title}</span>
      {#if t.dirty && !t.readOnly}<span class="dot" aria-label="unsaved changes" data-testid="tab-dirty">●</span>{/if}
      <button class="close icon" aria-label="Close {t.title}" title="Close tab{shortcuts ? ` (${mod}W)` : ''}" onclick={() => requestClose(t)}>✕</button>
    </div>
  {/each}
  <button class="add icon" aria-label="New tab" title="New tab{shortcuts ? ` (${mod}T)` : ''}" onclick={() => tabs.add()} data-testid="tab-add">+</button>
</div>

{#if asking}
  <div class="scrim" role="presentation"></div>
  <div class="glass dialog" role="alertdialog" aria-modal="true" aria-labelledby="close-tab-title" data-testid="close-tab-dialog">
    <h2 id="close-tab-title">Close {asking.title}?</h2>
    <p>This tab has unsaved changes or changes not yet in the database. Closing keeps its local draft.</p>
    <div class="row">
      <button class="primary" onclick={() => answer('save')}>Save project</button>
      <button onclick={() => answer('discard')} data-testid="close-tab-discard">Close without saving</button>
      <button onclick={() => answer('cancel')}>Cancel</button>
    </div>
  </div>
{/if}

<style>
  .tabbar {
    display: flex;
    align-items: stretch;
    gap: 2px;
    margin: var(--space-2) var(--space-2) 0;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
    z-index: 45;
    position: relative;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 7rem;
    max-width: 16rem;
    flex: 0 1 14rem;
    padding: 0.2rem 0.3rem 0.2rem var(--space-3);
    border: 1px solid var(--hairline);
    border-bottom: none;
    border-radius: var(--radius) var(--radius) 0 0;
    background: transparent;
    color: var(--text-muted);
    cursor: default;
    user-select: none;
    font-size: 0.85rem;
    touch-action: none;
  }
  .tab.active {
    background: var(--glass-strong);
    color: inherit;
    font-weight: 600;
  }
  .tab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tab.dragging {
    opacity: 0.6;
  }
  .tab.drop-before {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .tab.drop-after {
    box-shadow: inset -3px 0 0 var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .dot {
    color: var(--accent);
    font-size: 0.7rem;
  }
  .close {
    padding: 0 0.3rem;
    min-height: 0;
    line-height: 1.2;
    font-size: 0.75rem;
    border-color: transparent;
    background: transparent;
    opacity: 0.6;
  }
  .close:hover {
    opacity: 1;
  }
  .add {
    padding: 0 0.6rem;
    border-color: transparent;
    background: transparent;
    font-size: 1.1rem;
  }
  .spinner {
    width: 0.7rem;
    height: 0.7rem;
    border: 2px solid var(--hairline);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner { animation: none; }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.25);
    z-index: 60;
  }
  .dialog {
    position: fixed;
    top: 30%;
    left: 50%;
    transform: translateX(-50%);
    padding: var(--space-4);
    z-index: 61;
    max-width: 30rem;
  }
  .dialog h2 {
    margin: 0 0 var(--space-2);
    font-size: 1.05rem;
  }
  .row {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
    margin-top: var(--space-3);
  }
</style>
