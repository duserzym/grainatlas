<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from '../lib/store.svelte';
  import { fromDrop, fromInput } from '../lib/files';
  import { findAllGrains, loadProjectFile, openExample, openFiles, openLoose, openPaths, stitchAll } from '../lib/actions';
  import { isDesktop } from '../lib/core/core';
  import { readRecent, type RecentEntry } from '../lib/deeplink';

  let over = $state(false);
  let input: HTMLInputElement;
  let imagesInput: HTMLInputElement;
  const IMAGE_EXT = ['jpg', 'jpeg', 'png', 'tif', 'tiff', 'bmp'];
  let recent = $state<RecentEntry[]>([]);
  const desktop = isDesktop();
  let startupDone = false;

  onMount(() => {
    recent = readRecent();
    if (!desktop) return;
    if (!startupDone) {
      startupDone = true;
      import('@tauri-apps/api/core').then(async ({ invoke }) => {
        const s = await invoke<{ paths: string[]; stitch: boolean; segment: boolean; um_per_px: number | null; field_um: number | null }>('startup');
        if (!s.paths.length) return;
        await openPaths(s.paths);
        // Loose images with a scale on the command line open without the dialog.
        const pending = app.pendingLoose;
        if (pending && (s.um_per_px || s.field_um)) {
          await openLoose({ um_per_px: s.um_per_px ?? 0, ...(s.field_um ? { field_width_um: s.field_um } : {}), files: pending.images.map((i) => i.file) });
        }
        if (!app.pkg) return;
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
            if (e.payload.paths.length) openPaths(e.payload.paths);
          }
        })
        .then((f) => (un = f)),
    );
    return () => un?.();
  });

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
  }
</style>
