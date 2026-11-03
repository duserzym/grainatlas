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

1. Reads a Hitachi TM4000 Zigzag package (`.zag`, UTF-16LE `.ini` and per-tile
   `.txt`, unknown keys kept) or any folder of images with `positions.csv`
   (`file,x_um,y_um,pixel_size_nm`).
2. Stitches by stage prior, phase correlation with a Lucas-Kanade refinement
   and a global least-squares solve; corrects per-tile gain and offset, beam
   exposure steps and background; blends with seams and multiband
   (photocraft-algo). The 4 x 8 example area (32 tiles, 28 Mpx) takes 0.6 s
   natively and 8 s in the browser; see `docs/stitch_notes.md`.
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
make test         # core tests, wasm/native parity of the fixture mosaic, Playwright smoke test
make ui-build     # static page in ui/dist (serve it from any static server)
make ui-dev       # dev server on http://localhost:5174
make desktop-dev  # desktop app (Tauri 2) on this machine
```

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
