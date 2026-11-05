# grainatlas

Stitch, tone, segment, spot and print SEM grain-mount images (CL, BSE, SE) for
LA-ICP-MS sessions. One Rust core runs as a static web page (wasm in a Web
Worker) and as a desktop app (Tauri 2, native and multithreaded) behind the
same Svelte interface. Nothing leaves your computer: packages are read
in place. Companion to [laicpmsdb](../laicpmsdb) and [UPbRPy](../UPbRPy).

![grainatlas: CL zircon grains outlined on the canvas, with spots named sample-n](docs/images/07-dark.webp)

## Workflow

The screenshots are of the bundled test fixture: one 3 × 2 tile area of a
Hitachi TM4000 CL package at 150×. `make screenshots` takes them again.

### 1. Open a package

Drop a folder on the window, or choose one. grainatlas reads a TM4000 Zigzag
package (`.zag`, one subfolder per area), a single area folder, any folder of
images with a `positions.csv`, or loose images with a scale you type once.
Several packages or saved projects open as tabs.

![Open screen](docs/images/01-open.webp)

### 2. Stitch and tone

**Stitch all areas** places the tiles from the stage positions, refines every
overlap and solves the whole area at once (the residuals are shown on the
mosaic). Gain, offset, beam-exposure steps and background are corrected in
one solve. **Auto** sets the tone from the histogram; every control re-renders
the mosaic live, and **After ⇄** compares with the raw tiles.

![Area screen after stitching and Auto tone](docs/images/02-stitch-tone.webp)

### 3. Find the grains

On the **Canvas**, **Grains → Find grains** outlines every grain. For CL,
choose "brighter or darker" so CL-dark zircons are found too. Epoxy bubbles,
tile-seam slivers, exposure blocks and specks are flagged rather than deleted:
they show dimmed with their reason, and one click keeps one as a grain.
Grains are not numbered; spots are.

![Grains found on the canvas](docs/images/03-grains.webp)

### 4. Make the final call by hand

You decide what a grain is.

| Tool | Key | What it does |
|------|-----|--------------|
| **Delete** | D | Click a grain to remove it. The grain card also has **Not a grain** and **Delete grain**. |
| **Outline** | O | Draw a lasso, freehand or click by click; it snaps to grain edges (hold ⌘ for no snap). Close it with a double-click, Enter or a click on the first point. A new outline replaces the grain under it; ⇧ adds to that grain, ⌥ cuts from it. |
| **Brush** | B | Paint a grain larger at a size in µm; ⌥ erases. |
| **+Grain**, **Split** | G, K | Add a grain at a click; split touching grains. |

**Find again** keeps the grains you drew, and ⌘Z undoes any edit.

![Outlining a grain click by click with edge snapping](docs/images/04-outline.webp)

### 5. Place spots

With the **Spot** tool (S), click a grain to place a laser spot at the preset
diameter. Spots are named `{sample}-{n}`, and `n` counts per sample. If others
have taken numbers in between, type **Next n** to continue from any number,
or change one spot's `n` in its card. Drag a spot to move it, drag its ring to
resize it, and export the spots as CSV or GeoJSON.

![Spots named HJ-GC1-1 … HJ-GC1-6, with a spot card open](docs/images/05-spots.webp)

### 6. Print sheets for the laser session

**Layout → Make print sheets** puts every grain of the mount on paper once, at
one scale (tabloid, 1 mm = 8 µm by default):

- Each sample is cut into bands straight out of the mosaic.
- Each sheet carries the instrument header, a mount map and the laser spot
  sizes at print scale.
- **Export** writes the PDF and its `print_registration.json`, which maps
  every band back to the mosaic.

**Save project** (⌘S) writes the project back into the `.grainatlas` folder
it came from.

![A tabloid print sheet](docs/images/06-sheets.webp)

After the session, laicpmsdb's results show on the spots. A fiducial fit maps
the mosaic to the laser stage and writes the spot list (see "laicpmsdb and the
laser" below).

## What it does, in detail

Status: roadmap phases 1–11 of `ARCHITECTURE_PLAN.md` are implemented (readers
and stitching, photometric correction, web UI, segmentation and scale, spots,
print layout, desktop, the laicpmsdb link, atlas bundles, results overlay and
laser registration), except a reader for a second instrument (phase 8, waiting
for example files) and an adapter for the lab's laser software (phase 11,
waiting for its name and an example file).

1. Opens a Hitachi TM4000 Zigzag package (`.zag`, UTF-16LE `.ini` and per-tile
   `.txt`, unknown keys kept), one TM4000 area folder on its own (scale and
   stage positions from its sidecars), any folder of images with
   `positions.csv` (`file,x_um,y_um,pixel_size_nm`), or loose images with a
   scale typed once (each image an area, or one grid); see ADR-0006.
