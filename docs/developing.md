# Developing GrainAtlas

## Quick start

```bash
make setup        # Rust stable + wasm target, wasm-bindgen-cli, UI dependencies
make full         # Rust unit tests, svelte-check and the desktop crate
make ui-build     # static page in ui/dist (serve it from any static server)
make ui-dev       # dev server on http://localhost:5174
make desktop-dev  # desktop app (Tauri 2) on this machine
```

Sample data and the image-based test suite stay outside the public repository.
`make full` runs the Rust unit tests, type checks and a desktop compile.

One Rust core runs in a Web Worker through wasm in the static browser build,
and natively with multiple threads in the Tauri 2 desktop app. Both use the
same Svelte interface.

Development desktop builds use the separate `dev` browser storage profile by default.
Test bundles must be launched with `GRAINATLAS_PROFILE=test` or `--profile test`; named profiles keep localStorage separate and show their name in the window title (on macOS, separate stores require macOS 14 or later).
Release launches without a profile keep the normal app's existing storage.

CI: `ci`, `desktop` and `release` run on `v*` tags and by hand, never on
branch pushes or pull requests. They check out `upblab` alongside this
repository; a private checkout needs the read-only `UPBSTUDIO_TOKEN` secret.
`desktop` builds macOS and Windows installers and signed updates; `release`
builds the web archive. Tags attach assets to a draft release; manual runs
keep workflow artifacts. Pages is a separate, manual-only workflow.

## What it does, in detail

Status: roadmap phases 1–11 of [ARCHITECTURE_PLAN.md](../ARCHITECTURE_PLAN.md) are implemented (readers
and stitching, photometric correction, web UI, segmentation and scale, spots,
print layout, desktop, the UPbDB link, atlas bundles, results overlay and
laser registration), except a reader for a second instrument (phase 8, waiting
for example files) and an adapter for the lab's laser software (phase 11,
waiting for its name and an example file).

1. Opens a Hitachi TM4000 Zigzag package (`.zag`, UTF-16LE `.ini` and per-tile
   `.txt`, unknown keys kept), one TM4000 area folder on its own (scale and
   stage positions from its sidecars), any folder of images with
   `positions.csv` (`file,x_um,y_um,pixel_size_nm`), or loose images with a
   scale typed once (each image an area, or one grid); see [ADR-0006](decisions/ADR-0006-opening-kinds.md).
2. Stitches by stage prior, phase correlation with a Lucas-Kanade refinement
   and a global least-squares solve; corrects per-tile gain and offset, beam
   exposure steps and background in one joint solve over all tiles (overlaps agree,
   background flat; `stitch_notes.md`); blends with seams and multiband
   (photocraft-algo). The 4 x 8 example area (32 tiles, 28 Mpx) takes 0.6 s
   natively and 8 s in the browser; see `stitch_notes.md`. Tone is one
   click (Auto: leveling, stretch, CLAHE from the histogram, with Before /
   After and Reset) or manual; every control re-renders the mosaic live, and
   after segmentation each grain's tone can be matched to the area or to one
   grain.
3. Finds and numbers grains (reading order, stable across re-runs), with
   GrabCut refinement and manual add, split, merge and renumber; hand editing
   with an edge-snapping lasso and a brush, kept across re-runs; scale bars
   per grain; grain table CSV. The artifact rules follow the image signal
   (CL or BSE, read from the TM4000 metadata, overridable in the Grains tab;
   `print_sheets.md`). The Grains tab says when an area has never been
   segmented and finds grains in every area without grains in one click;
   `grainatlas segment <project> --missing` does the same from the command line.
   Find missed grains adds uncovered detections while keeping existing outlines,
   numbers and spot links. New outlines are dashed until accepted; one undo
   restores the whole action. The CLI equivalent is
   `grainatlas segment <project> --add-missing [--area ID]... [--signal auto|cl|bse] [--dry-run]`.
   It requires an existing grain list and cannot be combined with `--missing`.
