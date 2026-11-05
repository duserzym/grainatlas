<script lang="ts">
  import { app } from '../lib/store.svelte';
  import ThemeSwitch from '../components/ThemeSwitch.svelte';
  import { normalizeServer } from '../lib/laicpmsdb';
  import { openFromServer } from '../lib/remote';

  let openId = $state('');

  const hosted = app.hosted;
  let serverText = $state(app.link.server);
  const serverBad = $derived(serverText.trim() !== '' && !normalizeServer(serverText));
  function saveServer() {
    if (serverBad) return;
    app.link.server = normalizeServer(serverText) ?? '';
    serverText = app.link.server;
    app.saveLink();
  }

  let presets = $state(app.settings.spot_presets_um.join(', '));
  function save() {
    app.settings.spot_presets_um = presets.split(/[,\s]+/).map(Number).filter((v) => v > 0).sort((a, b) => a - b);
    app.saveSettings();
  }
</script>

<section class="settings">
  <div class="glass panel">
    <h2>Spots</h2>
    <label class="field">Naming pattern
      <input type="text" bind:value={app.settings.naming_pattern} onchange={save} data-testid="naming-pattern" />
    </label>
    <p class="faint small"><code>{'{sample}'}</code>, <code>{'{grain}'}</code>, <code>{'{n}'}</code> and zero-padded <code>{'{n:03}'}</code>; <code>n</code> continues from the highest number in the project.</p>
    <label class="field">Diameter presets (µm)
      <input type="text" bind:value={presets} onchange={save} />
    </label>
    <label class="field">Default diameter (µm)
      <input type="number" min="1" bind:value={app.settings.default_diameter_um} onchange={save} />
    </label>
    <label class="check"><input type="checkbox" bind:checked={app.settings.snap} onchange={save} /> snap to grain centroid, long-axis ends and the previous spot</label>
    <label class="field">Active spot colour
      <select bind:value={app.settings.spot_cursor_colour} onchange={save} data-testid="spot-cursor-colour">
        <option value="auto">Auto (contrast with the image)</option>
        <option value="#111">Black</option>
        <option value="#fff">White</option>
        <option value="#ff8c42">Orange</option>
        <option value="#56b4e9">Cyan</option>
        <option value="#f0e442">Yellow</option>
      </select>
    </label>
    <label class="field">Your name (created_by)
      <input type="text" bind:value={app.settings.user} onchange={save} />
    </label>
  </div>
  <div class="glass panel">
    <h2>Outlines</h2>
    <label class="field">Margin around new outlines (µm, 0 = off)
      <input type="number" min="0" max="20" step="0.5" bind:value={app.settings.outline_margin_um} onchange={save} data-testid="outline-margin" />
    </label>
    <label class="check"><input type="checkbox" bind:checked={app.settings.outline_smooth} onchange={save} data-testid="outline-smooth" /> Smooth new outlines</label>
    <p class="faint small">Applies to outlines found or drawn from now on; grains already outlined keep theirs.</p>
  </div>
  <div class="glass panel">
    <h2>laicpmsdb</h2>
    <label class="field">Server
      <input type="url" placeholder="http://localhost:8000" bind:value={serverText} onchange={saveServer} aria-invalid={serverBad} data-testid="server" />
    </label>
    <p class="faint small">
      {#if app.serverOverride}This page was opened for {app.serverOverride}, which is used until it is reloaded without <code>?server=</code>.{:else if !app.link.server && app.server}grainatlas is served by {app.server} and uses it.{:else}The laicpmsdb service that receives atlas bundles and spots and gives results. Empty = none; grainatlas works the same without it.{/if}
    </p>
    {#if hosted}
      <p class="faint small">In UPb Studio the shell's sign-in is used (no token needed here).</p>
    {:else}
      <label class="field">Token
        <input type="password" autocomplete="off" placeholder="bearer token from laicpmsdb" bind:value={app.link.token} onchange={() => app.saveLink()} data-testid="token" />
      </label>
      <label class="check"><input type="checkbox" bind:checked={app.link.rememberToken} onchange={() => app.saveLink()} /> remember the token on this computer (else until this tab closes)</label>
    {/if}
    <label class="field">Atlas tiles
      <select bind:value={app.link.quality} onchange={() => app.saveLink()} data-testid="quality">
        <option value={100}>lossless WebP</option>
        <option value={97}>near-lossless, quality 97 (±1 grey level)</option>
        <option value={92}>near-lossless, quality 92 (±2 grey levels)</option>
      </select>
    </label>
    <label class="field">SEM packages on the lab storage
      <input type="text" placeholder="e.g. /laicpmsdb/sem or \\nas\sem" bind:value={app.link.nasRoot} onchange={() => app.saveLink()} />
    </label>
    <div class="openrow">
      <label class="field">Open a published project
        <input type="text" placeholder="project id" bind:value={openId} data-testid="open-project-id" />
      </label>
      <button onclick={() => app.server && openFromServer(app.server, openId.trim())} disabled={!app.server || !openId.trim()} data-testid="open-from-server">Open from server</button>
    </div>
    <p class="faint small">Reads <code>{app.server ?? '<server>'}/atlas/projects/&lt;id&gt;/</code>: the mosaic tiles, grains, spots and scale bars, read-only. The same as the link <code>index.html?server=&lt;server&gt;&amp;project=&lt;id&gt;&amp;spot=&lt;spot id&gt;</code>.</p>
    <p class="faint small">Bundles record the raw package as <code>&lt;this folder&gt;/&lt;package name&gt;</code> and its content hash; the package itself is never uploaded.</p>
  </div>
  <div class="glass panel">
    <h2>Appearance and layout</h2>
    <div class="field">Theme <ThemeSwitch labels /></div>
    <div class="openrow">
      <p class="faint small grow">Panel positions and folded panels are remembered on this computer{app.hosted ? '' : ', and the desktop app remembers its window'}. Reset puts everything back where it starts.</p>
      <button onclick={() => app.resetLayout()} data-testid="reset-layout">Reset layout</button>
    </div>
    <p class="faint small">Light by default; System follows the computer's setting. Images stay on a dark surface in both. {app.hostTheme ? `UPb Studio sets the theme here (${app.hostTheme}).` : app.hosted ? 'In UPb Studio, with nothing chosen here, grainatlas follows the system setting.' : ''}</p>
  </div>
  <div class="glass panel">
    <h2>Keyboard</h2>
    <table class="data">
      <tbody>
        <tr><td><span class="kbd">V</span></td><td>select / move</td><td><span class="kbd">S</span></td><td>spot</td></tr>
        <tr><td><span class="kbd">L</span></td><td>line</td><td><span class="kbd">R</span></td><td>raster</td></tr>
        <tr><td><span class="kbd">M</span></td><td>measure</td><td><span class="kbd">T</span></td><td>label</td></tr>
        <tr><td><span class="kbd">D</span></td><td>delete tool</td><td><span class="kbd">⌫</span></td><td>delete selection</td></tr>
        <tr><td><span class="kbd">G</span></td><td>add grain (click-grow)</td><td><span class="kbd">K</span></td><td>split grain (draw a line)</td></tr>
        <tr><td><span class="kbd">O</span></td><td>outline a grain: drag around it, or click corners (double-click / Enter closes); ⇧ add, ⌥ cut, ⌘ no snapping</td><td><span class="kbd">B</span></td><td>grain brush: paint a grain in, ⌥ erase</td></tr>
        <tr><td><span class="kbd">⌘Z</span> / <span class="kbd">⇧⌘Z</span></td><td>undo / redo</td><td><span class="kbd">+</span> <span class="kbd">−</span> <span class="kbd">0</span></td><td>zoom in, out, fit</td></tr>
        <tr><td><span class="kbd">←</span> <span class="kbd">→</span> <span class="kbd">↑</span> <span class="kbd">↓</span></td><td>nudge the view (⇧ more); move selected spots on the Canvas</td><td>double-click</td><td>fit (on empty space)</td></tr>
        <tr><td><span class="kbd">[</span> <span class="kbd">]</span></td><td>rotate 15°</td><td><span class="kbd">Space</span>+drag</td><td>pan with any tool</td></tr>
        <tr><td><span class="kbd">1</span>…<span class="kbd">6</span></td><td>diameter preset</td><td><span class="kbd">⌥1</span>…<span class="kbd">⌥5</span></td><td>screens</td></tr>
      </tbody>
    </table>
    <p class="faint small">Every image view (Area, Canvas, Layout preview) pans and zooms the same way: the mouse wheel zooms about the cursor, two-finger scroll pans, pinch zooms, drag empty space to pan, double-click to fit. On the Canvas: click to place, drag a spot to move it, drag its ring handle to resize, double-click its name to rename.</p>
  </div>
</section>

<style>
  /* The whole screen scrolls inside main (never past the window): padding, not margin. */
  .settings {
    max-width: 960px;
    margin: 0 auto;
    padding: var(--space-6) var(--space-4);
    box-sizing: border-box;
    display: grid;
    align-content: start;
    gap: var(--space-4);
    overflow: auto;
    height: 100%;
  }
  .panel {
    display: grid;
    gap: var(--space-2);
  }
  .small {
    font-size: 0.8rem;
  }
  .openrow {
    display: flex;
    gap: var(--space-2);
    align-items: end;
  }
  .openrow .field,
  .openrow .grow {
    flex: 1;
  }
</style>
