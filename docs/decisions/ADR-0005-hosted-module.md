# ADR-0005: grainatlas as a hosted module of UPb Studio

Date: 2026-11-03. Status: accepted.

(The task brief called this ADR-0004; that number was already taken by the
segmentation ADR.)

## Context

UPb Studio (`../upblab`) shows the grainatlas web build as its Images tab:
a native child webview on desktop, an iframe named `lab-shell:<module>` in the
web delivery (`upblab/docs/decisions/ADR-0002-tab-hosting.md`). Modules talk
to the shell only through `@lab/bridge` (`open`, `token`, `notify`,
`isHosted`), and follow `upblab/docs/cross-platform-contract.md`. Tauri
injects `__TAURI_INTERNALS__` into the shell's module webviews too, but only
the three bridge commands are allowed there, not grainatlas's own commands.

## Decision

- **Dependency.** `ui/package.json` has `"@lab/bridge":
  "file:../../upblab/bridge"` until the bridge is published. `make bridge`
  (run by `make setup`) builds the sibling's `bridge/dist`; CI and release jobs
  check out both repositories side by side. README gives the tarball fallback.
- **Which core.** `isDesktop()` (`ui/src/lib/core/core.ts`) is false when
  `isHosted()`, and only otherwise checks `__TAURI_INTERNALS__`. Hosted means
  the wasm core, `<input type=file webkitdirectory>` and HTML drag-drop, plain
  `<a download>` saves, and no `@tauri-apps/plugin-dialog`. The Open screen,
  `files.ts` and `getCore()` all ask `isDesktop()`.
- **Deep links.** At start-up `?project=<id>&spot=<id>` (the shell's mapping of
  `grainatlas://project/<id>/spot/<id>`) is read and then dropped from the
  address. The recent list (`grainatlas.recent`) now stores each project's
  uuid; older entries are found by scanning `grainatlas.project.*`. The page
  keeps no pixels, so the package has to be read again: the bundled example
  opens by itself; any other package waits on the Open screen, which names
  the project id (and the package, when known) and offers Open. After the
  package (or, for an unknown id, its `project.json`) is open, the spot's area
  is stitched, the spot selected and the canvas centered on it. Without the
  parameters nothing changes.
- **Standalone desktop deep links** (`grainatlas://` through
  `tauri-plugin-deep-link`) are not added yet: they need a bundled build to
  test on macOS, the single-instance plugin on Windows, and a rule for the
  machine where both UPb Studio and grainatlas desktop claim the scheme. Noted
  in `ARCHITECTURE_PLAN.md` section 7.
- **Shell-owned things.** grainatlas binds `Alt 1–5`, `Ctrl/⌘ S` and
  unmodified tool keys, none of the shell's `Ctrl/⌘ 1–4`, `Ctrl/⌘ ,`,
  `Ctrl/⌘ R`. It never sets the window title. Its theme setting stays: the
  shell's theme applies only to the shell's own pages ("module tabs keep their
  own theme settings for now").
- **Storage.** Every `localStorage` key starts with `grainatlas.` (checked by
  grep and asserted in the deep-link Playwright test); there is no IndexedDB
  use. The desktop shell gives all bundled pages one origin.
- **Outbound.** A spot may carry `acquisition_id` (UI-held, editable in the
  spot panel). "Open in laicpmsdb" calls `open('laicpmsdb://acquisition/<id>')`,
  else `open('laicpmsdb://search?label=<spot name>')`; it is disabled with a
  tooltip when not hosted. Selecting one spot sends `{ type:
  'lab.spot.selected', project, spot, name, grain, area, via }`. "Send spots to
  laicpmsdb" on the Spots tab downloads the body of `POST
  /v1/mounts/{mount_id}/spots`: `{ project_id, sample, spots: [{ id, name,
  grain, x_um, y_um, diameter_um, kind: 'spot' }] }` for the current area's
  point spots, in mosaic micrometres (lines and rasters left out, counted in a
  notice). A direct POST with `token()` waits for the shell's phase 3.
- **Release asset.** `make release-web` packs `ui/dist` as
  `grainatlas-web-<version>.tar.gz` with `index.html` at the top and prints its
  SHA-256; `.github/workflows/release.yml` attaches it and its `.sha256` to a
  draft release on a `v*` tag, for `upbstudio/modules.lock.json`.

## Consequences

### Bridge v3

Hosted builds register `onCommand` beside `onOpenFiles`. `focus-spot` and
`focus-grain` activate the matching project tab, or open a published atlas
read-only using an optional HTTP(S) `server` (else the page's server or
Settings), then fall back to a pending deep link when unavailable. They
select and centre the target and briefly highlight it, with a static
highlight for reduced motion. Grain lookup accepts ids, area-qualified labels
and unambiguous numbers. `lab.spot.selected` replaces the outbound legacy
event and includes `{project, spot, name, grain, area, via}`; command spot
selections keep `via: 'shell'` until the selection changes. Offered project
folders keep their `project.json` in memory until the named SEM package opens,
using the existing Project file loader and replace dialog. Standalone builds
never register commands; older shells continue without commands.

### Existing hosting consequences

- One build serves the static page, the desktop app and both shell
  deliveries; the hosted page is the static page plus the bridge.
- A deep link to a project other than the example costs the user one folder
  pick: the browser cannot reopen a folder by itself without the File System
  Access API, which the contract excludes (WebView2 and WKWebView differ).
- `laicpmsdb://search?label=` is refused by today's shell resolver (it knows
  `acquisition`, `session`, `sample`); the shell and laicpmsdb need a search
  route, or grainatlas needs the acquisition id from laicpmsdb.
- In the desktop shell, Tauri's native drag-drop handler is on by default for
  child webviews and swallows HTML file drops; the shell should build module
  webviews with `disable_drag_drop_handler()` for drag-drop to reach the page.
  The file input works either way.
- CI depends on `duserzym/upblab` being reachable until the bridge is on
  npm.