4. Places laser spots, lines and rasters on an infinite canvas with the mouse
   or trackpad alone (click to place, drag to move,
   double-click to rename), named from a pattern, `{sample}_{n}` by default (n continues across
   samples within a session, or per sample when chosen), with UUIDv7 ids;
   batch edits of the selection; exports CSV and GeoJSON.
   Spot size comes only from the size input in the side panel, for new and
   selected spots; placed spots are never resized by dragging.
5. Packs grains or areas onto Letter, Tabloid, A4 or A3 pages with labels,
   scale bars, spots and cut marks; exports PDF and SVG.
6. Registers reference images (photos of annotated printouts, post-laser
   frames) to the mosaic by point pairs, to recover spots of legacy sessions
   (`legacy_workflow.md`).
7. Writes atlas bundles (`upblab/docs/WORKFLOW.md` section 3.1) and
   publishes them to UPbDB; opens published ones from a server; shows
   UPbDB's results on the spots; registers the laser stage and writes the
   spot list (see "UPbDB and the laser" below).

## Re-render saved areas

`grainatlas rerender <project.grainatlas>` re-runs photometric correction,
blend and tone from each area's saved placement and parameters. It requires
the original package at the saved path. Repeat `--area ID` to select areas.
`--background keep` (the default) uses the saved settings; `on` enables joint
background correction and `off` disables background correction. `--dry-run`
computes the JSON report without writing files. Without `--area`, areas with
no saved placement are skipped with a note on stderr and in the report. An
explicitly selected area without a placement is an error.

Save and close the project in the app before running rerender. The command
refuses to install results if project.json's modification time or size has
changed since it started. The app writes no on-disk open-project lock or
marker, so rerender cannot detect an open project that has not been saved.
The app's next save of a project that was open during a rerender would write
old photometric metadata. Reopen the project after rerender finishes.

The command keeps geometry, grains, spots and annotations unchanged. It
replaces the selected mosaic pyramids and photometric results, and saves
the originals as `project.before-rerender.YYYY-MM-DD_HHMM.json` and
`mosaic.before-rerender.YYYY-MM-DD_HHMM/<area>/` (UTC). An existing backup
for that minute is never overwritten. The JSON report includes elapsed
seconds, overlap RMS before and after correction, and grain and spot counts.

After a crash mid-install, keep the app closed and use backups with the same
stamp to recover by hand. Copy `project.before-rerender.STAMP.json` over
`project.json`. For every area folder present in
`mosaic.before-rerender.STAMP/`, move any replacement `mosaic/<area>/` aside,
then move the backed-up folder back to `mosaic/<area>/`. Leave areas absent
from that backup alone; they were not moved. Keep the backups until the
restored project opens correctly.

## Dependencies: `@lab/bridge`

grainatlas runs on its own and as the Images tab of UPb Lab. It talks to
the shell through `@lab/bridge`, which is not on npm yet: `ui/package.json`
depends on a sibling checkout, `"@lab/bridge": "file:../../upblab/bridge"`
(paths from `ui/`), so clone `upblab` next to `grainatlas`. `make setup`
builds `../upblab/bridge/dist` if it is missing (`make bridge`), before
`pnpm install`. CI checks out both repositories side by side
(`duserzym/upblab`; a private one needs a read-only `UPBSTUDIO_TOKEN`
secret).

Without an `upblab` checkout, take the bridge tarball from someone who has
one (`pnpm --filter @lab/bridge pack` in `upbstudio` writes
`lab-bridge-<version>.tgz`) and point the dependency at it, locally only:

```bash
cd ui && pnpm add ./lab-bridge-0.1.0.tgz
```

## Inside UPb Lab

- Hosted (the bridge's `isHosted()`), grainatlas uses the wasm core and the
  web file paths (`<input type=file webkitdirectory>`, drag-drop), even though
  Tauri's globals are present in the shell's webview.
- `index.html?project=<id>&spot=<id>` (the shell's mapping of
  `grainatlas://project/<id>/spot/<id>`) opens that project, selects the spot
  and centers the canvas on it. The browser keeps projects without pixels, so
  the package folder is opened again.
- A spot's panel has "Open in UPbDB" (`upbdb://acquisition/<id>` when
  the spot has an acquisition id, else `upbdb://search?label=<name>`);
  selecting a spot sends `lab.spot.selected` with `{project, spot, name,
  grain, area, via}`. `grain` is `<area_id>/<grain_number>` or null; `via` is
  `shell` for a command's selection until the user selects something else,
  otherwise `user`.
