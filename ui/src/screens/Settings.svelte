<script lang="ts">
  import { app } from '../lib/store.svelte';
  import ThemeSwitch from '../components/ThemeSwitch.svelte';

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
    <label class="field">Your name (created_by)
      <input type="text" bind:value={app.settings.user} onchange={save} />
    </label>
  </div>
  <div class="glass panel">
    <h2>Appearance</h2>
    <div class="field">Theme <ThemeSwitch labels /></div>
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
  .settings {
    max-width: 960px;
    margin: var(--space-6) auto;
    padding: 0 var(--space-4);
    display: grid;
    gap: var(--space-4);
    overflow: auto;
    max-height: 100%;
  }
  .panel {
    display: grid;
    gap: var(--space-2);
  }
  .small {
    font-size: 0.8rem;
  }
</style>
