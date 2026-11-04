<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from '../lib/store.svelte';
  import { fromDrop, fromInput } from '../lib/files';
  import { findAllGrains, loadProjectFile, openExample, openFiles, openPath, stitchAll } from '../lib/actions';
  import { isDesktop } from '../lib/core/core';
  import { readRecent, type RecentEntry } from '../lib/deeplink';

  let over = $state(false);
  let input: HTMLInputElement;
  let recent = $state<RecentEntry[]>([]);
  const desktop = isDesktop();
  let startupDone = false;

  onMount(() => {
    recent = readRecent();
    if (!desktop) return;
    if (!startupDone) {
      startupDone = true;
      import('@tauri-apps/api/core').then(async ({ invoke }) => {
        const s = await invoke<{ path: string | null; stitch: boolean; segment: boolean }>('startup');
        if (!s.path) return;
        await openPath(s.path);
        if (s.stitch) await stitchAll();
        if (s.segment) {
          await findAllGrains();
          app.screen = 'canvas';
        }
      });
    }
    // Desktop: dropped folders arrive as paths through Tauri.
    let un: (() => void) | undefined;
    import('@tauri-apps/api/webview').then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((e) => {
          if (e.payload.type === 'over') over = true;
          else if (e.payload.type === 'leave') over = false;
          else if (e.payload.type === 'drop') {
            over = false;
            const p = e.payload.paths[0];
            if (p) openPath(p);
          }
        })
        .then((f) => (un = f)),
    );
    return () => un?.();
  });

  async function pick() {
    if (desktop) {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const p = await open({ directory: true, multiple: false, title: 'Choose a package folder' });
      if (typeof p === 'string') openPath(p);
    } else input.click();
  }

  async function chosen() {
    if (!input.files?.length) return;
    const { name, files } = fromInput(input.files);
    input.value = '';
    await openFiles(name, files);
  }

  async function drop(e: DragEvent) {
    e.preventDefault();
    over = false;
    if (desktop || !e.dataTransfer) return;
    const r = await fromDrop(e.dataTransfer);
    if (r) await openFiles(r.name, r.files);
    else app.error = 'Drop a folder (the package), not single files.';
  }


  async function project(e: Event) {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f) await loadProjectFile(f).catch((err) => (app.error = String(err)));
  }
</script>

<section class="open">
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
    aria-label="Open a package folder"
    ondragover={(e) => { e.preventDefault(); over = true; }}
    ondragleave={() => (over = false)}
    ondrop={drop}
    onclick={pick}
    onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && pick()}
  >
    <h1>Open a package</h1>
    <p class="muted">Drop a folder here or click to choose one: a Hitachi TM4000 Zigzag package (the folder with the <code>.zag</code> and one subfolder per area) or any folder of images with a <code>positions.csv</code> (<code>file,x_um,y_um,pixel_size_nm</code>).</p>
    <p class="faint">Nothing leaves this computer: the files are read in this page{desktop ? ' (desktop app)' : ''}.</p>
    <input bind:this={input} type="file" webkitdirectory multiple hidden onchange={chosen} data-testid="package-input" />
  </div>

  {#if !desktop}<div class="example">
    <button onclick={openExample} data-testid="example">Try the example</button>
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
              <tr><td>{r.name}</td><td class="num">{r.areas}</td><td class="num">{r.spots}</td><td class="muted">{new Date(r.when).toLocaleString()}</td></tr>
            {/each}
          </tbody>
        </table>
        <p class="faint small">Re-open the same package folder to continue: grains, spots and parameters come back.</p>
      {:else}
        <p class="muted">None yet.</p>
      {/if}
    </div>
    <div class="glass panel">
      <h2>Project file</h2>
      <p class="muted">After opening the package, load a saved <code>project.json</code> to restore its parameters, grains and spots.</p>
      <input type="file" accept=".json,application/json" onchange={project} disabled={!app.pkg} />
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
  }
</style>