- Bridge v3 commands arrive through `onCommand`: `focus-spot {project, spot,
  flash?, via?, server?}` and `focus-grain {project, grain, spot?, flash?,
  via?, server?}`. They activate an open project tab, or open a published atlas
  read-only from the HTTP(S) server (the command's, else the page's server or
  Settings); otherwise they wait for the SEM package on Open. A grain may be
  its id, `<area_id>/<number>`, or an unambiguous number (qualified by the
  spot's area when supplied). Focus centres the canvas and highlights for
  about 1.5 s, unless `flash: false`; reduced motion uses a static highlight.
- A project folder offered by the shell keeps its `project.json` pending and
  names the SEM package to open. The next matching package loads that project
  automatically; an already open matching package loads it immediately, asking
  before replacing a different dirty project. `grainatlas.files.opened`
  reports `waiting: package <name>` and then `opened: project <name>`.
- "Send spots to UPbDB" on the Spots tab posts the spots with the shell's
  `token()` when a server and a mount are set, else downloads the JSON body of
  UPbDB's `POST /v1/mounts/{mount_id}/spots`.
- Storage keys all start with `grainatlas.` (the desktop shell shares one
  origin between its pages).

See `decisions/ADR-0005-hosted-module.md`.

In UPb Lab Review's read-only viewer, the mount inset shows every area,
the current area, the visible image region and selected spots. When UPb Lab
sends sample `groups` in `review-spots` (bridge v5.2), it shows a padded
region in each sample's colour around its spots in each area, with small
dots and sample names where labels fit. The header counts samples. A later
command without groups for this project restores the selected-spot view;
the editor never shows sample regions.

## UPbDB and the laser

All of this is optional; without a server grainatlas works as before.

- **Settings → UPbDB**: the server (`http://host:8000`) and, in the
  standalone desktop app, **Sign in…**: it opens the server's sign-in page
  in the browser (password, ORCID, Google or GitHub, whatever the server
  offers), the person confirms, and the browser hands a one-time code back to
  the app on `127.0.0.1` (RFC 8252 loopback; UPbDB's `#/connect` page). A
  pasted token still works; the static browser build uses token entry.
  Inside UPb Lab the shell's sign-in is used. Settings also holds
  the atlas tile quality (lossless WebP, or near-lossless at 97 or 92) and the
  folder where SEM packages live on the lab storage. A `?server=` in the page
  address wins for that page load; served by UPbDB under `/atlas/`,
  grainatlas uses its own origin.
- **Atlas bundle** (Canvas → UPbDB tab): `<project_id>.atlas/` with
  `atlas.json` (format `grainatlas-atlas` v1, ids, mount, scale, areas and
  levels, the raw package's content hash and storage path, sha256 and size of
  every file), `pyramid/<area>/<z>/<x>_<y>.webp` (256 px tiles, z = 0 full
  resolution), `grains.geojson`, `spots.json`, `scalebars.json` and
  `project.json`. Save it as a folder (desktop, or Chromium) or a zip, or
  **Publish** it: a multipart `POST /v1/mounts/{mount}/atlas` (field
  `bundle`); each publish is recorded in `project.json`
  (`upbdb.published`). The mount comes from `GET /v1/mounts` when the
  server answers, else it is typed. `grainatlas atlas <package> --out <dir or
  .zip>` writes the same bundle from the command line, and native and browser
  builds write identical bytes.
  In the mount picker or Register project dialog, **New mount…** lets a
  signed-in user create and select their own mount.
- **Open from server**: `index.html?server=<base>&project=<id>[&spot=<id>]`
  (`&mount=<id>&access=<token>` in UPbDB's own viewer links, enough for
  the files and the mount's results without signing in),
  or Settings → "Open a published project", reads
  `<base>/atlas/projects/<id>/` and shows the bundle read-only (tiles straight
  from the server, nothing stitched). **Edit a copy** makes it a local
  project with the same id. Opening Area or Layout starts loading the
  full-resolution images in the background, with the current area first.
  Area shows the published pyramid immediately, and area switching and Canvas
  editing stay available. The status line offers Stop and Retry. Tone and print
  sheets become editable after every area finishes; sync never publishes a
  partially loaded image bundle. Entering Edit alone does not start this load.
  Downloaded image files are checked against atlas.json's SHA-256 hashes and
  cached by content in the separate IndexedDB database `grainatlas-image-cache`
  in this browser or desktop webview profile. The cache persists between
  sessions, has a 1 GB cap, and evicts the least recently used files down to
  800 MB when full. Settings → Appearance and layout shows its size and offers
  **Clear image cache**. Clearing it leaves project and draft storage intact.
  If storage is unavailable, files still load from the network. Versioned file
  URLs also allow the browser's normal HTTP cache to reuse unchanged files.
  The tone can then be adjusted on top of the published image and print sheets
  made again from the current grains.
  Stitching and shading correction still need the raw package; opening it
  later picks the project up. A tone change is saved as
  `grainatlas_ui.bundle_tone` and published as a new atlas version; the raw
  package's own tone in the project is left as it was.
- **Results**: with a server and a linked mount, `GET /v1/mounts/{id}/results`
  fills the spot panel (preferred date ± 2σ, ratios, concentrations, flags,
  "Open acquisition") and a coloured results layer with a legend. Results are
  never saved in the project.
- **Laser** (Canvas → Laser tab): three or more fiducial spots with the stage
  coordinates read off the laser give an affine map (least squares, residuals
  in µm, a warning above 5 µm); "Export spot list" writes the generic CSV in
  the laser's units. Adding the lab's laser software: `laser-adapters.md`.

## Updates

The version chip stays in the bottom-right corner, including on the Canvas.
It shows when an update is available or ready; click it to open Settings >
Updates. Use **Check now** to check explicitly, choose Stable or Beta
(pre-releases), and turn automatic downloads on or off. A downloaded update
waits for **Restart to update**. UPb Lab updates hosted copies with its shell.

One-time owner setup (generates the signing key, writes the public key, and
sets `TAURI_SIGNING_PRIVATE_KEY` with `gh` when signed in):

```bash
make updater-setup
make check-version
```

Back up `~/.tauri/grainatlas-updater.key`. The `desktop` workflow now publishes
a draft release with installers, signed update bundles and `latest.json` on
`v*` tags; manual runs keep workflow artifacts. Publish the draft after the
workflows pass. See [In-app updates](in-app-updates.md) for setup and channels.

### Windows installer from macOS

From v0.1.5 the Windows release uses a current-user NSIS installer, with
passive in-app updates. Install `cargo-xwin`, Homebrew NSIS (`makensis`) and
the `x86_64-pc-windows-msvc` Rust target, then run:

```bash
make full
make windows-installer
```

`make windows-installer` builds the web page and icons. Inside its recipe it
reads `TAURI_SIGNING_PRIVATE_KEY_PATH` (default
`~/.tauri/grainatlas-updater.key`) into `TAURI_SIGNING_PRIVATE_KEY` without
printing the key, then runs this command from `tauri/`:

```bash
LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8 TAURI_SIGNING_PRIVATE_KEY=... cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis --config src-tauri/tauri.release.conf.json
```

`...` represents the key contents supplied through the environment, not a
literal value to type into shell history. Homebrew `makensis` aborts with
`std::bad_alloc` under the C locale; keep both UTF-8 locale variables.
The build writes `grainatlas_<version>_x64-setup.exe` and its `.exe.sig` in
`target/x86_64-pc-windows-msvc/release/bundle/nsis/`. Attach both to the
release and add `platforms["windows-x86_64"]` to `latest.json`: `url` is the
release download URL of the setup exe, and `signature` is the contents of
its `.exe.sig`. Keep the existing macOS entries. See the
[Windows release steps](in-app-updates.md#windows-release-from-a-mac) for the
JSON entry.

On Windows, in-app downloads and installation require `uninstall.exe`
(NSIS) or `Uninstall grainatlas.lnk` (MSI) beside the running executable.
Portable copies link to the GitHub release page; run the setup file once
and use the installed app for later in-app updates.

## Release asset for UPb Lab

```bash
make release-web
```

builds `ui/dist` and packs it as
`target/release-web/grainatlas-web-<version>.tar.gz` (`index.html` at the top,
`<version>` from `ui/package.json`), printing its SHA-256 and writing it to
`grainatlas-web-<version>.tar.gz.sha256`. Pushing a tag `v<version>` runs
`.github/workflows/release.yml`, which builds the same archive on
`ubuntu-latest` and attaches it and the `.sha256` to a draft release. For
`upbstudio/modules.lock.json`:

- `url`: `https://github.com/duserzym/grainatlas/releases/download/v<version>/grainatlas-web-<version>.tar.gz`
- `sha256`: of the downloaded asset, from
  `shasum -a 256 grainatlas-web-<version>.tar.gz` (macOS),
  `sha256sum grainatlas-web-<version>.tar.gz` (Linux) or
  `Get-FileHash -Algorithm SHA256 grainatlas-web-<version>.tar.gz` (Windows
  PowerShell; lower-case it). Use the hash of the published file, or the
  `.sha256` next to it: a local `make release-web` packs different bytes.

## Command line

The command line does batch work:

```bash
cargo run --release -p grainatlas-cli -- stitch <package-or-area-folder> --out <name>.grainatlas --auto --polarity both --print tabloid
```

`--print tabloid` writes print sheets for hand spot registration: every grain of
the package once (cross-area duplicates go to their owner area), at a scale chosen
from the grain sizes (median grain about 15 mm on paper; `--scale 8` fixes it),
with the `print_sheet` registration sidecar. Sheets follow the layout rule
"context first": grains are printed in continuous pieces of the mosaic with their
neighbours, and where a piece must be cut a hatched strip repeats the other side.
`--print-style grains` gives the older per-area layout of single grains cropped
alone, only when asked for by name. `grainatlas sheets <name>.grainatlas --out <dir>`
prints the sheets of a saved project again without stitching. See
[print_sheets.md](print_sheets.md).

The desktop app also takes `grainatlas <package-or-area-folder> [--stitch] [--segment]`
or `grainatlas <images…> --um-per-px <µm> | --field-um <µm> [--stitch]`, and
opens saved `<name>.grainatlas` folders and `<name>.gaproj` project files.

### Tabs and project files

One window holds several projects as tabs (desktop and web): **+** or Cmd/Ctrl+T for a new
tab, × or Cmd/Ctrl+W to close (a dot marks unsaved changes and closing asks), Cmd/Ctrl+1..9
and Ctrl+Tab to switch, drag to reorder. Each tab has its own session (package, mosaics,
grains, spots, undo); a stitch keeps running in its tab while you work in another. Opening
or dropping something while a project is open makes a new tab; several folders on the
command line open one tab each:

```bash
open -a grainatlas --args A.grainatlas B.grainatlas C.gaproj --stitch
```

**Save project** (Cmd/Ctrl+S) writes `<name>.gaproj` next to the package, or back to the
`.gaproj` file or `.grainatlas` folder the tab came from; **Save as…** writes a `.gaproj`
where you choose. A `.gaproj` is the project record alone (parameters, solved placements,
photometric results, grains, spots, labels, the UPbDB link; about 1-2 MB, no pixels).
Double-clicking one opens it as a tab: the package's own images are read from the recorded
path and every stitched area is rendered from its saved placement, without stitching again
(`--stitch` stitches only areas that have none). In the web build Save downloads the
`.gaproj`, and the Open screen's Project file loads one onto an opened package.

## Layout

```text
crates/grainatlas-core   all pixel operations (readers, mosaic, photometric, blend, segment, scale, spots, layout, project, atlas, laser)
crates/grainatlas-wasm   the core for the browser (wasm-bindgen)
crates/grainatlas-cli    `grainatlas` command line
ui/                    Svelte 5 + TypeScript + Vite; src/lib/core/{wasm,tauri}.ts implement the Core interface
tauri/                 Tauri 2 desktop shell (src-tauri)
docs/                  ADRs, stitch notes, legacy workflow, UX conventions, laser adapters
site/                  project website (static, no build step)
```

The repository holds no sample data or test fixtures. The screenshots in
`images/` are the only images from real mounts, with all names and metadata
removed; see [Screenshot policy](screenshots.md). `app-blank.png`
shows the app before anything opens.

The project website lives in `site/` (`make site` assembles it in
`target/site`); it is published to GitHub Pages only by running the Pages
workflow by hand.

License: MIT. Image algorithms reused from
[storytold/photocraft](https://github.com/storytold/photocraft) (MIT OR
Apache-2.0) and the lossless WebP encoder of
[image-rs/image-webp](https://github.com/image-rs/image-webp) (MIT OR
Apache-2.0); see [ATTRIBUTION.md](../ATTRIBUTION.md).

## Opened from UPb Lab

UPb Lab launches the installed standalone app with a one-time sign-in code:

```sh
open -n -b org.grainatlas.desktop --args --project <uuid> --server <base-url> --code <one-time-code> [--revision <n>]
```

A normal second launch forwards its arguments to the running instance, focuses
its window and exits. Launches with `--profile <dir>` (and development or named
profiles) skip single-instance forwarding and keep their separate storage.
`grainatlas://` links remain owned by UPb Lab.

GrainAtlas compares normalized server addresses. A different server asks
“Open project from <host>? This is not your configured server.” Nothing is
fetched from that server before confirmation. It then immediately exchanges
`{code}` at `POST /v1/auth/exchange`, reads `{token, account}`, and keeps the
token in memory for this session. The code is never saved or logged; Rust
Debug output redacts it. Do not pass a bearer token on the command line.

The project opens in View with its current mount revision. `--revision` is
the shell's observed revision; the app checks the database's current state
rather than reconstructing historical images. A locally held draft with the
same uuid opens in Edit and the chip says “Your draft here, based on r<n>”.
Hosted and standalone storage are separate; drafts are not transferred by
the launch arguments.

## Print performance checks

`make windows` cross-builds the Windows x86-64 executable with cargo-xwin and
`--no-bundle`. Install `cargo-tauri`, `cargo-xwin` and the
`x86_64-pc-windows-msvc` Rust target first. The supervisor builds the installer.
Native and wasm builds use release optimisation level 3, thin LTO, four codegen
units and no debug assertions. Wasm stays on the baseline instruction set:
SIMD is not required by this change, preserving compatibility with older hosts.

Generate only invented benchmark data beneath the ignored target directory:

```bash
cargo run --release -p grainatlas-core --example large_bundle
RAYON_NUM_THREADS=4 cargo run --release -p grainatlas-core --example print_bench -- target/large-bundle
# Read a local source package without writing it; no names or paths appear in output:
RAYON_NUM_THREADS=4 cargo run --release -p grainatlas-core --example print_bench -- "$PACKAGE_PATH" --package
make full
cd ui && pnpm build
pnpm exec playwright test e2e/print-sheets.spec.ts e2e/strip-editor.spec.ts
PERF=1 PERF_LABEL=after pnpm exec playwright test e2e/perf-print.spec.ts
```

The native benchmark separates planning, crop/enhancement, PNG, base64 and
serialization, preview, background quality, cached repacking and PDF bytes. Band and PNG times are summed worker time, so they exceed elapsed
render time. Package opening reports read, stitch, correction, blend, tone and
pyramid steps; bundle opening reports tile decode and mosaic/pyramid construction.
Browser first paint and WebView IPC require the supervisor's browser/app checks.
The performance spec uses four-times CPU throttling and a valid explicit
5 µm/mm scale (487 pages on the default bundle; the default auto-scale’s
672-page layout has an oversized band and correctly blocks export). It writes
a `before-pdf` JSON line and report in `target/perf` before attempting export,
then a `complete` line/report after PDF bytes are ready. It measures initial canvas imagery, first decoded preview, all
preview SVGs available, full-resolution readiness, PDF bytes after clicking
Export PDF, and cached return to Layout. Only the selected preview page is
mounted; the availability metric excludes Playwright pagination round trips.

Cache lifetime, implementation and measured results are in
[print-performance.md](print-performance.md).
