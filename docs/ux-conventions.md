# UX conventions (grainatlas, UPbDB, UPb Studio)

Seven rules, applied in grainatlas on 2026-11-03, for the three apps of the
lab workflow to adopt. Each says what to do and where grainatlas does it.

## 1. Light by default, with dark and system

- A three-way switch, **Light / Dark / System**, in the top bar and in
  Settings (`ui/src/components/ThemeSwitch.svelte`). Nothing stored means
  **light**.
- The choice is the viewer's, kept in their own storage
  (`localStorage['grainatlas.theme']`), never in a project file or shared
  record: opening someone else's project must not change your theme.
- Tokens: the light set is the base; `[data-theme='dark']` overrides it
  (`ui/src/app.css`). Chrome follows the theme; **image surfaces stay dark in
  both** (`--image-bg`), and overlay colours on images are one bright,
  colour-blind-safe set in both. Glass floating over an image uses a denser
  token (`--glass-on-image`) so text stays readable in light.
- Hosted in UPb Studio: the shell's theme wins while it passes one, as
  `?theme=light|dark` in the module URL or `data-theme` on the module's
  `<html>`; with nothing passed and nothing chosen in the module, follow the
  system (`prefers-color-scheme`).

## 2. One gesture module for every image view

Every view that shows an image (mosaic, tiles, page preview, thumbnails)
uses one module with one set of constants
(`ui/src/lib/canvas/gestures.ts`), so all feel the same:

| Input | Does |
| --- | --- |
| mouse wheel | zoom about the cursor (`exp(-dy * 0.0015)`) |
| trackpad two-finger scroll | pan |
| trackpad pinch, ctrl/cmd + wheel | zoom about the fingers / cursor |
| drag on empty space | pan, except in the Canvas Select tool, where it draws a selection box |
| middle drag, Space + drag | pan |
| two touch points | pinch zoom (rotate where the view allows it) |
| double-click on empty space | fit |
| arrows (Shift: more), `+` `-` `0` | nudge 60 / 240 px, zoom 1.5×, fit |