2. Stitches by stage prior, phase correlation with a Lucas-Kanade refinement
   and a global least-squares solve; corrects per-tile gain and offset, beam
   exposure steps and background in one joint solve over all tiles (overlaps agree,
   background flat; `docs/stitch_notes.md`); blends with seams and multiband
   (photocraft-algo). The 4 x 8 example area (32 tiles, 28 Mpx) takes 0.6 s
   natively and 8 s in the browser; see `docs/stitch_notes.md`. Tone is one
   click (Auto: leveling, stretch, CLAHE from the histogram, with Before /
   After and Reset) or manual; every control re-renders the mosaic live, and
   after segmentation each grain's tone can be matched to the area or to one
   grain.
3. Finds grains, bright and CL-dark (artifacts flagged with a reason), with
   GrabCut refinement; hand editing with delete, add, split, merge, an
   edge-snapping lasso (photocraft's live wire) and a brush, kept across
   re-runs; scale bars per grain; grain table CSV.
4. Places laser spots, lines and rasters on an infinite canvas with the mouse
   or trackpad alone (click to place, drag to move, drag the ring to resize,
   double-click to rename), named from a pattern, `{sample}-{n}` by default (n counted
   per sample, settable), with UUIDv7 ids; exports CSV and GeoJSON.
5. Packs grains or areas onto Letter, Tabloid, A4 or A3 pages with labels,
   scale bars, spots and cut marks; exports PDF and SVG.
6. Registers reference images (photos of annotated printouts, post-laser
   frames) to the mosaic by point pairs, to recover spots of legacy sessions
   (`docs/legacy_workflow.md`).
7. Writes **atlas bundles** (`upbstudio/docs/WORKFLOW.md` section 3.1) and
   publishes them to laicpmsdb; opens published ones from a server; shows
   laicpmsdb's results on the spots; registers the laser stage and writes the
   spot list (see "laicpmsdb and the laser" below).

## Quick start

```bash
make setup        # Rust stable + wasm target, wasm-bindgen-cli, UI dependencies, Playwright browser
make test         # core tests, wasm/native parity of the fixture mosaic, Playwright tests
make ui-build     # static page in ui/dist (serve it from any static server)
make ui-dev       # dev server on http://localhost:5174
make desktop-dev  # desktop app (Tauri 2) on this machine
```

CI: until the first release, GitHub Actions runs only on `v*` tags and by hand,
not on push or pull requests. Run it with `gh workflow run ci` (tests),
`gh workflow run desktop` (installers) or `gh workflow run release` (web
archive, no draft release); all three need the `UPBSTUDIO_TOKEN` secret below.

## Dependencies: `@lab/bridge`

grainatlas runs on its own and as the Images tab of UPb Studio. It talks to
the shell through `@lab/bridge`, which is not on npm yet: `ui/package.json`
depends on a sibling checkout, `"@lab/bridge": "file:../../upbstudio/bridge"`
(paths from `ui/`), so clone `upbstudio` next to `grainatlas`. `make setup`
builds `../upbstudio/bridge/dist` if it is missing (`make bridge`), before
`pnpm install`. CI checks out both repositories side by side
(`duserzym/upbstudio`; a private one needs a read-only `UPBSTUDIO_TOKEN`
secret).

Without an `upbstudio` checkout, take the bridge tarball from someone who has
one (`pnpm --filter @lab/bridge pack` in `upbstudio` writes
`lab-bridge-<version>.tgz`) and point the dependency at it, locally only:

```bash
cd ui && pnpm add ./lab-bridge-0.1.0.tgz
```

## Inside UPb Studio

- Hosted (the bridge's `isHosted()`), grainatlas uses the wasm core and the
  web file paths (`<input type=file webkitdirectory>`, drag-drop), even though
  Tauri's globals are present in the shell's webview.
- `index.html?project=<id>&spot=<id>` (the shell's mapping of
  `grainatlas://project/<id>/spot/<id>`) opens that project, selects the spot
  and centers the canvas on it. The browser keeps projects without pixels, so
  the package folder is opened again; the bundled example opens by itself.
- A spot's panel has "Open in laicpmsdb" (`laicpmsdb://acquisition/<id>` when
  the spot has an acquisition id, else `laicpmsdb://search?label=<name>`);
  selecting a spot sends `grainatlas.spot.selected` to the shell.
- "Send spots to laicpmsdb" on the Spots tab posts the spots with the shell's
  `token()` when a server and a mount are set, else downloads the JSON body of
  laicpmsdb's `POST /v1/mounts/{mount_id}/spots`.
- Storage keys all start with `grainatlas.` (the desktop shell shares one
  origin between its pages).

See `docs/decisions/ADR-0005-hosted-module.md`.

## laicpmsdb and the laser

All of this is optional; without a server grainatlas works as before.

- **Settings → laicpmsdb**: the server (`http://host:8000`), a token when
  grainatlas runs on its own (inside UPb Studio the shell's sign-in is used),
  the atlas tile quality (lossless WebP, or near-lossless at 97 or 92) and the
  folder where SEM packages live on the lab storage. A `?server=` in the page
  address wins for that page load; served by laicpmsdb under `/atlas/`,
  grainatlas uses its own origin.
- **Atlas bundle** (Canvas → laicpmsdb tab): `<project_id>.atlas/` with
  `atlas.json` (format `grainatlas-atlas` v1, ids, mount, scale, areas and
  levels, the raw package's content hash and storage path, sha256 and size of
  every file), `pyramid/<area>/<z>/<x>_<y>.webp` (256 px tiles, z = 0 full
  resolution), `grains.geojson`, `spots.json`, `scalebars.json` and
  `project.json`. Save it as a folder (desktop, or Chromium) or a zip, or
  **Publish** it: a multipart `POST /v1/mounts/{mount}/atlas` (field
  `bundle`); each publish is recorded in `project.json`
  (`laicpmsdb.published`). The mount comes from `GET /v1/mounts` when the
  server answers, else it is typed. `grainatlas atlas <package> --out <dir or
  .zip>` writes the same bundle from the command line, and native and browser
  builds write identical bytes (`make parity`).
- **Open from server**: `index.html?server=<base>&project=<id>[&spot=<id>]`
  (`&mount=<id>&access=<token>` in laicpmsdb's own viewer links, enough for
  the files and the mount's results without signing in),
  or Settings → "Open a published project", reads
  `<base>/atlas/projects/<id>/` and shows the bundle read-only (tiles straight
  from the server, nothing stitched). **Edit a copy** makes it a local
  project with the same id; opening the raw package later picks it up.
- **Results**: with a server and a linked mount, `GET /v1/mounts/{id}/results`
  fills the spot panel (preferred date ± 2σ, ratios, concentrations, flags,
  "Open acquisition") and a coloured results layer with a legend. Results are
  never saved in the project.
- **Laser** (Canvas → Laser tab): three or more fiducial spots with the stage
  coordinates read off the laser give an affine map (least squares, residuals
  in µm, a warning above 5 µm); "Export spot list" writes the generic CSV in
  the laser's units. Adding the lab's laser software: `docs/laser-adapters.md`.
- **Developing without laicpmsdb**: `pnpm dev` and `pnpm preview` mount a fake
  laicpmsdb at `/fake-laicpmsdb` (`ui/scripts/fake-laicpmsdb.mjs`, token
  `test-token`) with atlas upload and serving, mounts, spot upload and
  `ui/tests/fixtures/mount-results.json` as the results; `make atlas-fixture`
  writes the fixture bundle it serves. Never part of a build.

## Release asset for UPb Studio

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

The command line does batch work:

```bash
cargo run --release -p grainatlas-cli -- stitch <package-or-area-folder> --out <name>.grainatlas --auto --polarity both --print tabloid
```

`--print tabloid` writes print sheets for hand spot registration: every grain of
the package once (cross-area duplicates go to their owner area), at 1 mm = 8 µm,
with the `print_sheet` registration sidecar; `--print-style grains` gives the
older per-area grain layout. See [docs/print_sheets.md](docs/print_sheets.md).

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
photometric results, grains, spots, labels, the laicpmsdb link; about 1-2 MB, no pixels).
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
tests/fixtures/        one TM4000 area (6 tiles) for tests; tests/golden/ the fixture mosaic hash
docs/                  ADRs, stitch notes, legacy workflow, UX conventions, laser adapters; images/ README screenshots
site/                  project website (static, no build step)
```

Code only lives here. Image data stays on the lab NAS or your laptop; the only
images in this repository are the small test fixture under `tests/fixtures/`
and the README screenshots in `docs/images/` (WebP, about 600 KB in all,
taken of that fixture by `make screenshots`).

The project website lives in `site/` (`make site` assembles it in
`target/site`); it is published to GitHub Pages only by running the Pages
workflow by hand.

License: MIT. Image algorithms reused from
[storytold/photocraft](https://github.com/storytold/photocraft) (MIT OR
Apache-2.0) and the lossless WebP encoder of
[image-rs/image-webp](https://github.com/image-rs/image-webp) (MIT OR
Apache-2.0); see `ATTRIBUTION.md`.
