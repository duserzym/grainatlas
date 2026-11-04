# grainatlas

Open-source stitching, correction, segmentation, spot-placement and print-layout
tool for SEM grain-mount images (CL, BSE, SE) used to plan LA-ICP-MS sessions.
One Rust core, delivered as a static web page (wasm in a Web Worker) and as a
desktop app (Tauri 2, native multithreaded core) from the same Svelte
interface. Companion to [laicpmsdb](../laicpmsdb) and [UPbRPy](../UPbRPy).

Status: roadmap phases 1–7 of `ARCHITECTURE_PLAN.md` are implemented (readers
and stitching, photometric correction, web UI, segmentation and scale, spots,
print layout, desktop). Phase 8 (laicpmsdb link, second instrument) is next.

## What it does

1. Opens a Hitachi TM4000 Zigzag package (`.zag`, UTF-16LE `.ini` and per-tile
   `.txt`, unknown keys kept), one TM4000 area folder on its own (scale and
   stage positions from its sidecars), any folder of images with
   `positions.csv` (`file,x_um,y_um,pixel_size_nm`), or loose images with a
   scale typed once (each image an area, or one grid); see ADR-0006.
2. Stitches by stage prior, phase correlation with a Lucas-Kanade refinement
   and a global least-squares solve; corrects per-tile gain and offset, beam
   exposure steps and background; blends with seams and multiband
   (photocraft-algo). The 4 x 8 example area (32 tiles, 28 Mpx) takes 0.6 s
   natively and 8 s in the browser; see `docs/stitch_notes.md`. Tone is one
   click (Auto: leveling, stretch, CLAHE from the histogram, with Before /
   After and Reset) or manual; every control re-renders the mosaic live, and
   after segmentation each grain's tone can be matched to the area or to one
   grain.
3. Finds and numbers grains (reading order, stable across re-runs), with
   GrabCut refinement and manual add, split, merge and renumber; scale bars
   per grain; grain table CSV.
4. Places laser spots, lines and rasters on an infinite canvas with the mouse
   or trackpad alone (click to place, drag to move, drag the ring to resize,
   double-click to rename), named from a pattern such as
   `{sample}-{grain}-{n}`, with UUIDv7 ids; exports CSV and GeoJSON.
5. Packs grains or areas onto Letter, Tabloid, A4 or A3 pages with labels,
   scale bars, spots and cut marks; exports PDF and SVG.
6. Registers reference images (photos of annotated printouts, post-laser
   frames) to the mosaic by point pairs, to recover spots of legacy sessions
   (`docs/legacy_workflow.md`).

## Quick start

```bash
make setup        # Rust stable + wasm target, wasm-bindgen-cli, UI dependencies, Playwright browser
make test         # core tests, wasm/native parity of the fixture mosaic, Playwright tests
make ui-build     # static page in ui/dist (serve it from any static server)
make ui-dev       # dev server on http://localhost:5174
make desktop-dev  # desktop app (Tauri 2) on this machine
```

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
- "Send spots to laicpmsdb" on the Spots tab downloads the JSON body of
  laicpmsdb's `POST /v1/mounts/{mount_id}/spots`.
- Storage keys all start with `grainatlas.` (the desktop shell shares one
  origin between its pages).

See `docs/decisions/ADR-0005-hosted-module.md`.

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
cargo run --release -p grainatlas-cli -- stitch <package-or-area-folder> --out <name>.grainatlas --segment --print tabloid
```

The desktop app also takes `grainatlas <package-folder> [--stitch] [--segment]`
and opens saved `<name>.grainatlas` folders.

## Layout

```text
crates/grainatlas-core   all pixel operations (readers, mosaic, photometric, blend, segment, scale, spots, layout, project)
crates/grainatlas-wasm   the core for the browser (wasm-bindgen)
crates/grainatlas-cli    `grainatlas` command line
ui/                    Svelte 5 + TypeScript + Vite; src/lib/core/{wasm,tauri}.ts implement the Core interface
tauri/                 Tauri 2 desktop shell (src-tauri)
tests/fixtures/        one TM4000 area (6 tiles) for tests; tests/golden/ the fixture mosaic hash
docs/                  ADRs, stitch notes, legacy workflow
```

Code only lives here. Image data stays on the lab NAS or your laptop; the only
images in this repository are the small test fixture under `tests/fixtures/`.

License: MIT. Image algorithms reused from
[storytold/photocraft](https://github.com/storytold/photocraft) (MIT OR
Apache-2.0); see `ATTRIBUTION.md`.