Engines differ, so classify each wheel event: Chromium (and WebView2)
report a pinch as `ctrlKey` + small `deltaY`; WebKit (Safari, WKWebView in
Tauri on macOS) sends `gesturestart/change/end` instead; a trackpad scroll
has `wheelDeltaY === -3 * deltaY` in both, a mouse notch a multiple of 120;
Firefox has no `wheelDeltaY` and reports notches with `deltaMode` 1. Plots
inside a scrolling page (UPbDB's timeline) are not image views: plain
wheel keeps scrolling the page there, ctrl + wheel zooms.

**Selecting several.** Cmd-click (macOS) or Ctrl-click toggles a spot or
grain. Shift-click selects a range in a table and adds on the Canvas.
In the Select tool, plain drag on empty space or a grain draws a box and
replaces selection with the items whose centroids are inside; an empty box
clears selection. Plain drag starting on a selected spot moves the selected
spots. Shift+drag adds items in a box. Cmd/Ctrl+drag toggles them, also with
Shift held. Modified boxes can start on a spot without moving or deleting
it. A Shift-click without movement keeps the tool's usual click behaviour.
Other tools pan on plain drag on empty space. Space+drag, middle drag and
trackpad two-finger scroll pan in every tool.

| Canvas tool | Drag behaviour and box selection |
| --- | --- |
| Select (V) | Plain drag replaces; Shift adds; Cmd/Ctrl toggles; enclosed spots take priority in the Spots tab, otherwise grains |
| Spot (S) | Plain drag on empty space pans; Shift adds; Cmd/Ctrl toggles; grains in the Grains tab, spots otherwise |
| Delete (D) | Plain drag on empty space pans; Shift adds; Cmd/Ctrl toggles; enclosed spots take priority in the Spots tab, otherwise grains |
| Add grain (G) | Plain drag on empty space pans; Shift adds; Cmd/Ctrl toggles; always grains |
| Outline (O), Brush (B), Split (K) | Not available; keep stroke modifiers; switch to Select (V) to box-select |

Artifacts are included only while the artifacts layer is visible. Grain
table filters still apply. Box selection changes selection only, leaving
spot data untouched. Selected grains appear in the table and on the Canvas
and use the same grain actions as click selection.
Select and Delete boxes switch to the Grains tab when they select grains.
In the Spots tab, a box with no enclosed spots falls back to visible grains;
an empty box still clears selection. Other side tabs target grains with these
two tools. A canceled pointer, lost pointer capture or a second touch discards
the active tool gesture. Native text and image dragging is disabled on the canvas.

Cmd/Ctrl+A selects all spots
in the current area, or visible grains in the Grains tab. Escape clears
the selection and keeps the tool; with no selection it switches to Select.
Delete or Backspace deletes the selected spots, or the selected grains.
The Delete tool removes the whole grain selection when clicking a selected grain.

Spot size comes only from the size input in the side panel, for new and selected
spots; placed spots are never resized by dragging.

## 3. One click for a good result; every control still there; Reset

- An **Auto** button gives a result that is hard to get wrong, computed from
  the data (grainatlas: leveling on, a stretch that clips almost nothing,
  CLAHE with a clip limit from the histogram). It is deterministic and
  idempotent: a second click changes nothing.
- **Before / After** swaps the Auto result with what was there; every manual
  control stays visible and editable; an explicit **Reset** returns the
  defaults.
- Every control acts at once: a low-resolution preview while it moves, the
  full result when it settles. No control may silently wait for another
  button ("applies on stitch"); if one truly needs a heavy step, its label
  says so.
- Results are parameters in the project file, never baked pixels; native
  and browser builds give identical results (parity hash in `make test`).

## 4. Open anything, from a whole folder down to loose files

- One Open screen and drag-drop accept a whole instrument package, one
  sub-folder of it on its own, or loose files; the app detects which and
  never asks the user to say what they dropped.
- Missing information is asked for once, in one dialog, then the file set
  opens like any other (grainatlas: `Session::open_source` returns
  `opened` or `needs_scale`, ADR-0006).
- The project records how it was opened (`package.kind`: `package`,
  `area`, `loose`, plus what was asked), so reopening rebuilds it the same
  way without asking again.
- Hosted in UPb Studio, the shell opens anything dropped on its window (or
  opened through the OS), routes it by content to the tab that reads it and
  hands the files over (`@lab/bridge` `onOpenFiles`). The module opens the
  set exactly like a drop on its own Open screen, the one-time scale dialog
  included, and takes the files at once.
- Opening never silently discards work: when a project is already open,
  one dialog (Open, Cancel) asks before the new set replaces it. Cancel is
  the default button, so Enter keeps the open project.

## 5. The scale is always known, or asked once

- Take the scale from the files when they carry it (TM4000 `PixelSize` in
  the `.txt` sidecars). Never guess a scale silently.
- Otherwise ask once per set: µm per pixel, or the field width in µm. The
  last value typed is the default for the rest of the session
  (`sessionStorage`), the answer is stored in the project, shown where it is
  used, and can be changed later ("Change…" on the Area screen).

## 6. Server features are additive, and say where data comes from

- Everything that needs UPbDB sits behind one Settings entry (the
  server) and works the same without it; a missing server, mount or token
  turns a server action into its file equivalent (grainatlas: the atlas
  bundle saves as a folder or zip) and the
  message says which was missing.
- Sign-in: the shell's `token()` when hosted; standalone, a token pasted in
  Settings, kept until the tab closes unless "remember" is ticked. Never in
  a project file or a URL.
- Something opened from a server is **read-only** until the user asks for a
  copy ("Edit a copy" in the top bar); the top bar names the server, and a
  line in the side panel says why nothing can be edited. A read-only view
  writes nothing to local storage.
- Values read from the server (results) are shown, never written into the
  project; the panel says so and has a Refresh button. Errors from the server
  stay next to the feature (status line) instead of a toast that hides the
  work.
- Server image tiles go through the same viewer and gesture module as local
  pixels (rule 2); overlays on images use the colour-blind-safe set (the
  results layer: viridis, flags as a dashed ring).

## 7. Fit a 14-inch laptop as well as a large monitor

- Every app is checked at **1440 x 900** and **1280 x 800** (CSS pixels):
  nothing sticks out of the window, nothing is cut off by a panel that does
  not scroll, and every control can be reached (grainatlas:
  `ui/tests/sizes.spec.ts` walks every screen and side-panel tab at both
  sizes; content inside a scrolling area counts as reachable).
- Panels collapse, dock, scroll or tab rather than overflow. grainatlas: the
  Canvas side panel is tabbed, its tab body scrolls under fixed tabs and a
  Layers section, and it folds to a strip (» / «) to give the image the
  width; the floating spot and grain panel scrolls inside and is pulled back
  inside the canvas when the window shrinks; the toolbar wraps. The Area
  screen's area list and its right-hand panel scroll on their own beside the
  image; the Layout screen's controls column scrolls and its page preview
  pans and zooms like every image view (rule 2); Settings scrolls inside the
  window.
- Minimum sizes keep nothing clipped: the desktop window opens at 1280 x 800,
  centred, and cannot go below 1024 x 640; lists in panels keep a minimum
  height of a few rows.
- Window and splitter geometry is remembered per viewer, never in a project:
  grainatlas keeps the spot panel's position, the folded side panel and the
  open Layers section in `localStorage['grainatlas.layout']`, and the desktop
  app keeps its window size and position in `window.json` in its config
  folder, restored only when that place is still on a connected monitor and
  clamped to its work area (`tauri/src-tauri/src/geometry.rs`). grainatlas
  has no splitters yet; one added later stores its position in the same key.
- A **Reset layout** action puts all of it back (grainatlas: Settings →
  Appearance and layout; the desktop window returns to 1280 x 800, centred).

## What the other apps need to change

- **UPb Studio shell**: default theme is dark (`ui/src/lib/settings.svelte.ts`);
  make it light. Pass the theme to module tabs (append `?theme=` to the
  module URL, or set `data-theme` on the module document from the host;
  a bridge `theme` event would cover live changes). Settings says "module
  tabs keep their own theme settings for now".
- **UPbDB**: default theme is `system` (`ui/src/lib/theme.svelte.ts`);
  make nothing-stored mean light, keep the three-way choice, and when
  hosted read `?theme=` before its own stored choice. It writes
  `data-theme` on its own `<html>`, so the shell should use `?theme=` for
  it. Any image view it adds uses the gesture table above.
- **All apps** (rule 7): check every screen at 1440 x 900 and 1280 x 800,
  remember window and splitter geometry outside shared records, and offer
  Reset layout.
